#!/usr/bin/env python3
"""Real Codex/Claude Code initialization and dummy-key HTTPS API transport."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import signal
import socket
import ssl
import subprocess
import threading
import time

from smoke import ROOT, artifact, drain, helper_identity, LOG_LIMIT
from ssh_smoke import ENV, SOCKETS, command, require
from workload_smoke import WorkloadInstance


DUMMY_KEY = 'devcroft-microvm-deliberately-invalid-api-key'
VENDORS = {'codex': ('api.openai.com', '/v1/responses'), 'claude': ('api.anthropic.com', '/v1/messages')}
CA_FILE = '/usr/etc/ssl/certs/ca-bundle.crt'


def invocation(agent, extra_ca=None):
    environment = {'PATH': '/usr/bin:/bin', 'HOME': '/home/devcroft', 'SHELL': '/bin/sh',
                   'LANG': 'C.UTF-8', 'SSL_CERT_FILE': '/usr/etc/ssl/certs/ca-bundle.crt'}
    if extra_ca:
        environment['NODE_EXTRA_CA_CERTS'] = extra_ca
    if agent == 'codex':
        environment['CODEX_API_KEY'] = DUMMY_KEY
        provider = '{ name = "microvm_probe", base_url = "https://api.openai.com:443/v1", wire_api = "responses", env_key = "CODEX_API_KEY", request_max_retries = 0, stream_max_retries = 0, supports_websockets = false }'
        argv = ['codex', 'exec', '--skip-git-repo-check', '--json', '--sandbox', 'read-only',
                '-c', 'model_provider="microvm_probe"', '-c', 'model_providers.microvm_probe=' + provider,
                'Reply only ok.']
    else:
        environment.update(ANTHROPIC_API_KEY=DUMMY_KEY, ANTHROPIC_BASE_URL='https://api.anthropic.com:443',
                           CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1', CLAUDE_CODE_MAX_RETRIES='0')
        argv = ['claude', '--bare', '-p', '--output-format', 'stream-json', '--verbose',
                '--no-session-persistence', '--tools', '', '--model', 'claude-sonnet-4-5', 'Reply only ok.']
    script = shlex.join(['env', '-i', *[f'{k}={v}' for k, v in environment.items()], *argv])
    return script


def timed_cli(vm, agent, out, extra_ca=None, timeout=20):
    stdout, stderr, markers = bytearray(), bytearray(), []
    overflow = threading.Event()
    started = time.monotonic()
    process = subprocess.Popen(['ssh', *vm.options(), 'devcroft@guest', invocation(agent, extra_ca)],
                               stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               env=ENV, close_fds=True, start_new_session=True)

    def read_stdout():
        while line := process.stdout.readline(LOG_LIMIT + 1):
            if len(stdout) + len(line) > LOG_LIMIT:
                overflow.set()
                return
            stdout.extend(line)
            try:
                value = json.loads(line)
            except (ValueError, UnicodeDecodeError):
                continue
            if value.get('type') in ('thread.started', 'turn.started') or (value.get('type'), value.get('subtype')) == ('system', 'init'):
                markers.append({'event': value['type'], 'subtype': value.get('subtype'),
                                'upper_bound_seconds': time.monotonic() - started})

    threads = [threading.Thread(target=read_stdout, daemon=True),
               threading.Thread(target=drain, args=(process.stderr, stderr, overflow), daemon=True)]
    for thread in threads:
        thread.start()
    try:
        code = process.wait(timeout=timeout)
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)
        for thread in threads:
            thread.join(timeout=2)
        (out / f'{agent}-session.stdout').write_bytes(stdout)
        (out / f'{agent}-session.stderr').write_bytes(stderr)
    require(not overflow.is_set(), 'agent output exceeded fixture budget')
    return started, time.monotonic() - started, code, stdout, stderr, markers


class UntrustedTlsEndpoint:
    """Negative fixture only: TLS handshake, no HTTP/request parser or relay."""
    def __init__(self, host, out):
        self.out, self.events = out, []
        config = out / 'untrusted-tls.cnf'
        config.write_text('[req]\nprompt=no\ndistinguished_name=dn\nx509_extensions=extensions\n'
                          f'[dn]\nCN={host}\n[extensions]\nsubjectAltName=DNS:{host}\n'
                          'basicConstraints=critical,CA:TRUE\nextendedKeyUsage=serverAuth\n')
        self.cert, key = out / 'untrusted-tls.pem', out / 'untrusted-tls.key'
        made = command(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
                        '-config', config, '-out', self.cert, '-keyout', key], out, 'untrusted-tls-generate', timeout=15)
        require(made.returncode == 0, made.stderr)
        self.context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self.context.load_cert_chain(self.cert, key)
        self.context.set_alpn_protocols(['http/1.1'])
        self.server_name = None
        def server_name(_socket, name, _context):
            self.server_name = name
        self.context.set_servername_callback(server_name)
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(8)
        self.listener.settimeout(0.2)
        self.port = self.listener.getsockname()[1]
        self.done = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        while not self.done.is_set() and len(self.events) < 32:
            try:
                stream, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            with stream:
                stream.settimeout(5)
                self.server_name = None
                try:
                    with self.context.wrap_socket(stream, server_side=True) as tls:
                        # Some TLS clients finish the cryptographic handshake
                        # before reporting verification failure. Count actual
                        # application bytes separately; never log headers/keys.
                        event = {'at': time.monotonic(), 'server_name': self.server_name,
                                 'handshake_accepted': True, 'application_bytes_received': 0}
                        self.events.append(event)
                        data = tls.recv(4096)
                        event['application_bytes_received'] = len(data)
                        event['client_closed_without_application_data'] = not data
                        if data:
                            body = b'{"type":"error","error":{"type":"authentication_error","message":"untrusted TLS fixture received application data"}}'
                            tls.sendall(b'HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: '
                                        + str(len(body)).encode() + b'\r\nConnection: close\r\n\r\n' + body)
                except (OSError, ssl.SSLError) as error:
                    self.events.append({'at': time.monotonic(), 'server_name': self.server_name,
                                        'handshake_accepted': False, 'error': str(error)})

    def close(self):
        self.done.set()
        self.listener.close()
        self.thread.join(timeout=6)
        (self.out / 'untrusted-tls-events.json').write_text(json.dumps(self.events, indent=2) + '\n')
        require(not self.thread.is_alive(), 'TLS negative fixture did not stop')


def probe_agent(vm, agent, out, repetitions):
    host, path = VENDORS[agent]
    control = vm.path / 'agent-ssh-control'
    error_file = (out / f'{agent}-forward.stderr').open('wb')
    forward = None
    try:
        for number, spec in enumerate(('0.0.0.0:443:127.0.0.1:1', '127.0.0.1:8083:127.0.0.1:1', '127.0.0.1:0:127.0.0.1:1')):
            rejected = command(['ssh', *vm.options(), '-T', '-N', '-o', 'ExitOnForwardFailure=yes', '-R', spec,
                                'devcroft@guest'], out, f'{agent}-denied-forward-{number}', timeout=8)
            require(rejected.returncode == 255 and b'remote port forwarding failed' in rejected.stderr, rejected.stderr)
        # The fixed mapping belongs to this trusted controller/OpenSSH client.
        # The guest receives no general CONNECT/SOCKS/HTTP proxy or host socket.
        forward = subprocess.Popen(['ssh', *vm.options(), '-T', '-N', '-M', '-S', str(control),
                                    '-o', 'ExitOnForwardFailure=yes', '-o', 'ControlPersist=no',
                                    '-R', f'127.0.0.1:443:{host}:443', 'devcroft@guest'],
                                   stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=error_file,
                                   env=ENV, close_fds=True, start_new_session=True)
        deadline = time.monotonic() + 10
        while not control.exists():
            require(forward.poll() is None, 'SSH reverse forward exited before readiness')
            require(time.monotonic() < deadline, 'SSH reverse forward did not create its control endpoint')
            time.sleep(0.02)
        ready = command(['ssh', '-F', '/dev/null', '-S', control, '-O', 'check', 'devcroft@guest'], out,
                        f'{agent}-forward-ready')
        require(ready.returncode == 0, ready.stderr)
        auth = ['-H', 'Authorization: Bearer ' + DUMMY_KEY] if agent == 'codex' else [
            '-H', 'x-api-key: ' + DUMMY_KEY, '-H', 'anthropic-version: 2023-06-01']
        curl = ['curl', '--max-time', '15', '--cacert', CA_FILE, '-v', '-sS', '-o', '/workspace/tls-response',
                '-w', '%{http_code}\\n%{ssl_verify_result}\\n', '-H', 'Content-Type: application/json',
                *auth, '--data', '{}', f'https://{host}:443{path}']
        positive = vm.execute(agent + '-tls-positive', shlex.join(curl), timeout=20)
        require(positive.stdout == b'401\n0\n', ('verified guest TLS/auth response missing', positive.stdout, positive.stderr))
        samples = []
        for repetition in range(repetitions):
            trial = out / f'{agent}-session-{repetition}'
            trial.mkdir()
            started, elapsed, exit_code, stdout, stderr, markers = timed_cli(vm, agent, trial)
            require(markers, ('real CLI readiness event missing', exit_code, stdout[-3000:], stderr[-3000:]))
            initialized = [m for m in markers if m['event'] == 'turn.started' or m['subtype'] == 'init']
            require(initialized, ('agent turn/init event missing', markers))
            require(exit_code != 0 and b'401' in stdout + stderr,
                    ('real vendor dummy-key refusal missing', exit_code, stdout[-3000:], stderr[-3000:]))
            sample = {'repetition': repetition, 'session_exit_seconds': elapsed, 'cli_ready_events': markers,
                      'initialization_upper_bound_seconds': initialized[0]['upper_bound_seconds'],
                      'startup_within_5_seconds': initialized[0]['upper_bound_seconds'] <= 5,
                      'cli_exit_code': exit_code, 'real_vendor_guest_tls_dummy_key_status': 401}
            (trial / 'sample.json').write_text(json.dumps(sample, indent=2) + '\n')
            samples.append(sample)
        cancelled = command(['ssh', '-F', '/dev/null', '-S', control, '-O', 'cancel', '-R',
                             f'127.0.0.1:443:{host}:443', 'devcroft@guest'], out, f'{agent}-forward-cancel')
        require(cancelled.returncode == 0, cancelled.stderr)
        negative = out / f'{agent}-untrusted-certificate'
        negative.mkdir()
        endpoint = UntrustedTlsEndpoint(host, negative)
        try:
            mapped = command(['ssh', '-F', '/dev/null', '-S', control, '-O', 'forward', '-R',
                              f'127.0.0.1:443:127.0.0.1:{endpoint.port}', 'devcroft@guest'], out,
                             f'{agent}-negative-forward')
            require(mapped.returncode == 0, mapped.stderr)
            bounded_negative_stop = False
            try:
                _, _, bad_code, bad_out, bad_error, _ = timed_cli(vm, agent, negative, timeout=5)
            except subprocess.TimeoutExpired:
                # Bun can reconnect after rejecting a certificate without
                # emitting the final SDK error. Keep the supervised stop and
                # independently prove certificate sensitivity below.
                bounded_negative_stop = True
                bad_code = -signal.SIGKILL
                bad_out = (negative / f'{agent}-session.stdout').read_bytes()
                bad_error = (negative / f'{agent}-session.stderr').read_bytes()
            default_trust_events = [dict(e) for e in endpoint.events]
            require(bad_code != 0 and b'401' not in bad_out + bad_error
                    and default_trust_events and all(e['server_name'] == host for e in default_trust_events)
                    and not any(e.get('application_bytes_received', 0) for e in default_trust_events),
                    ('application data reached untrusted TLS endpoint', bad_code, default_trust_events))
            certificate_alert = any('CA' in e.get('error', '') or 'CERTIFICATE' in e.get('error', '') for e in default_trust_events)
            explicit_trust_control = None
            if not certificate_alert:
                require(agent == 'claude', ('no certificate rejection witness', default_trust_events))
                sent = vm.ssh('upload-public-test-ca', 'cat > /workspace/tls-test-ca.pem', data=endpoint.cert.read_bytes())
                require(sent.returncode == 0, sent.stderr)
                trusted = negative / 'explicit-test-ca'
                trusted.mkdir()
                before = len(endpoint.events)
                _, _, trusted_code, trusted_out, trusted_error, _ = timed_cli(vm, agent, trusted,
                    extra_ca='/workspace/tls-test-ca.pem', timeout=10)
                explicit_trust_control = [dict(e) for e in endpoint.events[before:]]
                require(trusted_code != 0 and b'401' in trusted_out + trusted_error
                        and any(e.get('application_bytes_received', 0) > 0 for e in explicit_trust_control),
                        ('same TLS endpoint did not work with explicit test CA', trusted_code, explicit_trust_control,
                         trusted_out[-2000:], trusted_error[-2000:]))
            cancelled = command(['ssh', '-F', '/dev/null', '-S', control, '-O', 'cancel', '-R',
                                 f'127.0.0.1:443:127.0.0.1:{endpoint.port}', 'devcroft@guest'], out,
                                f'{agent}-negative-forward-cancel')
            require(cancelled.returncode == 0, cancelled.stderr)
        finally:
            endpoint.close()
        after = vm.ssh(agent + '-cancelled-listener', 'curl --max-time 2 -s http://127.0.0.1:443/')
        require(after.returncode == 7, ('cancelled forward remains reachable', after.returncode))
        return {'startup_samples': samples,
                'measurement_scope': 'host before new pinned-key SSH exec to actual CLI initialization event; prepared VM and SSH reverse transport; vendor response separately verified',
                'private_home_reused_within_prepared_vm': True,
                'real_vendor_guest_tls_dummy_key_status': 401, 'guest_tls_verified': True,
                'real_cli_untrusted_certificate_refused': True, 'tls_negative_events': default_trust_events, 'negative_probe_supervised_stop': bounded_negative_stop, 'explicit_test_ca_positive': explicit_trust_control, 'untrusted_certificate': artifact(endpoint.cert),
                'startup_within_5_seconds': all(s['startup_within_5_seconds'] for s in samples),
                'completed_agent_task': False, 'forward_wrong_address_port_dynamic_denied': True,
                'fixed_host_destination': host + ':443', 'cancelled_listener_refused': True}
    finally:
        if forward and forward.poll() is None:
            forward.terminate()
            forward.wait(timeout=5)
        error_file.close()
        control.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--adapter', choices=['libkrun', 'hvi'], required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--inspect', action='store_true', help='versions/help only, not agent readiness')
    parser.add_argument('--trace-claude', action='store_true', help='diagnose pinned CLI under the unchanged guest policy')
    parser.add_argument('--repetitions', type=int, default=5)
    parser.add_argument('--image', type=Path, default=ROOT / 'artifacts/agents/toolchain.img')
    parser.add_argument('--seed', type=Path, default=ROOT / 'artifacts/agents/data.seed.img')
    parser.add_argument('--guest', type=Path, default=ROOT / 'artifacts/stage-b-agents/guest.json')
    args = parser.parse_args()
    require(1 <= args.repetitions <= 10, 'agent repetitions must be between one and ten')
    os.umask(0o077)
    args.out.mkdir(parents=True, exist_ok=False)
    sources = args.out / 'sources'
    sources.mkdir()
    source_hashes = {}
    for name in ('agent_probe.py', 'smoke.py', 'ssh_smoke.py', 'workload_smoke.py'):
        content = (ROOT / name).read_bytes()
        (sources / name).write_bytes(content)
        source_hashes[name] = hashlib.sha256(content).hexdigest()
    helper = ROOT / ('hvi/target/debug/devcroft-hvi-spike' if args.adapter == 'hvi' else 'libkrun/target/debug/devcroft-krun-spike')
    guest = json.loads(args.guest.read_text())
    receipt = json.loads((args.image.parent / 'build.json').read_text())
    require(guest.get('agents') and guest.get('nix'), 'agent probe requires its declared guest fixture')
    report = {'schema': 1, 'status': 'failed', 'qualification': 'real CLI initialization and dummy-key transport; not completed agent task',
              'host': list(os.uname()), **helper_identity(helper), 'helper': artifact(helper),
              'guest': guest, 'closure_build': receipt, 'agent_selection': ['codex', 'claude'],
              'resources_per_vm': {'vcpus': 2, 'memory_mib': 1024}, 'raw_evidence': str(args.out), 'agents': []}
    report['harness_sources'] = source_hashes
    protocol = ROOT / 'fixtures/agent-start-protocol.json'
    report['measurement_protocol'] = json.loads(protocol.read_text())
    report['protocol_artifact'] = artifact(protocol)
    threshold_record = ROOT / 'fixtures/no-go-thresholds.json'
    require(artifact(threshold_record)['sha256'] == report['measurement_protocol']['thresholds']['sha256'],
            'accepted thresholds changed after agent protocol freeze')
    require(json.loads(threshold_record.read_text())['original_no_go_thresholds']['agent_session_start_seconds'] == 5,
            'startup comparison does not match accepted limit')
    require(receipt['versions'] == {'claudeCode': '2.1.228', 'codex': '0.147.0', 'gcc': '15.3.0', 'redis': '8.8.1'},
            'agent closure versions differ from frozen fixture')
    report['kernel'] = artifact(ROOT / 'artifacts/kernel-landlock/Image')
    report['agent_image'] = artifact(args.image)
    report['seed'] = artifact(args.seed)
    require(report['agent_image']['sha256'] == receipt['toolchain.img']['sha256']
            and report['seed']['sha256'] == receipt['data.seed.img']['sha256'],
            'agent image/seed differs from its materialization receipt')
    vm = WorkloadInstance('agent-vm', args.out, helper, ROOT / 'artifacts/kernel-landlock/Image', guest,
                          args.image, args.seed)
    vm.launch['memory_mib'] = 1024
    vm.launch_path.write_text(json.dumps(vm.launch, indent=2) + '\n')
    (vm.out / 'launch.json').write_text(vm.launch_path.read_text())
    report['instance'] = {'path': str(vm.path), 'boots': vm.boots}
    try:
        vm.prepare(validation_timeout=90)
        vm.start()
        vm.ready(timeout=120)
        if args.trace_claude:
            vm.execute('trace-directory', 'mkdir -p /workspace/tracer')
            trace_files = ROOT / 'artifacts/agents/tracer'
            report['tracer_artifacts'] = []
            for file in sorted(trace_files.iterdir()):
                require(file.is_file() and '/' not in file.name, 'unexpected tracer member')
                report['tracer_artifacts'].append(artifact(file))
                sent = vm.ssh('upload-' + file.name, 'cat > /workspace/tracer/' + shlex.quote(file.name), data=file.read_bytes())
                require(sent.returncode == 0, sent.stderr)
            vm.execute('trace-permissions', 'chmod +x /workspace/tracer/strace /workspace/tracer/ld-musl-aarch64.so.1')
            traced = vm.ssh('claude-trace', '/workspace/tracer/ld-musl-aarch64.so.1 --library-path /workspace/tracer /workspace/tracer/strace -f -s 200 -o /workspace/claude.trace /usr/bin/claude --version')
            report['trace_exit'] = traced.returncode
            exported = vm.execute('export-trace', 'cat /workspace/claude.trace')
            (args.out / 'claude.trace').write_bytes(exported.stdout)
            require(traced.returncode == 0, ('claude-trace', traced.returncode))
        for agent in ('codex', 'claude'):
            binary = 'claude' if agent == 'claude' else 'codex'
            version = vm.execute(agent + '-version', binary + ' --version')
            vm.execute(agent + '-help', binary + ' --help')
            report['agents'].append({'agent': agent, 'version_stdout': version.stdout.decode().strip()})
        if args.inspect:
            wrapper = vm.execute('claude-wrapper', shlex.join(['python3', '-c',
                'import sys; sys.stdout.buffer.write(open("/usr/bin/claude", "rb").read(8192))']))
            (args.out / 'claude-wrapper.bin').write_bytes(wrapper.stdout)
        if not args.inspect:
            for agent in report['agents']:
                agent['probe'] = probe_agent(vm, agent['agent'], args.out, args.repetitions)
        vm.shutdown()
        report['status'] = 'passed'
        report['functional_status'] = 'passed'
        if args.inspect:
            report['qualification'] = 'CLI version/help inspection only; no agent readiness claim'
        else:
            report['agent_start_criterion'] = 'passed' if all(a['probe']['startup_within_5_seconds'] for a in report['agents']) else 'failed'
            report['status'] = report['agent_start_criterion']
            if args.repetitions != report['measurement_protocol']['repetitions_per_cli']:
                report['qualification'] = 'calibration-only; repetition count differs from frozen protocol'
    except BaseException as error:
        report['error'] = f'{type(error).__name__}: {error}'
    finally:
        vm.stop()
        report['cleanup'] = {'helper_reaped': vm.process is None,
                             'endpoints_removed': all(not (vm.path / name).exists() for name in SOCKETS)}
        if not all(report['cleanup'].values()):
            report.update(status='failed', cleanup_error='owned resources survived cleanup')
        (args.out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'status': report['status'], 'agents': [dict(agent=a['agent'], ready_seconds=[s['initialization_upper_bound_seconds'] for s in a.get('probe', {}).get('startup_samples', [])]) for a in report['agents']], 'error': report.get('error')}))
    raise SystemExit(report['status'] != 'passed')


if __name__ == '__main__':
    main()
