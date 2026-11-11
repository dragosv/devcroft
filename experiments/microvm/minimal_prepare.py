#!/usr/bin/env python3
"""Measure a fresh mount-free private builder and complete retained storage."""
import argparse
import datetime
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

from cold_prepare import require_no_active_collector
from density import usage
from smoke import ROOT


def digest(path):
    with path.open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--inputs', type=Path, default=ROOT / 'artifacts/minimal-inputs')
    args = parser.parse_args()
    require_no_active_collector(ROOT / 'runs')
    lease = (ROOT / 'artifacts/nested-runner.lock').open('a')
    fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
    os.umask(0o077)
    args.out, args.inputs = args.out.resolve(), args.inputs.resolve()
    args.out.mkdir(parents=True, exist_ok=False)
    began = time.monotonic()
    publication = json.loads((args.inputs / 'manifest.json').read_text())
    protocol = json.loads((ROOT / 'fixtures/minimal-preparation-protocol.json').read_text())
    thresholds = json.loads((ROOT / 'fixtures/no-go-thresholds.json').read_text())
    if digest(ROOT / 'fixtures/no-go-thresholds.json') != protocol['thresholds_sha256'] or publication['status'] != 'passed':
        raise RuntimeError('unverified publication or changed accepted thresholds')
    for name, record in publication['files'].items():
        if Path(name).name != name or digest(args.inputs / name) != record['sha256']:
            raise RuntimeError('downloaded minimal input changed')
    lima_home = Path('/private/tmp/dc-min-' + hashlib.sha256(str(args.out).encode()).hexdigest()[:10])
    if lima_home.exists():
        raise RuntimeError('private builder namespace already exists')
    lima_home.mkdir(mode=0o700)
    env = dict(os.environ, LIMA_HOME=str(lima_home))
    remote = '/home/devcroft/project'
    report = {'schema': 1, 'status': 'running', 'qualification': 'fresh private downloaded-input closure reconstruction; not source compilation or production preparation confinement',
              'platform': 'macos-arm64-native', 'raw_evidence': str(args.out), 'builder_namespace': str(lima_home),
              'protocol': protocol, 'publication_sha256': digest(args.inputs / 'manifest.json'), 'phases': []}

    def save():
        (args.out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')

    def phase(name, argv, timeout=900):
        row = {'name': name, 'argv': list(map(str, argv)), 'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat()}
        report['phases'].append(row)
        save()
        started = time.monotonic()
        with (args.out / f'{name}.stdout').open('wb') as stdout, (args.out / f'{name}.stderr').open('wb') as stderr:
            code = subprocess.run(row['argv'], env=env, stdout=stdout, stderr=stderr, timeout=timeout).returncode
        row.update(exit_code=code, seconds=time.monotonic() - started)
        save()
        print(json.dumps({'phase': name, 'seconds': row['seconds'], 'exit_code': code}), flush=True)
        if code:
            raise RuntimeError(name + ' failed; see retained logs')
        return (args.out / f'{name}.stdout').read_text()

    def lima(name, *argv, timeout=900):
        return phase(name, ['limactl', *argv], timeout)

    inputs = [(ROOT / 'nix/build.py', 'build.py'), (ROOT / 'minimal/driver.py', 'driver.py'),
              (ROOT / 'agents/flake.nix', 'definition/flake.nix'), (ROOT / 'agents/flake.lock', 'definition/flake.lock'),
              (ROOT / 'fixtures/nix-bootstrap-aarch64.json', 'bootstrap.json'),
              (ROOT / 'fixtures/workload-source.json', 'source.json'),
              (ROOT / 'artifacts/workload/xxhash.tar.gz', 'xxhash.tar.gz'),
              (args.inputs / 'manifest.json', 'publication.json')]
    frozen = args.out / 'sources'
    for source, name in inputs:
        target = frozen / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(source.read_bytes())
    template = {'vmType': 'vz', 'arch': 'aarch64', 'cpus': 2, 'memory': '4GiB', 'disk': '10GiB',
                'images': [{'location': str(args.inputs / 'base.qcow2'), 'arch': 'aarch64',
                            'digest': 'sha512:' + publication['base_image']['sha512']}],
                'mounts': [], 'containerd': {'system': False, 'user': False}, 'propagateProxyEnv': False,
                'upgradePackages': False, 'ssh': {'loadDotSSHPubKeys': False, 'forwardAgent': False, 'forwardX11': False},
                'portForwards': [{'guestPortRange': [1, 65535], 'ignore': True}],
                'user': {'name': 'devcroft', 'comment': 'Private materializer', 'home': '/home/devcroft', 'uid': 1000, 'shell': '/bin/bash'}}
    template_path = args.out / 'builder.yaml'
    template_path.write_text(json.dumps(template, indent=2) + '\n')
    builder_started = False
    try:
        reserve = 2 * 1024**3
        if shutil.disk_usage('/private/tmp').free < reserve + 5 * 1024**3:
            raise RuntimeError('insufficient host headroom for a fresh builder and exported image; reserve retained')
        builder_started = True
        lima('fresh-builder-start', 'start', '--tty=false', '--name=materialize', template_path)
        lima('project-directory', 'shell', 'materialize', 'mkdir', '-p', remote + '/definition')
        for index, (source, name) in enumerate(inputs):
            lima(f'input-{index}', 'copy', str(frozen / name), f'materialize:{remote}/{name}')
        for name in ('inputs.tar', 'nix-bootstrap.tar.xz'):
            lima('copy-' + name, 'copy', str(args.inputs / name), f'materialize:{remote}/{name}')
        lima('offline-reconstruct', 'shell', 'materialize', 'sudo', 'unshare', '--net', 'python3', remote + '/driver.py', remote)
        for name in ('build.json', 'closure.json', 'materialization-memory.txt'):
            lima('receipt-' + name, 'copy', f'materialize:{remote}/{name}', str(args.out / name))
        built = json.loads((args.out / 'build.json').read_text())
        if built['closure_nar_hashes'] != publication['original_closure_nar_hashes'] or built['profile'] != publication['original_profile']:
            raise RuntimeError('clean reconstruction differs from frozen closure')
        lima('remove-offline-intermediates', 'shell', 'materialize', 'sudo', 'rm', '-rf',
             *[remote + '/' + name for name in ('root', 'cache', 'cache.tar', 'inputs.tar', 'apk', 'bootstrap', 'nix-bootstrap.tar.xz')])
        lima('trim-offline-intermediates', 'shell', 'materialize', 'sudo', 'fstrim', '-v', '/')
        for name in ('toolchain.img', 'data.seed.img'):
            lima('export-' + name, 'copy', f'materialize:{remote}/{name}', str(args.out / name))
            if digest(args.out / name) != built[name]['sha256']:
                raise RuntimeError('exported materialization differs from its receipt')
        lima('remove-builder-export-copies', 'shell', 'materialize', 'rm', '-f', remote + '/toolchain.img', remote + '/data.seed.img')
        lima('trim-builder-exports', 'shell', 'materialize', 'sudo', 'fstrim', '-v', '/')
        lima('stop-builder', 'stop', 'materialize')
        builder_started = False
        report['offline_preparation_seconds'] = time.monotonic() - began
        phase('clean-guest-verification', ['python3', ROOT / 'agent_probe.py', '--adapter', 'libkrun',
              '--guest', ROOT / 'artifacts/stage-b-agents-ssh-loss-fix-final/guest.json', '--repetitions', '1', '--verify-workload',
              '--image', args.out / 'toolchain.img', '--seed', args.out / 'data.seed.img', '--out', args.out / 'verification'])
        report['end_to_end_with_verification_seconds'] = time.monotonic() - began
        verified = json.loads((args.out / 'verification/report.json').read_text())
        guest = verified['guest']
        runtime = [Path(verified['helper']['path']), Path(verified['kernel']['path']),
                   Path(guest['initramfs']['path']), Path(guest['runtime']['path'])]
        private = [Path(verified['instance']['path']), Path(verified['instance']['controller_dir'])]
        report['storage'] = {'builder': usage([lima_home]), 'downloaded_inputs': usage([args.inputs]),
                             'outputs_and_raw_receipts': usage([args.out]), 'common_runtime': usage(runtime),
                             'private_environment': usage(private),
                             'full_single_environment': usage([lima_home, args.inputs, args.out, *runtime, *private])}
        limits = thresholds['original_no_go_thresholds']
        report['criteria'] = {'cold_preparation': 'passed' if report['end_to_end_with_verification_seconds'] <= limits['cold_preparation_seconds'] else 'failed',
                              'complete_retained_single_environment_storage': 'passed' if report['storage']['full_single_environment']['allocated_bytes'] <= limits['physical_storage_per_environment_gib'] * 1024**3 else 'failed'}
        report['status'] = 'passed' if all(value == 'passed' for value in report['criteria'].values()) else 'failed'
        # The retained report includes the allocation inventory itself. Measure
        # again after writing it, until its physical allocation is stable.
        for _ in range(8):
            save()
            allocated = report['storage']['full_single_environment']['allocated_bytes']
            report['storage']['outputs_and_raw_receipts'] = usage([args.out])
            report['storage']['full_single_environment'] = usage([lima_home, args.inputs, args.out, *runtime, *private])
            report['criteria']['complete_retained_single_environment_storage'] = (
                'passed' if report['storage']['full_single_environment']['allocated_bytes'] <= limits['physical_storage_per_environment_gib'] * 1024**3 else 'failed')
            report['status'] = 'passed' if all(value == 'passed' for value in report['criteria'].values()) else 'failed'
            if report['storage']['full_single_environment']['allocated_bytes'] == allocated:
                break
        else:
            raise RuntimeError('retained report allocation did not stabilize')
    except BaseException as error:
        report.update(status='failed', error=f'{type(error).__name__}: {error}')
    finally:
        if builder_started:
            try:
                lima('finally-stop-builder', 'stop', 'materialize')
            except Exception as error:
                report['cleanup_error'] = str(error)
        save()
    print(json.dumps({key: report.get(key) for key in ('status', 'criteria', 'end_to_end_with_verification_seconds', 'error')}), flush=True)
    raise SystemExit(report['status'] != 'passed')


if __name__ == '__main__':
    main()
