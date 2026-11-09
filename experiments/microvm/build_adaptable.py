#!/usr/bin/env python3
"""Prepare offline inputs for the private writable-base/setup-window experiment."""
import argparse
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent
REMOTE = '/home/dragos.guest/devcroft-adaptable'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--freeze-inputs', action='store_true')
    args = parser.parse_args()
    out = ROOT / 'artifacts/adaptable'
    out.mkdir(parents=True, exist_ok=True)
    pin = ROOT / 'fixtures/adaptable-recipe-aarch64.json'
    if not pin.exists() and not args.freeze_inputs:
        parser.error('initial package resolution requires --freeze-inputs')
    env = dict(os.environ, LIMA_HOME='/private/tmp/devcroft-kernel-builder')
    def lima(*args):
        subprocess.run(['limactl', *map(str, args)], check=True, env=env)
    try:
        lima('start', '--tty=false', 'kernel-build')
        lima('shell', 'kernel-build', 'sudo', 'apk', 'add', '--no-cache', 'python3', 'curl', 'e2fsprogs')
        lima('shell', 'kernel-build', 'mkdir', '-p', REMOTE)
        for file, name in [(ROOT / 'adaptable/build.py', 'build.py'), (ROOT / 'adaptable/recipe.py', 'recipe.py'),
            (ROOT / 'adaptable/probe.c', 'probe.c'), (ROOT / 'artifacts/workload/toolchain.img', 'base.img')]:
            lima('copy', file, f'kernel-build:{REMOTE}/{name}')
        if not args.freeze_inputs:
            lima('copy', pin, f'kernel-build:{REMOTE}/recipe.json')
        lima('shell', 'kernel-build', 'python3', REMOTE + '/build.py', *(['--freeze-inputs'] if args.freeze_inputs else []))
        for name in ('toolchain.img', 'data.seed.img', 'packages.tar', 'recipe.json', 'build.json'):
            lima('copy', f'kernel-build:{REMOTE}/{name}', out / name)
        if args.freeze_inputs:
            pin.write_bytes((out / 'recipe.json').read_bytes())
        print((out / 'build.json').read_text())
    finally:
        lima('stop', 'kernel-build')


if __name__ == '__main__':
    main()
