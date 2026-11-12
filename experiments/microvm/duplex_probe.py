#!/usr/bin/env python3
"""Bounded native OpenSSH duplex/receive-window proof against a separate keeper."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import threading
import time

from cold_prepare import require_no_active_collector
from fault_probe import process_start
from smoke import ROOT, artifact, helper_identity
from ssh_smoke import ENV, SOCKETS, require
from workload_smoke import WorkloadInstance


def stream(vm, name, script, payload, extra=(), before_send=None, blocked=None):
    report = {'input_bytes': len(payload), 'input_sha256': hashlib.sha256(payload).hexdigest()}
    received, error, prefix = hashlib.sha256(), bytearray(), bytearray()
    count = 0
    overflow, header, stopping = threading.Event(), threading.Event(), threading.Event()
    stop_readers = threading.Event()
    started = time.monotonic()
    child = subprocess.Popen(['ssh', *vm.options(), *extra, 'devcroft@guest', script],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        env=ENV, close_fds=True, start_new_session=True)
    for pipe in (child.stdin, child.stdout, child.stderr):
        os.set_blocking(pipe.fileno(), False)

    def read(pipe, size):
        while True:
            try:
                return os.read(pipe.fileno(), size)
            except BlockingIOError:
                if stop_readers.wait(0.01):
                    return b''

    def read_output():
        nonlocal count
        while chunk := read(child.stdout, 65536):
            count += len(chunk)
            received.update(chunk)
            prefix.extend(chunk[:max(0, 128 - len(prefix))])
            if b'\n' in prefix:
                header.set()
            if count > len(payload) + 65536:
                overflow.set()

    def read_error():
        while chunk := read(child.stderr, 4096):
            error.extend(chunk[:max(0, 65536 - len(error))])
            if len(error) == 65536:
                overflow.set()

    def write_input():
        try:
            position = 0
            while position < len(payload) and not stopping.is_set():
                try:
                    position += os.write(child.stdin.fileno(), memoryview(payload)[position:position + 65536])
                except BlockingIOError:
                    stopping.wait(0.01)
            report['input_written_bytes'] = position
        except OSError as problem:
            report['stdin_writer_error'] = type(problem).__name__
        finally:
            try:
                child.stdin.close()
            except OSError:
                pass

    readers = [threading.Thread(target=read_output, daemon=True), threading.Thread(target=read_error, daemon=True)]
    writer = None
    for thread in readers:
        thread.start()
    try:
        if before_send:
            # read1 returns the small PID marker without waiting for a 64-KiB
            # buffer. The command must be live before overload input begins.
            require(header.wait(5), 'session PID marker not received')
            report['positive_control'] = before_send(bytes(prefix))
        writer = threading.Thread(target=write_input, daemon=True)
        writer.start()
        if blocked:
            time.sleep(0.5)
            require(writer.is_alive() and child.poll() is None, 'nonreading stdin was not flow-controlled')
            report['input_throttled'] = True
            report['independent_channel'] = blocked()
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
        report['exit_code'] = child.wait(timeout=30)
    finally:
        if child.poll() is None:
            os.killpg(child.pid, signal.SIGKILL)
            child.wait(timeout=5)
        stopping.set()
        if writer:
            writer.join(timeout=2)
        # A mux client's exit-status can precede the master's last stdio writes.
        # Drain to actual pipe EOF before measuring the byte/hash result.
        for thread in readers:
            thread.join(timeout=7)
        workers_joined = all(not thread.is_alive() for thread in [*readers, *([writer] if writer else [])])
        if not workers_joined:
            stop_readers.set()
            for thread in readers:
                thread.join(timeout=1)
        if not writer:
            child.stdin.close()
        report.update(seconds=time.monotonic() - started, output_bytes=count, output_sha256=received.hexdigest(),
                      prefix=bytes(prefix).decode(errors='replace'), stderr=bytes(error).decode(errors='replace'),
                      output_budget_exceeded=overflow.is_set(), workers_joined=workers_joined)
        (vm.out / f'{name}.json').write_text(json.dumps(report, indent=2) + '\n')
    require(not overflow.is_set() and report['workers_joined'], report)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--adapter', choices=['libkrun', 'hvi'], required=True)
    parser.add_argument('--guest', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    require_no_active_collector(ROOT / 'runs')
    lease = (ROOT / 'artifacts/nested-runner.lock').open('a')
    fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
    os.umask(0o077)
    args.out.mkdir(parents=True, exist_ok=False)
    helper = ROOT / ('hvi/target/debug/devcroft-hvi-spike' if args.adapter == 'hvi' else 'libkrun/target/debug/devcroft-krun-spike')
    kernel = ROOT / 'artifacts/kernel-landlock/Image'
    guest = json.loads(args.guest.read_text())
    image, seed = [ROOT / name for name in ('artifacts/nix/toolchain.img', 'artifacts/nix/data.seed.img')]
    report = {'schema': 1, 'status': 'failed', 'qualification': 'native functional OpenSSH byte-exact burst/stream/overload proof; not density or unpaced russh-client qualification',
              'platform': 'macos-arm64-native', **helper_identity(helper), 'helper': artifact(helper), 'kernel': artifact(kernel),
              'guest': guest, 'image': artifact(image), 'seed': artifact(seed), 'raw_evidence': str(args.out), 'cases': []}
    sources = args.out / 'sources'
    sources.mkdir()
    for name in ('duplex_probe.py', 'fault_probe.py', 'ssh_smoke.py', 'ssh_transport.py', 'smoke.py', 'workload_smoke.py'):
        (sources / name).write_bytes((ROOT / name).read_bytes())
    vm, master, master_error = None, None, None
    try:
        vm = WorkloadInstance('guest', args.out, helper, kernel, guest, image, seed)
        vm.prepare(validation_timeout=90)
        vm.start()
        vm.ready(timeout=120)
        control = vm.controller_dir / 'duplex-control'
        master_error = (vm.out / 'master.stderr').open('wb')
        master = subprocess.Popen(['ssh', *vm.options(), '-M', '-N', '-S', str(control), 'devcroft@guest'],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=master_error,
            env=ENV, close_fds=True, start_new_session=True)
        deadline = time.monotonic() + 8
        while not control.is_socket():
            require(master.poll() is None and time.monotonic() < deadline, 'SSH master did not start')
            time.sleep(0.02)
        extra = ['-S', str(control), '-o', 'ControlMaster=no']

        def responsive_channel():
            require(master.poll() is None, 'master connection exited')
            result = vm.ssh('responsive-on-master', 'printf responsive', extra=[*extra, '-vv'])
            require(result.returncode == 0 and result.stdout == b'responsive', result.stderr)
            require(b'mux_client_request_session: master session id:' in result.stderr, 'responsive command did not reuse the master connection')
            require(master.poll() is None, 'master connection exited after command')
            return {'same_connection_usable': True, 'master_pid': master.pid}

        pattern = bytes(range(251))
        for size in (131072, 16 * 1024 * 1024):
            payload = (pattern * (size // len(pattern) + 1))[:size]
            row = stream(vm, f'echo-{size}', 'cat; exit 29', payload, extra=extra)
            report['cases'].append({'name': f'binary-echo-{size}', **row})
            require(row['exit_code'] == 29 and row['output_bytes'] == size and row['output_sha256'] == row['input_sha256'], row)

        def live(marker):
            pid = int(marker.splitlines()[0].removeprefix(b'SESSION_PID='))
            require(pid > 1, marker)
            ticks = process_start(vm, pid, 'overload-positive-control')
            require(ticks is not None, 'nonreading command not live before input')
            return {'pid': pid, 'start_ticks': ticks}

        def lose_transport():
            witness = responsive_channel()
            os.killpg(master.pid, signal.SIGKILL)
            master.wait(timeout=5)
            return {**witness, 'master_transport_lost': True}

        row = stream(vm, 'blocked-input', "trap '' TERM; printf 'SESSION_PID=%s\\n' \"$$\"; exec sleep 60", b'x' * (12 * 1024 * 1024), extra=extra, before_send=live, blocked=lose_transport)
        report['cases'].append({'name': 'nonreading-command-flow-control', **row})
        require(row['exit_code'] != 0 and 'SSH stdin backlog exceeded' not in row['stderr'], row)
        positive = row['positive_control']
        deadline = time.monotonic() + 7
        while process_start(vm, positive['pid'], 'overload-after-close') == positive['start_ticks']:
            require(time.monotonic() < deadline, 'overloaded command survived channel cleanup')
            time.sleep(0.05)
        row['guest_command_reaped'] = True
        responsive = vm.ssh('responsive-after-transport-loss', 'printf responsive')
        require(responsive.returncode == 0 and responsive.stdout == b'responsive', responsive.stderr)
        row['new_connection_usable'] = True
        vm.shutdown()
        report['status'] = 'passed'
    except BaseException as error:
        report['error'] = f'{type(error).__name__}: {error}'
    finally:
        if master:
            if master.poll() is None:
                os.killpg(master.pid, signal.SIGKILL)
            master.wait(timeout=5)
        if master_error:
            master_error.close()
        if vm:
            vm.stop()
            report['boots'] = vm.boots
        report['cleanup'] = {'helper_reaped': vm is None or vm.process is None,
                             'master_reaped': master is None or master.poll() is not None,
                             'endpoints_removed': vm is None or all(not (vm.path / name).exists() for name in SOCKETS)}
        if not all(report['cleanup'].values()):
            report['status'] = 'failed'
        (args.out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({key: report.get(key) for key in ('status', 'error')}))
    raise SystemExit(report['status'] != 'passed')


if __name__ == '__main__':
    main()
