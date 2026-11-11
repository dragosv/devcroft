#!/usr/bin/env python3
"""Freeze already-downloaded Nix inputs and tools for a fresh minimal builder.

Uses only the dedicated experiment builder. Exported duplicate builder images
and materialization intermediates may be reclaimed after validating their host
receipts. The original runtime images and diagnostic evidence are retained.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import urllib.request

from cold_prepare import require_no_active_collector
from smoke import ROOT


def digest(path, algorithm='sha256'):
    with path.open('rb') as file:
        return hashlib.file_digest(file, algorithm).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--inputs', type=Path, required=True)
    args = parser.parse_args()
    require_no_active_collector(ROOT / 'runs')
    lock = (ROOT / 'artifacts/nested-runner.lock').open('a')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    os.umask(0o077)
    args.out = args.out.resolve()
    args.inputs = args.inputs.resolve()
    args.out.mkdir(parents=True, exist_ok=False)
    args.inputs.mkdir(parents=True, exist_ok=False)
    report = {'schema': 1, 'status': 'running', 'qualification': 'trusted fixed-input publication, excluded from downloaded-input preparation timing',
              'raw_evidence': str(args.out), 'inputs': str(args.inputs), 'phases': [], 'reclaimed': []}
    env = dict(os.environ, LIMA_HOME='/private/tmp/devcroft-kernel-builder')
    remote = '/home/dragos.guest/devcroft-minimal-inputs'
    started = False

    def save():
        (args.out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')

    def phase(name, argv, timeout=600):
        before = time.monotonic()
        row = {'name': name, 'argv': list(map(str, argv))}
        report['phases'].append(row)
        save()
        with (args.out / f'{name}.stdout').open('wb') as stdout, (args.out / f'{name}.stderr').open('wb') as stderr:
            code = subprocess.run(row['argv'], env=env, stdout=stdout, stderr=stderr, timeout=timeout).returncode
        row.update(exit_code=code, seconds=time.monotonic() - before)
        save()
        print(json.dumps(row), flush=True)
        if code:
            raise RuntimeError(f'{name} failed; raw logs retained')
        return (args.out / f'{name}.stdout').read_text()

    def lima(name, *argv, timeout=600):
        return phase(name, ['limactl', *argv], timeout)

    try:
        for prefix in ('nix', 'agents'):
            receipt = json.loads((ROOT / f'fixtures/{prefix}-aarch64-build.json').read_text())
            for filename in ('toolchain.img', 'data.seed.img', 'cache.tar'):
                if digest(ROOT / 'artifacts' / prefix / filename) != receipt[filename]['sha256']:
                    raise RuntimeError(f'host artifact changed: {prefix}/{filename}')
        before = Path('/private/tmp/devcroft-kernel-builder/kernel-build/disk').stat()
        report['builder_allocated_before'] = before.st_blocks * 512
        listing = lima('precondition', 'list', '--json')
        if any(json.loads(line)['status'] != 'Stopped' for line in listing.splitlines() if line):
            raise RuntimeError('dedicated builder must be stopped initially')
        started = True
        lima('start', 'start', '--tty=false', 'kernel-build')
        # Only our known output namespaces; never /nix/store or arbitrary paths.
        for prefix in ('nix', 'agents'):
            receipt = json.loads((ROOT / f'fixtures/{prefix}-aarch64-build.json').read_text())
            namespace = '/home/dragos.guest/devcroft-' + prefix
            check = '''import hashlib,json,pathlib,sys
p=pathlib.Path(sys.argv[1]); names=('toolchain.img','data.seed.img','cache.tar')
print(json.dumps({name:hashlib.file_digest((p/name).open('rb'),'sha256').hexdigest() for name in names if (p/name).exists()}))
'''
            hashes = json.loads(lima('check-exported-' + prefix, 'shell', 'kernel-build', 'python3', '-c', check, namespace))
            for name, value in hashes.items():
                if value != receipt[name]['sha256']:
                    raise RuntimeError(f'remote exported duplicate differs: {namespace}/{name}')
            lima('reclaim-exported-' + prefix, 'shell', 'kernel-build', 'sudo', 'rm', '-rf',
                 *[namespace + '/' + name for name in ('root', 'cache', *hashes)])
            report['reclaimed'].append({'namespace': namespace, 'verified_duplicate_hashes': hashes,
                                        'intermediates': ['root', 'cache'], 'host_exports_retained': True})
        lima('trim-before-export', 'shell', 'kernel-build', 'sudo', 'fstrim', '-v', '/')
        lima('input-directory', 'shell', 'kernel-build', 'mkdir', '-p', remote + '/cache', remote + '/apk')
        nix = '/home/dragos.guest/.nix-profile/bin/nix'
        nix_config = 'NIX_CONFIG=experimental-features = nix-command flakes\nbuild-users-group =\n'
        archive = json.loads(lima('flake-input-closure', 'shell', 'kernel-build', 'env', nix_config,
                                   nix, 'flake', 'archive', '--offline', '--json',
                                   'path:/home/dragos.guest/devcroft-agents/definition'))
        nixpkgs = archive['inputs']['nixpkgs']['path']
        info = json.loads(lima('nixpkgs-content', 'shell', 'kernel-build', 'env', nix_config,
                               nix, 'path-info', '--json', nixpkgs))
        lockfile = json.loads((ROOT / 'agents/flake.lock').read_text())
        if info[nixpkgs]['narHash'] != lockfile['nodes']['nixpkgs']['locked']['narHash']:
            raise RuntimeError('nixpkgs source differs from the fixed lock')
        receipt = json.loads((ROOT / 'fixtures/agents-aarch64-build.json').read_text())
        report['nixpkgs'] = {'path': nixpkgs, **info[nixpkgs]}
        lima('publish-nars', 'shell', 'kernel-build', 'env', nix_config,
             nix, 'copy', '--offline', '--to', 'file://' + remote + '/cache', receipt['profile'], nixpkgs)
        # Keep only the public immutable-fetch cache, copied through SQLite's
        # backup API while the dedicated builder is idle. No operator config.
        backup = '''import pathlib,sqlite3,sys
out=pathlib.Path(sys.argv[1]); base=pathlib.Path.home()/'.cache/nix'
for source in sorted(base.glob('fetcher-cache-*.sqlite')):
 with sqlite3.connect('file:'+str(source)+'?mode=ro',uri=True) as src,sqlite3.connect(out/source.name) as dst: src.backup(dst)
 print(source.name)
'''
        report['fetcher_databases'] = lima('fetcher-cache-backup', 'shell', 'kernel-build', 'python3', '-c', backup, remote).splitlines()
        if not report['fetcher_databases']:
            raise RuntimeError('no immutable fetch metadata found')
        tools = ['coreutils', 'e2fsprogs', 'curl', 'xz', 'tar', 'util-linux', 'python3', 'sudo', 'bash']
        lima('download-tool-apks', 'shell', 'kernel-build', 'apk', 'fetch', '--recursive', '--output', remote + '/apk', *tools)
        lima('bundle-inputs', 'shell', 'kernel-build', 'tar', '-cf', remote + '/inputs.tar', '-C', remote,
             'cache', 'apk', *report['fetcher_databases'])
        lima('export-inputs', 'copy', 'kernel-build:' + remote + '/inputs.tar', str(args.inputs / 'inputs.tar'))
        lima('export-bootstrap', 'copy', 'kernel-build:/home/dragos.guest/devcroft-nix/nix-bootstrap.tar.xz',
             str(args.inputs / 'nix-bootstrap.tar.xz'))
        bootstrap = json.loads((ROOT / 'fixtures/nix-bootstrap-aarch64.json').read_text())
        if digest(args.inputs / 'nix-bootstrap.tar.xz') != bootstrap['sha256']:
            raise RuntimeError('bootstrap archive changed')
        lima('remove-export-intermediates', 'shell', 'kernel-build', 'rm', '-rf',
             remote + '/cache', remote + '/apk', remote + '/inputs.tar')
        lima('trim-after-export', 'shell', 'kernel-build', 'sudo', 'fstrim', '-v', '/')
        lima('stop', 'stop', 'kernel-build')
        started = False
        image_url = 'https://dl-cdn.alpinelinux.org/alpine/v3.23/releases/cloud/nocloud_alpine-3.23.3-aarch64-uefi-cloudinit-r0.qcow2'
        image_sha512 = '7a3cdfaefb0cbf3bb6824cd6ae80d6a3e0b0e367609e5fc50c5f714374d31e8d70dd607094a4e9cfe2e6ead7537781782469fe93bcea79611fbce4ddeacc92e1'
        with urllib.request.urlopen(image_url, timeout=180) as source, (args.inputs / 'base.qcow2').open('wb') as target:
            shutil.copyfileobj(source, target)
        if digest(args.inputs / 'base.qcow2', 'sha512') != image_sha512:
            raise RuntimeError('minimal builder base image hash mismatch')
        report['base_image'] = {'url': image_url, 'sha512': image_sha512}
        report['builder_allocated_after'] = Path('/private/tmp/devcroft-kernel-builder/kernel-build/disk').stat().st_blocks * 512
        report['files'] = {file.name: {'sha256': digest(file), 'bytes': file.stat().st_size}
                           for file in args.inputs.iterdir() if file.is_file()}
        report['original_profile'] = receipt['profile']
        report['original_closure_nar_hashes'] = receipt['closure_nar_hashes']
        report['status'] = 'passed'
        (args.inputs / 'manifest.json').write_text(json.dumps(report, indent=2) + '\n')
    except BaseException as error:
        report.update(status='failed', error=f'{type(error).__name__}: {error}')
    finally:
        if started:
            try:
                lima('finally-stop', 'stop', 'kernel-build')
            except Exception as error:
                report['cleanup_error'] = str(error)
        save()
    print(json.dumps({'status': report['status'], 'error': report.get('error')}), flush=True)
    raise SystemExit(report['status'] != 'passed')


if __name__ == '__main__':
    main()
