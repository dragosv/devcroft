#!/usr/bin/env python3
"""Build the pinned ARM64 Nix fixture inside the dedicated Linux builder."""
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent
REMOTE = '/home/dragos.guest/devcroft-nix'


def main():
    out = ROOT / 'artifacts/nix'
    out.mkdir(parents=True, exist_ok=True)
    source = ROOT / 'artifacts/workload/xxhash.tar.gz'
    if not source.is_file():
        raise RuntimeError('fetch the pinned workload source with build_workload.py first')
    env = dict(os.environ, LIMA_HOME='/private/tmp/devcroft-kernel-builder')
    def lima(*args):
        subprocess.run(['limactl', *map(str, args)], check=True, env=env)
    try:
        lima('start', '--tty=false', 'kernel-build')
        lima('shell', 'kernel-build', 'sudo', 'apk', 'add', '--no-cache', 'python3', 'curl', 'xz', 'coreutils', 'e2fsprogs')
        lima('shell', 'kernel-build', 'mkdir', '-p', REMOTE + '/definition')
        for file, name in [(ROOT / 'nix/build.py', 'build.py'),
            (ROOT / 'fixtures/nix-bootstrap-aarch64.json', 'bootstrap.json'),
            (ROOT / 'nix/flake.nix', 'definition/flake.nix'), (ROOT / 'nix/flake.lock', 'definition/flake.lock'),
            (ROOT / 'fixtures/workload-source.json', 'source.json'), (source, 'xxhash.tar.gz')]:
            lima('copy', file, f'kernel-build:{REMOTE}/{name}')
        lima('shell', 'kernel-build', 'python3', REMOTE + '/build.py')
        for name in ('toolchain.img', 'data.seed.img', 'cache.tar', 'closure.json', 'build.json'):
            lima('copy', f'kernel-build:{REMOTE}/{name}', out / name)
        print((out / 'build.json').read_text())
    finally:
        lima('stop', 'kernel-build')


if __name__ == '__main__':
    main()
