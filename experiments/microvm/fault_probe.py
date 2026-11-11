#!/usr/bin/env python3
"""Opt-in transport/keeper/helper loss and recovery against a live sibling VM."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import time

from cold_prepare import require_no_active_collector
from smoke import ROOT, artifact, connect, helper_identity, line
from ssh_smoke import ENV, SOCKETS, require
from workload_smoke import WorkloadInstance


def process_start(vm, pid, name):
    result = vm.ssh(name, f'cat /proc/{pid}/stat')
    if result.returncode:
        # A failed SSH transport is not evidence that the session was reaped.
        require(result.returncode == 1 and b'No such file' in result.stderr,
                (result.returncode, result.stderr))
        return None
    # comm may contain spaces and parentheses; fields after its final ')' start
    # with state (3), making starttime (22) index 19 in the remainder.
    return int(result.stdout.rsplit(b') ', 1)[1].split()[19])


def lost_ssh_session(vm):
    error = (vm.out / 'abrupt-client.stderr').open('wb')
    client = subprocess.Popen(['ssh', *vm.options(), 'devcroft@guest',
        "printf 'SESSION_PID=%s\\n' \"$$\"; exec sleep 60"], stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=error, env=ENV, close_fds=True, start_new_session=True)
    session_pid = None
    session_start = None
    reaped = False
    try:
        deadline = time.monotonic() + 8
        header = bytearray()
        while not header.endswith(b'\n'):
            remaining = deadline - time.monotonic()
            require(remaining > 0 and select.select([client.stdout], [], [], remaining)[0],
                    'SSH session never started')
            chunk = os.read(client.stdout.fileno(), 128 - len(header))
            require(chunk, 'SSH session closed before its PID marker')
            header.extend(chunk)
            require(len(header) < 128, 'SSH PID marker exceeded its bound')
        require(header.startswith(b'SESSION_PID=') and header.endswith(b'\n'), header)
        session_pid = int(header.split(b'=')[1])
        require(session_pid > 1, 'invalid session PID')
        session_start = process_start(vm, session_pid, 'session-live-before-loss')
        require(session_start is not None, 'session positive control missing')
        os.killpg(client.pid, signal.SIGKILL)
        client.wait(timeout=5)
        deadline = time.monotonic() + 7
        while True:
            current = process_start(vm, session_pid, 'session-after-loss')
            if current != session_start:
                reaped = True
                return {'host_client_signal': 'SIGKILL', 'guest_session_pid': session_pid,
                        'guest_session_start_ticks': session_start, 'guest_session_reaped': True}
            if time.monotonic() >= deadline:
                raise RuntimeError('SSH command survived abrupt client transport loss')
            time.sleep(0.05)
    finally:
        if client.poll() is None:
            os.killpg(client.pid, signal.SIGKILL)
            client.wait(timeout=5)
        error.close()
        if session_start is not None and not reaped:
            # Kill only the process started by this disposable session probe.
            if process_start(vm, session_pid, 'session-cleanup-identity') == session_start:
                vm.ssh('session-probe-cleanup', f'kill -KILL {session_pid} 2>/dev/null || true')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--adapter', choices=['libkrun', 'hvi'], required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--guest', type=Path, default=ROOT / 'artifacts/stage-b-nix/guest.json')
    args = parser.parse_args()
    require_no_active_collector(ROOT / 'runs')
    lease = (ROOT / 'artifacts/nested-runner.lock').open('a')
    fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
    os.umask(0o077)
    args.out.mkdir(parents=True, exist_ok=False)
    helper = ROOT / ('hvi/target/debug/devcroft-hvi-spike' if args.adapter == 'hvi' else 'libkrun/target/debug/devcroft-krun-spike')
    guest = json.loads(args.guest.read_text())
    image, seed, kernel = [ROOT / name for name in ('artifacts/nix/toolchain.img', 'artifacts/nix/data.seed.img', 'artifacts/kernel-landlock/Image')]
    report = {'schema': 1, 'status': 'failed', 'qualification': 'native functional fault injection; no host power-loss or host-suspend claim',
              'platform': 'macos-arm64-native', 'host': list(os.uname()), **helper_identity(helper),
              'helper': artifact(helper), 'guest': guest, 'kernel': artifact(kernel), 'image': artifact(image),
              'seed': artifact(seed), 'raw_evidence': str(args.out), 'cases': [], 'instances': []}
    report['guest_receipt'] = artifact(args.guest)
    report['planned_cases'] = ['abrupt-ssh-client-loss', 'helper-process-pause-resume',
                               'keeper-death-and-cold-recovery', 'helper-term-and-cold-recovery']
    report['not_run'] = [{'case': 'init-death', 'reason': 'baseline PID 1 has no fixed test-only exit injection'},
                         {'case': 'host-suspend-resume', 'reason': 'requires a physical-host maintenance window; helper SIGSTOP does not qualify it'}]
    snapshots = args.out / 'sources'
    snapshots.mkdir()
    for name in ('fault_probe.py', 'smoke.py', 'ssh_smoke.py', 'workload_smoke.py'):
        (snapshots / name).write_bytes((ROOT / name).read_bytes())
    instances = []

    def save():
        (args.out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')

    def case(name, callback):
        row = {'name': name, 'status': 'failed'}
        report['cases'].append(row)
        try:
            row['evidence'] = callback()
            row['status'] = 'passed'
        except Exception as error:
            row['error'] = f'{type(error).__name__}: {error}'
        save()
        print(json.dumps(row), flush=True)

    try:
        for label in ('a', 'sibling'):
            vm = WorkloadInstance(label, args.out, helper, kernel, guest, image, seed)
            instances.append(vm)
            report['instances'].append({'label': label, 'private_dir': str(vm.path), 'boots': vm.boots})
            vm.prepare(validation_timeout=90)
            vm.start()
            vm.ready(timeout=120)
        a, sibling = instances
        builds = {vm.label: vm.build() for vm in instances}
        ids = {vm.label: vm.start_redis() for vm in instances}
        for vm in instances:
            vm.write_acknowledged()
            vm.verify_values()

        def sibling_unchanged():
            sibling.verify_values('sibling-fault-check')
            value = sibling.execute('sibling-run-id', "redis-cli --raw INFO server | grep '^run_id:'").stdout.decode().strip().split(':')[1]
            require(value == ids[sibling.label], 'sibling Redis restarted')
            return {'sibling_run_id_unchanged': True, 'sibling_acknowledged_keys': 32}

        def transport():
            result = lost_ssh_session(a)
            return {**result, **sibling_unchanged()}
        case('abrupt-ssh-client-loss', transport)

        def pause_resume():
            process = a.process
            os.killpg(process.pid, signal.SIGSTOP)
            try:
                witness = sibling_unchanged()
                time.sleep(0.2)
            finally:
                os.killpg(process.pid, signal.SIGCONT)
            a.verify_values('after-helper-resume')
            return {'helper_only_pause_resume': True, 'host_suspended': False, **witness}
        case('helper-process-pause-resume', pause_resume)

        def keeper_death():
            value = a.execute('keeper-pid', '/bin/busybox pidof guest-keeper').stdout.strip()
            require(value.isdigit() and int(value) > 1, value)
            killed = a.ssh('keeper-kill', f'kill -KILL {int(value)}')
            # The SSH server and its relay are inside the killed keeper.
            require(killed.returncode != 0, 'keeper death did not close SSH')
            sock, greeting = connect(a.path / 'control.sock', a.process, time.monotonic() + 5, a.overflow)
            with sock:
                sock.sendall(b'PING keeper-dead\n')
                require(line(sock) == b'PONG keeper-dead\n', 'PID 1 control died with keeper')
            refused = a.ssh('keeper-dead-session', 'printf unexpected')
            require(refused.returncode != 0, 'dead keeper accepted a new SSH session')
            witness = sibling_unchanged()
            a.shutdown()
            a.start()
            a.ready(timeout=120)
            a.verify_build(builds[a.label])
            a.start_redis()
            a.verify_values('after-keeper-recovery')
            return {'keeper_signal': 'SIGKILL', 'pid1_control_survived': True,
                    'substrate_greeting_after_keeper_death': greeting.decode().strip(),
                    'new_ssh_refused_until_restart': True, 'cold_restart_restored_keeper': True,
                    'existing_build_and_32_acknowledged_keys_recovered': True, **witness}
        case('keeper-death-and-cold-recovery', keeper_death)

        def helper_death():
            a.boots[-1]['forced_stop'] = {'expected': True, 'signal': 'SIGTERM'}
            os.killpg(a.process.pid, signal.SIGTERM)
            code = a.process.wait(timeout=10)
            a.stop()
            witness = sibling_unchanged()
            a.start()
            a.ready(timeout=120)
            a.verify_build(builds[a.label])
            a.start_redis()
            a.verify_values('after-helper-term-recovery')
            return {'signal': 'SIGTERM', 'helper_exit_code': code, 'cold_restart_restored_service': True,
                    'existing_build_and_32_acknowledged_keys_recovered': True, **witness}
        case('helper-term-and-cold-recovery', helper_death)
        for vm in instances:
            vm.clean_shutdown()
        report['status'] = 'passed' if all(row['status'] == 'passed' for row in report['cases']) else 'failed'
    except BaseException as error:
        report['error'] = f'{type(error).__name__}: {error}'
    finally:
        for vm in instances:
            vm.stop()
        report['cleanup'] = {'all_helpers_reaped': all(vm.process is None for vm in instances),
                             'endpoints_removed': all(not (vm.path / name).exists() for vm in instances for name in SOCKETS)}
        if not all(report['cleanup'].values()):
            report.update(status='failed', cleanup_error='owned resources survived cleanup')
        completed = {row['name'] for row in report['cases']}
        report['not_run'].extend({'case': name, 'reason': 'earlier failure prevented execution'}
                                 for name in report['planned_cases'] if name not in completed)
        save()
    print(json.dumps({'status': report['status'], 'error': report.get('error')}))
    raise SystemExit(report['status'] != 'passed')


if __name__ == '__main__':
    main()
