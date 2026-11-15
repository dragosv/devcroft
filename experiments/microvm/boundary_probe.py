#!/usr/bin/env python3
"""Layer-aware native egress and immutable cache probes on disposable copies."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import traceback

import keeper_probe as wire
from cold_prepare import require_no_active_collector
from fault_probe import control_reply
from protocol_probe import Canary
from smoke import ROOT, artifact, helper_identity
from ssh_smoke import SOCKETS, command, require
from stop_receipt import owned_identity
from workload_smoke import WorkloadInstance, sparse_copy

LINUX_ERRNO = {'EACCES': 13, 'EROFS': 30, 'EXDEV': 18, 'ENETUNREACH': 101,
               'EAFNOSUPPORT': 97, 'EIO': 5}
POSITIVE = {'tcp_allowed_http_identity', 'tcp_allowed_bind', 'udp_unlisted_loopback_delivery',
            'cache_existing_file_read', 'private_file_operations'}
TCP_DENIAL = {'tcp_live_unlisted_port', 'tcp_external_unlisted_port', 'tcp_unlisted_bind'}
NO_ROUTE = {'tcp_external_allowed_port', 'udp_external_send'}
CACHE_DENIAL = {'cache_append', 'cache_truncate', 'cache_create', 'cache_unlink', 'cache_rename',
                'cache_hardlink', 'cache_write_via_workspace_symlink'}
RAW_DENIAL = {'raw_cache_device_read', 'raw_private_device_read', 'raw_bootstrap_device_read'}


def validate_pair(challenge, ordinary):
    if type(challenge.get('cache_boundary_probe_schema')) is not int or challenge['cache_boundary_probe_schema'] != 1:
        raise ValueError('explicit cache challenge guest required')
    if 'cache_boundary_probe_schema' in ordinary:
        raise ValueError('ordinary guest must not enable the privileged cache command')
    for key in ('arch', 'source_sha256', 'keeper_sha256', 'busybox_sha256'):
        if challenge.get(key) != ordinary.get(key):
            raise ValueError('fixture pair mismatch: ' + key)
    if challenge.get('arch') != 'aarch64' or not all(g.get('workload') is True for g in (challenge, ordinary)):
        raise ValueError('ordinary Alpine workload guests required')
    if any(g.get(key) for g in (challenge, ordinary)
           for key in ('nix', 'agents', 'adaptable', 'pid1_exit_probe', 'remote_ssh_fixture')):
        raise ValueError('cache challenge must not mix other special fixture types')
    if challenge['runtime']['sha256'] != ordinary['runtime']['sha256']:
        raise ValueError('fixture runtime mismatch')
    if challenge['initramfs']['sha256'] == ordinary['initramfs']['sha256']:
        raise ValueError('enabled and ordinary initramfs must differ')


def assess_child(data, instance_id):
    if type(data.get('schema')) is not int or data['schema'] != 1 or data.get('errno_constants') != LINUX_ERRNO:
        raise ValueError('unexpected Linux probe schema or errno ABI')
    if data.get('ipv4_interfaces') != ['lo'] or data.get('http_instance_id') != instance_id:
        raise ValueError('unexpected interface inventory or HTTP instance identity')
    if data.get('ipv6_socket_errno') != LINUX_ERRNO['EAFNOSUPPORT']:
        raise ValueError('IPv6 diagnostic disagrees with the pinned IPv4-only kernel')
    rows = data['checks']
    expected = POSITIVE | TCP_DENIAL | NO_ROUTE | CACHE_DENIAL | RAW_DENIAL
    if len(rows) != len(expected) or {r['name'] for r in rows} != expected:
        raise ValueError('missing or duplicate boundary check')
    results = []
    for row in rows:
        name, rc, error = row['name'], row['rc'], row['errno']
        if type(rc) is not int or type(error) is not int:
            raise ValueError('integer syscall result and errno required')
        if name in POSITIVE:
            passed, layer = rc == 0 and error == 0, 'positive-control'
        elif name in TCP_DENIAL | RAW_DENIAL:
            passed, layer = rc == -1 and error == LINUX_ERRNO['EACCES'], 'guest-landlock'
        elif name in NO_ROUTE:
            passed, layer = rc == -1 and error == LINUX_ERRNO['ENETUNREACH'], 'guest-no-route'
        else:
            allowed = {LINUX_ERRNO[k] for k in ('EACCES', 'EROFS')}
            if name in {'cache_rename', 'cache_hardlink'}:
                allowed.add(LINUX_ERRNO['EXDEV'])
            passed = rc == -1 and error in allowed
            layer = {13: 'guest-landlock', 30: 'guest-readonly-mount', 18: 'guest-reference-or-filesystem-boundary'}.get(error, 'unexpected')
        results.append({**row, 'status': 'passed' if passed else 'failed', 'enforcement_layer': layer})
    return results


def assess_raw_cache(value):
    """Classify the observed refusal layer; never infer direct virtqueue coverage."""
    base = (type(value.get('schema')) is int and value['schema'] == 1
            and value.get('phase') == 'before-mount-and-keeper'
            and value.get('clear_ro_rc') == 0 and value.get('inspect_ro_rc') == 0
            and value.get('cache_open_write_rc') == 0 and value.get('cache_write_rc') == -1
            and value.get('cache_sector_unchanged') is True
            and value.get('private_write_read_restore') is True)
    ro, error = value.get('guest_ro_after_clear'), value.get('cache_write_errno')
    layer = 'unqualified'
    if type(ro) is int and type(error) is int:
        if ro == 1 and error in (1, 30):
            layer = 'guest-block-readonly'
        elif ro == 0 and error in (1, 5, 30):
            layer = 'block-io-path-with-guest-ro-cleared'
    return {'status': 'passed' if base and layer != 'unqualified' else 'failed',
            'enforcement_layer': layer, 'direct_vmm_backend_bypass_qualified': False,
            'evidence': value}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--adapter', choices=['libkrun', 'hvi'], required=True)
    parser.add_argument('--fixtures', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads((args.fixtures / 'build.json').read_text())
    guests = [json.loads((args.fixtures / name / 'guest.json').read_text()) for name in ('challenge', 'ordinary')]
    validate_pair(*guests)
    probe = args.fixtures / 'boundary-probe'
    require(artifact(probe) == manifest['probe'], 'probe executable identity changed')
    require_no_active_collector(ROOT / 'runs')
    lease = (ROOT / 'artifacts/nested-runner.lock').open('a')
    fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
    os.umask(0o077)
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=False)
    helper = ROOT / ('libkrun/target/debug/devcroft-krun-spike' if args.adapter == 'libkrun'
                     else 'hvi/target/debug/devcroft-hvi-spike')
    expected_helper = artifact(helper)
    source_cache = ROOT / 'artifacts/workload/toolchain.img'
    source_identity = artifact(source_cache)
    copied_cache = args.out / 'disposable-cache.img'
    sparse_copy(source_cache, copied_cache)
    require(artifact(copied_cache)['sha256'] == source_identity['sha256'], 'cache copy differs from source')
    report = {'schema': 1, 'status': 'failed', 'platform': 'macos-arm64-native', **helper_identity(helper),
              'qualification': 'layer-aware IPv4/no-route and copied immutable-cache sample; no general egress firewall or production cache qualification',
              'raw_evidence': str(args.out), 'helper': expected_helper, 'probe': artifact(probe),
              'kernel': artifact(ROOT / 'artifacts/kernel-landlock/Image'),
              'kernel_config': artifact(ROOT / 'artifacts/kernel-landlock/config'),
              'shared_source_cache': source_identity,
              'cache_copy': {'recorded_path': str(copied_cache), 'before_sha256': source_identity['sha256']},
              'guests': guests, 'fixture_build': artifact(args.fixtures / 'build.json'),
              'checks': [], 'instances': [], 'observations': []}
    sources = ['boundary_probe.py', 'build_boundary_guest.py', 'build_guest.py', 'guest/init.c',
               'guest/boundary_probe.c', 'guest-keeper/src/linux.rs', 'keeper_probe.py', 'protocol_probe.py',
               'ssh_smoke.py', 'smoke.py', 'workload_smoke.py', 'fault_probe.py', 'stop_receipt.py']
    for name in sources:
        target = args.out / 'sources' / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes((ROOT / name).read_bytes())
    vms, canaries, ids = [], [], []
    listener = None
    live_socket = None

    def save():
        (args.out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')

    def identity(vm, label):
        return owned_identity(vm.process, ROOT / 'artifacts/measure-macos', args.out / f'{label}-identity.jsonl')

    def alive(label):
        for canary in canaries:
            canary.echo(f'{label}:{canary.vm.label}\n'.encode())
        require(identity(vms[1], label + '-sibling') == ids[1], 'sibling helper changed')

    def child(vm, label):
        out, err, status = wire.execute(vm.path / 'keeper.sock', '/workspace/boundary-probe')
        (args.out / f'{label}.stdout').write_bytes(out)
        (args.out / f'{label}.stderr').write_bytes(err)
        require(not err and status == {'code': 0, 'signal': None}, (label, err, status))
        data = json.loads(out)
        rows = assess_child(data, vm.id)
        for row in rows:
            report['checks'].append({'phase': label, **row})
        report['observations'].append({'phase': label, 'udp_unlisted_loopback': 'delivered; not filtered by Landlock TCP rules',
            'ipv4_interfaces': data['ipv4_interfaces'], 'ipv6': 'not-run: compiled out of pinned kernel',
            'ipv6_socket_errno': data['ipv6_socket_errno'], 'http_instance_id': data['http_instance_id']})
        alive(label)
        save()
        require(all(row['status'] == 'passed' for row in rows), (label, [r for r in rows if r['status'] != 'passed']))

    def backend(label):
        reply = control_reply(vms[0], b'TEST CACHE WRITE')
        (args.out / f'{label}-raw-reply.json').write_bytes(reply)
        value = json.loads(reply)
        observed = assess_raw_cache(value)
        report['checks'].append({'phase': label, 'name': 'privileged-raw-cache-refusal',
                                 'boot': vms[0].boots[-1]['boot'], **observed})
        alive(label)
        save()
        require(observed['status'] == 'passed', ('raw cache refusal or positive control failed', value))

    try:
        for label, guest in zip(('a', 'sibling'), guests):
            vm = WorkloadInstance(label, args.out, helper, ROOT / 'artifacts/kernel-landlock/Image', guest,
                                  copied_cache, ROOT / 'artifacts/workload/data.seed.img')
            vms.append(vm)
            report['instances'].append({'label': label, 'boots': vm.boots})
        a, sibling = vms
        live_socket = a.controller_dir / 'boundary-canary.sock'
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(str(live_socket)); listener.listen(8)
        for name in ('host-shared-cache', 'host-checkout'):
            target = args.out / (name + '-canary')
            target.write_text('test-only protected host target\n')
            before = hashlib.sha256(target.read_bytes()).hexdigest()
            result = command([helper, 'controller-test', a.launch_path, target, live_socket], args.out, name, timeout=90)
            require(result.returncode == 0, result.stderr)
            evidence = json.loads(result.stdout)
            require(evidence['passed'] and evidence['outside_denied'] and evidence['outside_write_denied']
                    and evidence['outside_unix_connect_denied'] and evidence['runtime_write_denied'], evidence)
            require(hashlib.sha256(target.read_bytes()).hexdigest() == before, 'protected host file changed')
            report['checks'].append({'phase': 'host-profile', 'name': name, 'status': 'passed', 'evidence': evidence})
            for endpoint in SOCKETS:
                (a.path / endpoint).unlink(missing_ok=True)
        listener.close(); listener = None
        live_socket.unlink()
        for vm in vms:
            require(artifact(helper) == expected_helper, 'helper changed before launch')
            vm.prepare(validation_timeout=90)
            vm.start(); vm.ready(timeout=120)
            ids.append(identity(vm, vm.label + '-first'))
            canaries.append(Canary(vm))
            out, err, status = wire.execute(vm.path / 'keeper.sock',
                'cat > /workspace/boundary-probe; /bin/busybox chmod 700 /workspace/boundary-probe', probe.read_bytes())
            require((out, err, status) == (b'', b'', {'code': 0, 'signal': None}), 'probe upload failed')
        require(control_reply(sibling, b'TEST CACHE WRITE') == b'ERR command\n', 'ordinary guest enabled cache hook')
        report['checks'].append({'phase': 'ordinary-control', 'name': 'privileged-hook-refused', 'status': 'passed'})
        child(a, 'a-first'); child(sibling, 'sibling-first')
        backend('backend-first')
        require(identity(a, 'a-after-inputs') == ids[0], 'first helper unexpectedly restarted')
        canaries[0].close()
        a.shutdown()
        require(artifact(helper) == expected_helper, 'helper changed before cold restart')
        a.start(); a.ready(timeout=120)
        require(identity(a, 'a-restart') != ids[0], 'cold restart did not change helper identity')
        canaries[0] = Canary(a)
        out, err, status = wire.execute(a.path / 'keeper.sock', 'sha256sum /workspace/boundary-probe')
        require(not err and status['code'] == 0 and out.decode().split()[0] == manifest['probe']['sha256'],
                'existing uploaded probe did not persist')
        child(a, 'a-after-restart'); backend('backend-after-restart')
        for canary in canaries:
            canary.close()
        canaries.clear()
        for vm in vms:
            vm.shutdown()
        require(artifact(helper) == expected_helper, 'helper changed during run')
        report['cache_copy']['after_sha256'] = artifact(copied_cache)['sha256']
        require(report['cache_copy']['after_sha256'] == source_identity['sha256'], 'copied cache image changed')
        require(artifact(source_cache) == source_identity, 'original source cache changed')
        report['status'] = 'passed'
    except Exception as error:
        report.update(error=f'{type(error).__name__}: {error}', traceback=traceback.format_exc())
    finally:
        if listener is not None:
            listener.close()
        if live_socket is not None:
            live_socket.unlink(missing_ok=True)
        for canary in canaries:
            canary.sock.close()
        for vm in vms:
            vm.stop()
        report['cleanup'] = {'helpers_reaped': all(vm.process is None for vm in vms),
            'endpoints_removed': all(not (vm.path / name).exists() for vm in vms for name in SOCKETS)}
        if all(report['cleanup'].values()):
            report['discarded_private_resources'] = []
            for vm in vms:
                for directory in (vm.path, vm.controller_dir):
                    shutil.rmtree(directory)
                    report['discarded_private_resources'].append(str(directory))
            report['cleanup']['private_resources_removed'] = True
            if report['status'] == 'passed':
                copied_cache.unlink()
                report['cleanup']['verified_cache_copy_removed'] = True
        else:
            report.update(status='failed', error='owned helper or endpoint survived cleanup')
        save()
    print(json.dumps({'status': report['status'], 'checks': len(report['checks']), 'error': report.get('error')}), flush=True)
    raise SystemExit(report['status'] != 'passed')


if __name__ == '__main__':
    main()
