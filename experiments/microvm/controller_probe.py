#!/usr/bin/env python3
"""Prove VMM/controller authority separation with live native guests."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import socket
import time

from cold_prepare import require_no_active_collector
from smoke import ROOT, artifact, helper_identity
from ssh_smoke import SOCKETS, command, require
from workload_smoke import WorkloadInstance


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--adapter', choices=['libkrun', 'hvi'], required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--guest', type=Path, default=ROOT / 'artifacts/stage-b-nix-ssh-loss-fix-final/guest.json')
    args = parser.parse_args()
    require_no_active_collector(ROOT / 'runs')
    lease = (ROOT / 'artifacts/nested-runner.lock').open('a')
    fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
    os.umask(0o077)
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=False)
    helper = ROOT / args.adapter / 'target/debug' / ('devcroft-krun-spike' if args.adapter == 'libkrun' else 'devcroft-hvi-spike')
    guest = json.loads(args.guest.read_text())
    report = {'schema': 1, 'status': 'failed', 'qualification': 'native controller separation and confined-helper negative controls; not a VMM escape demonstration',
              'platform': 'macos-arm64-native', **helper_identity(helper), 'helper': artifact(helper),
              'guest': guest, 'raw_evidence': str(args.out), 'checks': [], 'instances': []}
    sources = args.out / 'sources'
    sources.mkdir()
    for name in ('controller_probe.py', 'ssh_smoke.py', 'smoke.py', 'workload_smoke.py',
                 'libkrun/src/config.rs', 'libkrun/src/sandbox.rs', 'libkrun/src/main.rs', 'hvi/src/main.rs'):
        target = sources / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes((ROOT / name).read_bytes())
    vms = []
    listener = None
    socket_path = None
    try:
        for label in ('a', 'sibling'):
            vm = WorkloadInstance(label, args.out, helper, ROOT / 'artifacts/kernel-landlock/Image', guest,
                                  ROOT / 'artifacts/nix/toolchain.img', ROOT / 'artifacts/nix/data.seed.img')
            vms.append(vm)
            report['instances'].append({'label': label, 'runtime_dir': str(vm.path),
                                       'controller_dir': str(vm.controller_dir), 'boots': vm.boots})
            require(vm.launch_path.parent == vm.controller_dir and vm.controller_dir.parent == vm.path.parent
                    and vm.controller_dir != vm.path, 'controller/runtime scopes overlap')
            require(not any((vm.path / name).exists() for name in ('launch.json', 'client', 'known_hosts', 'agent-ssh-control')),
                    'trusted controller metadata found inside writable runtime')
        a, sibling = vms
        socket_path = a.controller_dir / 'master-canary.sock'
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(str(socket_path))
        listener.listen(32)
        checkout = args.out / 'checkout-canary'
        checkout.write_text('owned host-checkout control\n')
        cache = args.out / 'shared-cache-canary'
        cache.write_text('owned shared-cache control\n')
        for name, target in [('launch', a.launch_path), ('client-key', a.controller_dir / 'client'),
                             ('known-hosts', a.known_hosts), ('sibling-client-key', sibling.controller_dir / 'client'),
                             ('checkout', checkout), ('shared-cache', cache)]:
            original = hashlib.sha256(target.read_bytes()).hexdigest()
            result = command([helper, 'controller-test', a.launch_path, target, socket_path], args.out,
                             'confined-' + name, timeout=90)
            require(result.returncode == 0, (name, result.stderr))
            checks = json.loads(result.stdout)
            require(checks['passed'] and checks['outside_denied'] and checks['outside_write_denied']
                    and checks['outside_unix_connect_denied'], (name, checks))
            require(hashlib.sha256(target.read_bytes()).hexdigest() == original, 'protected target changed')
            report['checks'].append({'name': name, 'status': 'passed', 'target_sha256': original, 'controls': checks})
            for endpoint in SOCKETS:
                (a.path / endpoint).unlink(missing_ok=True)
        listener.close()
        listener = None
        socket_path.unlink()
        for vm in vms:
            vm.prepare(validation_timeout=90)
            vm.start()
            vm.ready(timeout=120)
        for vm in vms:
            hashes = vm.build()
            vm.start_redis()
            vm.write_acknowledged()
            vm.verify_values()
            vm.boots[-1]['controller_separated_build'] = hashes
        a.shutdown()
        a.start()
        a.ready(timeout=120)
        a.verify_build(a.boots[0]['controller_separated_build'])
        a.start_redis()
        a.verify_values('controller-separated-restart')
        sibling.verify_values('sibling-survives-restart')
        for vm in vms:
            vm.clean_shutdown()
        report['status'] = 'passed'
        report['live_controls'] = {'two_builds_and_redis_services': True, 'cold_restart_preserves_keys_and_build': True,
                                   'sibling_survives_restart': True, 'controller_records_outside_vmm_write_grant': True}
    except BaseException as error:
        report['error'] = f'{type(error).__name__}: {error}'
    finally:
        if listener is not None:
            listener.close()
        if socket_path is not None:
            socket_path.unlink(missing_ok=True)
        for vm in vms:
            vm.stop()
        report['cleanup'] = {'all_owned_helpers_reaped': all(vm.process is None for vm in vms),
                             'controller_canary_socket_removed': socket_path is None or not socket_path.exists(),
                             'endpoints_removed': all(not (vm.path / name).exists() for vm in vms for name in SOCKETS)}
        if not all(report['cleanup'].values()):
            report['status'] = 'failed'
        (args.out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'status': report['status'], 'checks': len(report['checks']), 'error': report.get('error')}), flush=True)
    raise SystemExit(report['status'] != 'passed')


if __name__ == '__main__':
    main()
