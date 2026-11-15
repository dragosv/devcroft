#!/usr/bin/env python3
"""Build an opt-in cache challenge pair with an already qualified keeper binary."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

from build_guest import ROOT, build
from smoke import artifact


def verify_reuse(receipt, keeper, workspace):
    if hashlib.sha256(keeper.read_bytes()).hexdigest() != receipt['keeper_sha256']:
        raise ValueError('reused keeper binary does not match its receipt')
    sources = {name: digest for name, digest in receipt['sources'].items()
               if name.startswith('src/keeper/') or name.startswith('experiments/microvm/guest-keeper/')}
    if not sources or 'src/keeper/protocol.rs' not in sources:
        raise ValueError('reused keeper source identities are missing')
    for name, expected in sources.items():
        if hashlib.sha256((workspace / name).read_bytes()).hexdigest() != expected:
            raise ValueError('reused keeper source drift: ' + name)
    return sources


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--keeper-receipt', type=Path, default=ROOT / 'artifacts/stage-b-protocol-final/guest.json')
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    if args.out.exists():
        parser.error('--out must be a new directory')
    receipt = json.loads(args.keeper_receipt.read_text())
    keeper = ROOT / 'guest-keeper/target/aarch64-unknown-linux-musl/release/devcroft-guest-keeper'
    sources = verify_reuse(receipt, keeper, ROOT.parent.parent)
    busybox = args.keeper_receipt.parent / 'busybox'
    if hashlib.sha256(busybox.read_bytes()).hexdigest() != receipt['busybox_sha256']:
        raise ValueError('busybox identity changed')
    zig = shutil.which('zig')
    if not zig:
        raise RuntimeError('installed Zig required')
    args.out = args.out.resolve()
    args.out.mkdir(parents=True)
    env = dict(os.environ, ZIG_GLOBAL_CACHE_DIR=str(ROOT / '.zig-cache'))
    subprocess.run([zig, 'cc', '-target', 'aarch64-linux-musl', '-static', '-O2',
                    '-Wall', '-Wextra', '-Werror', str(ROOT / 'guest/boundary_probe.c'),
                    '-o', str(args.out / 'boundary-probe')], env=env, check=True)
    for name, enabled in [('ordinary', False), ('challenge', True)]:
        out = args.out / name
        build(out, 'aarch64', zig, keeper=keeper, busybox=busybox, workload=True,
              cache_boundary_probe=enabled)
        path = out / 'guest.json'
        guest = json.loads(path.read_text())
        guest['reused_keeper_receipt'] = artifact(args.keeper_receipt)
        guest['keeper_sources'] = sources
        path.write_text(json.dumps(guest, indent=2) + '\n')
    manifest = {'schema': 1, 'qualification': 'disposable copied-cache fixture only; no ordinary guest root command added',
                'probe': artifact(args.out / 'boundary-probe'),
                'ordinary': artifact(args.out / 'ordinary/guest.json'),
                'challenge': artifact(args.out / 'challenge/guest.json'),
                'sources': {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest()
                            for name in ('guest/boundary_probe.c', 'guest/init.c', 'build_guest.py', 'build_boundary_guest.py')}}
    (args.out / 'build.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps(manifest, indent=2))


if __name__ == '__main__':
    main()
