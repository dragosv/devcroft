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
import traceback

from cold_prepare import require_no_active_collector
from smoke import ROOT, artifact, connect, helper_identity, line
from ssh_smoke import ENV, SOCKETS, require
from workload_smoke import WorkloadInstance


def validate_pid1_fixture_pair(guest, control):
    """Do not turn a normal receipt or a mismatched sibling into an exit test."""
    if guest.get('pid1_exit_probe') is not True or control.get('pid1_exit_probe', False) is not False:
        raise ValueError('PID 1 exit requires an enabled fault fixture and a disabled control fixture')
    for name in ('arch', 'source_sha256', 'keeper_sha256', 'busybox_sha256', 'nix'):
        if name not in guest or guest[name] != control.get(name):
            raise ValueError('PID 1 fault/control fixture mismatch: ' + name)
    if guest['arch'] != 'aarch64' or guest['nix'] is not True:
        raise ValueError('PID 1 probe currently qualifies only the native ARM64 Nix fixture')
    if guest['runtime']['sha256'] != control['runtime']['sha256']:
        raise ValueError('PID 1 fault/control runtime mismatch')
    if guest['initramfs']['sha256'] == control['initramfs']['sha256']:
        raise ValueError('PID 1 fault/control initramfs must differ')


def control_reply(vm, request):
    sock, _ = connect(vm.path / 'control.sock', vm.process, time.monotonic() + 5, vm.overflow)
    with sock:
        sock.sendall(request + b'\n')
        return line(sock)


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
    parser.add_argument('--pid1-exit', action='store_true', help='also exercise the fixed test-only PID 1 exit')
    parser.add_argument('--control-guest', type=Path, help='matching ordinary guest used as a live negative-control sibling')
    args = parser.parse_args()
    guest = json.loads(args.guest.read_text())
    control_guest = json.loads(args.control_guest.read_text()) if args.control_guest else guest
    if args.pid1_exit:
        if not args.control_guest:
            parser.error('--pid1-exit requires a matching --control-guest with no exit hook')
        try:
            validate_pid1_fixture_pair(guest, control_guest)
        except (ValueError, KeyError) as error:
            parser.error(str(error))
    elif args.control_guest:
        parser.error('--control-guest is used only with --pid1-exit')
    require_no_active_collector(ROOT / 'runs')
    lease = (ROOT / 'artifacts/nested-runner.lock').open('a')
    fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
    os.umask(0o077)
    args.out.mkdir(parents=True, exist_ok=False)
    helper = ROOT / ('hvi/target/debug/devcroft-hvi-spike' if args.adapter == 'hvi' else 'libkrun/target/debug/devcroft-krun-spike')
    image, seed, kernel = [ROOT / name for name in ('artifacts/nix/toolchain.img', 'artifacts/nix/data.seed.img', 'artifacts/kernel-landlock/Image')]
    report = {'schema': 1, 'status': 'failed', 'qualification': 'native functional fault injection; no host power-loss or host-suspend claim',
              'platform': 'macos-arm64-native', 'host': list(os.uname()), **helper_identity(helper),
              'helper': artifact(helper), 'guest': guest, 'kernel': artifact(kernel), 'image': artifact(image),
              'seed': artifact(seed), 'raw_evidence': str(args.out), 'cases': [], 'instances': []}
    report['guest_receipt'] = artifact(args.guest)
    report['planned_cases'] = ['abrupt-ssh-client-loss', 'helper-process-pause-resume',
                               'keeper-death-and-cold-recovery', 'helper-term-and-cold-recovery']
    report['not_run'] = [{'case': 'host-suspend-resume', 'reason': 'requires a physical-host maintenance window; helper SIGSTOP does not qualify it'}]
    if args.pid1_exit:
        report['planned_cases'].append('init-death-and-cold-recovery')
        report['control_guest'] = control_guest
        report['control_guest_receipt'] = artifact(args.control_guest)
    else:
        report['not_run'].append({'case': 'init-death', 'reason': 'requires --pid1-exit and a separate enabled fixture plus ordinary control guest'})
    snapshots = args.out / 'sources'
    snapshots.mkdir()
    for name in ('fault_probe.py', 'smoke.py', 'ssh_smoke.py', 'ssh_transport.py', 'workload_smoke.py',
                 'build_guest.py', 'build_keeper_guest.py', 'guest/init.c'):
        target = snapshots / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes((ROOT / name).read_bytes())
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
            row['traceback'] = traceback.format_exc()
        save()
        print(json.dumps(row), flush=True)

    try:
        for label in ('a', 'sibling'):
            vm_guest = control_guest if label == 'sibling' else guest
            vm = WorkloadInstance(label, args.out, helper, kernel, vm_guest, image, seed)
            instances.append(vm)
            report['instances'].append({'label': label, 'private_dir': str(vm.path), 'boots': vm.boots,
                                        'guest_initramfs_sha256': vm_guest['initramfs']['sha256'],
                                        'pid1_exit_probe': vm_guest.get('pid1_exit_probe', False)})
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

        if args.pid1_exit:
            def init_death():
                # Prove the exact destructive command is absent in the ordinary
                # guest, then keep that same live sibling as the crash witness.
                refused = control_reply(sibling, b'TEST PID1 EXIT 86')
                require(refused == b'ERR command\n', refused)
                require(control_reply(sibling, b'PING pid1-control') == b'PONG pid1-control\n',
                        'ordinary guest stopped responding after refusing the exit command')
                for command in (b'TEST PID1 EXIT', b'TEST PID1 EXIT 0', b'TEST PID1 EXIT 86 extra'):
                    require(control_reply(a, command) == b'ERR command\n', 'exit hook accepted a non-fixed command')
                require(process_start(a, 1, 'pid1-live-before-exit') is not None, 'PID 1 positive control missing')
                a.boots[-1]['forced_stop'] = {'expected': True, 'guest_pid': 1, 'guest_exit_code': 86,
                                             'injection': 'compile-time test-only fixed control command'}
                sock, _ = connect(a.path / 'control.sock', a.process, time.monotonic() + 5, a.overflow)
                started = time.monotonic()
                with sock:
                    sock.sendall(b'TEST PID1 EXIT 86\n')
                code = a.process.wait(timeout=15)
                stopped_seconds = time.monotonic() - started
                a.stop()
                console = bytes(a.stdout + a.stderr)
                require(b'guest: test-only PID1 exit=86' in console, 'PID 1 injection marker missing')
                require(b'Kernel panic - not syncing: Attempted to kill init!' in console
                        and b'exitcode=0x00005600' in console, 'expected PID 1 exit panic missing')
                require(not a.boots[-1]['forced_cleanup'] and not a.boots[-1]['log_overflow'],
                        'PID 1 panic did not terminate the helper without host cleanup')
                require(all(not (a.path / name).exists() for name in SOCKETS), 'stale crash endpoints remain')
                witness = sibling_unchanged()
                a.start()
                a.ready(timeout=120)
                a.verify_build(builds[a.label])
                a.start_redis()
                a.verify_values('after-init-death-recovery')
                return {'test_only_exit_hook': True, 'guest_pid': 1, 'guest_exit_code': 86,
                        'normal_guest_refused_exact_exit_command': True, 'nonfixed_commands_refused': 3,
                        'kernel_init_death_panic_observed': True, 'helper_exit_code': code,
                        'helper_stopped_seconds': stopped_seconds, 'host_forced_cleanup_needed': False,
                        'owned_endpoints_removed_before_restart': True, 'cold_restart_restored_service': True,
                        'existing_build_and_32_acknowledged_keys_recovered': True, **witness}
            case('init-death-and-cold-recovery', init_death)
        for vm in instances:
            vm.clean_shutdown()
        report['status'] = 'passed' if all(row['status'] == 'passed' for row in report['cases']) else 'failed'
    except BaseException as error:
        report['error'] = f'{type(error).__name__}: {error}'
        report['traceback'] = traceback.format_exc()
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
