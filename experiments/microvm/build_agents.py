#!/usr/bin/env python3
"""Materialize both real agent CLIs without changing the baseline Nix image."""
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent
REMOTE = '/home/dragos.guest/devcroft-agents'


def main():
    out = ROOT / 'artifacts/agents'
    out.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, LIMA_HOME='/private/tmp/devcroft-kernel-builder')

    def lima(*args):
        subprocess.run(['limactl', *map(str, args)], check=True, env=env)

    try:
        lima('start', '--tty=false', 'kernel-build')
        lima('shell', 'kernel-build', 'sudo', 'apk', 'add', '--no-cache', 'python3', 'curl', 'xz', 'coreutils', 'e2fsprogs')
        lima('shell', 'kernel-build', 'mkdir', '-p', REMOTE + '/definition')
        for file, name in [
            (ROOT / 'nix/build.py', 'build.py'),
            (ROOT / 'fixtures/nix-bootstrap-aarch64.json', 'bootstrap.json'),
            (ROOT / 'agents/flake.nix', 'definition/flake.nix'),
            (ROOT / 'agents/flake.lock', 'definition/flake.lock'),
            (ROOT / 'fixtures/workload-source.json', 'source.json'),
            (ROOT / 'artifacts/workload/xxhash.tar.gz', 'xxhash.tar.gz'),
        ]:
            lima('copy', file, f'kernel-build:{REMOTE}/{name}')
        args = ['python3', REMOTE + '/build.py', '--work-dir', REMOTE]
        for binary in ('gcc', 'make', 'redis-server', 'codex', 'claude', 'python3', 'curl'):
            args.extend(['--required-binary', binary])
        lima('shell', 'kernel-build', *args)
        for name in ('toolchain.img', 'data.seed.img', 'cache.tar', 'closure.json', 'build.json'):
            lima('copy', f'kernel-build:{REMOTE}/{name}', out / name)
        receipt = json.loads((out / 'build.json').read_text())
        for name in ('toolchain.img', 'data.seed.img', 'cache.tar', 'closure.json'):
            with (out / name).open('rb') as file:
                if hashlib.file_digest(file, 'sha256').hexdigest() != receipt[name]['sha256']:
                    raise RuntimeError(f'exported agent fixture hash mismatch: {name}')
        # Only verified, reconstructible copies in this dedicated namespace.
        lima('shell', 'kernel-build', 'sudo', 'rm', '-rf', REMOTE + '/root', REMOTE + '/cache',
             REMOTE + '/toolchain.img', REMOTE + '/cache.tar')
        lima('shell', 'kernel-build', 'sudo', 'fstrim', '-v', '/')
        print((out / 'build.json').read_text())
    finally:
        lima('stop', 'kernel-build')


if __name__ == '__main__':
    main()
