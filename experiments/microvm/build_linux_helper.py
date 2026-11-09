#!/usr/bin/env python3
"""Cross-build the static ARM64 Linux helper; no VM or permissive policy mode."""
import argparse
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--clippy', action='store_true')
    args = parser.parse_args()
    subprocess.run(['python3', str(ROOT / 'build_helper.py'), '--prepare-only'], check=True)
    target = 'aarch64-unknown-linux-musl'
    linker = str(ROOT / 'guest-keeper/zig-linker.sh')
    env = dict(os.environ)
    env.update(CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=linker,
               CC_aarch64_unknown_linux_musl=linker,
               AR_aarch64_unknown_linux_musl=str(ROOT / 'guest-keeper/zig-ar.sh'),
               CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUSTFLAGS='-C target-feature=+crt-static -C link-self-contained=no')
    command = ['cargo', 'clippy' if args.clippy else 'build', '--manifest-path', str(ROOT / 'libkrun/Cargo.toml'),
               '--target', target, '--locked']
    if args.clippy:
        command.extend(['--', '-D', 'warnings'])
    subprocess.run(command, env=env, check=True)


if __name__ == '__main__':
    main()
