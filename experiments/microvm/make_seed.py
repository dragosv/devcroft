#!/usr/bin/env python3
"""Create an empty ext4 seed of a chosen size in the mount-free Linux builder.

macOS has no mkfs.ext4 here. The builder formats the image and gzips it; the
host expands it as a sparse file so an empty multi-GiB seed costs only its
metadata on disk. The expanded image's SHA-256 is recorded in a receipt.
"""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent
REMOTE = '/tmp/devcroft-seed'
CHUNK = 1024 * 1024


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mib', type=int, required=True)
    parser.add_argument('--uuid', default='d31c0006-0000-4000-8000-000000000006')
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    if not 128 <= args.mib <= 8192 or args.out.exists():
        parser.error('size must be 128..8192 MiB and --out must not exist')
    env = dict(os.environ, LIMA_HOME='/private/tmp/devcroft-kernel-builder')

    def lima(*argv):
        subprocess.run(['limactl', *map(str, argv)], check=True, env=env)

    args.out.parent.mkdir(parents=True, exist_ok=True)
    packed = args.out.with_suffix('.img.gz')
    try:
        lima('start', '--tty=false', 'kernel-build')
        lima('shell', 'kernel-build', 'sudo', 'apk', 'add', '--no-cache', 'e2fsprogs')
        lima('shell', 'kernel-build', 'sh', '-c',
             f'set -eu; rm -rf {REMOTE}; mkdir -p {REMOTE}; cd {REMOTE}; truncate -s {args.mib}M seed.img; '
             f'mkfs.ext4 -F -q -m 0 -O ^orphan_file -E nodiscard -U {args.uuid} seed.img; gzip -1 seed.img')
        lima('copy', f'kernel-build:{REMOTE}/seed.img.gz', packed)
        lima('shell', 'kernel-build', 'rm', '-rf', REMOTE)
    finally:
        lima('stop', 'kernel-build')
    digest = hashlib.sha256()
    with gzip.open(packed, 'rb') as source, args.out.open('xb') as target:
        while chunk := source.read(CHUNK):
            digest.update(chunk)
            if chunk.count(0) == len(chunk):
                target.seek(len(chunk), os.SEEK_CUR)
            else:
                target.write(chunk)
        target.truncate()
    packed.unlink()
    size = args.out.stat().st_size
    if size != args.mib * CHUNK:
        raise RuntimeError(f'unexpected seed size {size}')
    receipt = {'schema': 1, 'filesystem': 'ext4, empty', 'mib': args.mib, 'uuid': args.uuid,
               'sha256': digest.hexdigest(), 'allocated_bytes': args.out.stat().st_blocks * 512,
               'script_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
    args.out.with_suffix('.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(receipt, indent=2))


if __name__ == '__main__':
    main()
