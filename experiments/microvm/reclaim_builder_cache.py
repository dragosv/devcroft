#!/usr/bin/env python3
"""Prune only unreferenced Nix build dependencies in the owned Linux builder."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import subprocess
import time

from cold_prepare import require_no_active_collector
from smoke import ROOT


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    require_no_active_collector(ROOT / 'runs')
    lease = (ROOT / 'artifacts/nested-runner.lock').open('a')
    fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
    os.umask(0o077)
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=False)
    publication = json.loads((ROOT / 'artifacts/minimal-inputs/manifest.json').read_text())
    if publication['status'] != 'passed':
        raise RuntimeError('fixed-input publication must be complete before pruning')
    roots = [json.loads((ROOT / f'fixtures/{name}-aarch64-build.json').read_text())['profile'] for name in ('nix', 'agents')]
    roots.append(publication['nixpkgs']['path'])
    report = {'schema': 1, 'status': 'running', 'qualification': 'owned experimental builder cache cleanup; original runtime/evidence retained',
              'raw_evidence': str(args.out), 'protected_roots': roots, 'phases': []}
    env = dict(os.environ, LIMA_HOME='/private/tmp/devcroft-kernel-builder')
    disk = Path('/private/tmp/devcroft-kernel-builder/kernel-build/disk')
    report['before_allocated_bytes'] = disk.stat().st_blocks * 512
    started = False

    def save():
        (args.out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')

    def lima(name, *argv):
        row = {'name': name, 'argv': ['limactl', *map(str, argv)]}
        report['phases'].append(row)
        save()
        began = time.monotonic()
        with (args.out / f'{name}.stdout').open('wb') as stdout, (args.out / f'{name}.stderr').open('wb') as stderr:
            code = subprocess.run(row['argv'], env=env, stdout=stdout, stderr=stderr, timeout=600).returncode
        row.update(exit_code=code, seconds=time.monotonic() - began)
        save()
        print(json.dumps(row), flush=True)
        if code:
            raise RuntimeError(name + ' failed; logs retained')
        return (args.out / f'{name}.stdout').read_text()

    try:
        initial = lima('precondition', 'list', '--json')
        if any(json.loads(line)['status'] != 'Stopped' for line in initial.splitlines() if line):
            raise RuntimeError('builder must initially be stopped')
        started = True
        lima('start', 'start', '--tty=false', 'kernel-build')
        root_dir = '/home/dragos.guest/devcroft-preserved-inputs'
        lima('gc-root-directory', 'shell', 'kernel-build', 'mkdir', '-p', root_dir)
        installed = lima('nix-profile-root', 'shell', 'kernel-build', 'readlink', '-f', '/home/dragos.guest/.nix-profile').strip()
        roots.append(installed)
        for index, root in enumerate(roots):
            if not root.startswith('/nix/store/') or '/' in root[len('/nix/store/'):]:
                raise RuntimeError('unexpected protected store root')
            lima(f'protect-{index}', 'shell', 'kernel-build', '/home/dragos.guest/.nix-profile/bin/nix-store',
                 '--add-root', root_dir + '/root-' + str(index), '--indirect', '--realise', root)
        nix_store = '/home/dragos.guest/.nix-profile/bin/nix-store'
        flags = ['--option', 'keep-derivations', 'false', '--option', 'keep-outputs', 'false', '--gc']
        dead = lima('garbage-plan', 'shell', 'kernel-build', nix_store, *flags, '--print-dead').splitlines()
        if set(dead) & set(roots):
            raise RuntimeError('garbage plan contains a protected root')
        report['unreferenced_paths_in_plan'] = len(dead)
        lima('prune-unreferenced', 'shell', 'kernel-build', nix_store, *flags)
        lima('verify-protected-contents', 'shell', 'kernel-build', 'env',
             'NIX_CONFIG=experimental-features = nix-command flakes\nbuild-users-group =\n',
             '/home/dragos.guest/.nix-profile/bin/nix', 'store', 'verify', '--recursive', '--no-trust', *roots)
        lima('trim', 'shell', 'kernel-build', 'sudo', 'fstrim', '-v', '/')
        lima('stop', 'stop', 'kernel-build')
        started = False
        report.update(status='passed', after_allocated_bytes=disk.stat().st_blocks * 512,
                      protected_contents_verified=True, host_runtime_images_and_raw_evidence_retained=True)
    except BaseException as error:
        report.update(status='failed', error=f'{type(error).__name__}: {error}')
    finally:
        if started:
            try:
                lima('finally-stop', 'stop', 'kernel-build')
            except Exception as error:
                report['cleanup_error'] = str(error)
        save()
    print(json.dumps({key: report.get(key) for key in ('status', 'before_allocated_bytes', 'after_allocated_bytes', 'error')}), flush=True)
    raise SystemExit(report['status'] != 'passed')


if __name__ == '__main__':
    main()
