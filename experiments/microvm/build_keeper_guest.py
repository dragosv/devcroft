#!/usr/bin/env python3
"""Cross-compile the real keeper + nono and package a pinned static-shell fixture."""
import gzip
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import urllib.request

from build_guest import ROOT, build


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workload", action="store_true")
    parser.add_argument("--nix", action="store_true", help="package the declared Nix workload fixture")
    parser.add_argument('--agents', action='store_true', help='separate pinned real-agent probe; implies --nix')
    parser.add_argument('--adaptable', action='store_true')
    parser.add_argument('--setup-window', action='store_true')
    parser.add_argument('--pid1-exit-probe', action='store_true',
                        help='compile fixed PID 1 exit injection into a separate Nix fault fixture')
    parser.add_argument('--setup-fault-probe', action='store_true',
                        help='compile test-only setup interruption stalls into a separate setup fixture')
    parser.add_argument('--out', type=Path, help='separate guest artifacts; preserve earlier measured images')
    args = parser.parse_args()
    if args.pid1_exit_probe and (not args.nix or not args.out or args.agents or args.adaptable):
        parser.error('--pid1-exit-probe requires --nix --out and excludes --agents/--adaptable')
    if args.pid1_exit_probe and args.out.exists() and (
            not args.out.is_dir() or any(p.name != 'busybox.apk' for p in args.out.iterdir())):
        parser.error('PID 1 exit fixture --out must be new or contain only its pinned busybox.apk')
    if args.setup_fault_probe and (not args.adaptable or not args.setup_window or not args.out):
        parser.error('--setup-fault-probe requires --adaptable --setup-window --out')
    if args.setup_fault_probe and args.out.exists() and (
            not args.out.is_dir() or any(p.name != 'busybox.apk' for p in args.out.iterdir())):
        parser.error('setup fault fixture --out must be new or contain only its pinned busybox.apk')
    ssh_pin = json.loads((ROOT / 'fixtures/ssh-consumption-windows.json').read_text())
    expected = {ROOT / 'russh' / name: digest for name, digest in ssh_pin['vendor_files'].items()}
    expected[ROOT / ssh_pin['keeper_fork']['path']] = ssh_pin['keeper_fork']['sha256']
    expected[ROOT / ssh_pin['patch']['path']] = ssh_pin['patch']['sha256']
    expected[ROOT / ssh_pin['keeper_fork']['patch']] = ssh_pin['keeper_fork']['patch_sha256']
    for source, digest in expected.items():
        if source.is_symlink() or hashlib.sha256(source.read_bytes()).hexdigest() != digest:
            raise RuntimeError('experimental SSH source pin changed: ' + str(source))
    if args.agents:
        args.nix = True
    if args.setup_window and not args.adaptable or args.nix and args.adaptable:
        parser.error('--setup-window requires --adaptable; --nix and --adaptable are exclusive')
    if args.adaptable:
        args.workload = True
    if args.nix:
        args.workload = True
    zig = shutil.which("zig")
    if not zig:
        raise RuntimeError("an installed Zig and the aarch64-unknown-linux-musl Rust target are required")
    crate = ROOT / "guest-keeper"
    env = dict(os.environ,
               CC_aarch64_unknown_linux_musl=str(crate / "zig-linker.sh"),
               AR_aarch64_unknown_linux_musl=str(crate / "zig-ar.sh"),
               CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=str(crate / "zig-linker.sh"),
               CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C link-self-contained=no")
    subprocess.run(["cargo", "build", "--manifest-path", str(crate / "Cargo.toml"), "--locked",
                    "--target", "aarch64-unknown-linux-musl", "--release"], check=True, env=env, timeout=900)
    out = ROOT / ("artifacts/stage-b-nix" if args.nix else "artifacts/stage-b-workload" if args.workload else "artifacts/stage-b")
    if args.adaptable:
        out = ROOT / ('artifacts/stage-c-setup' if args.setup_window else 'artifacts/stage-c-runtime')
    if args.agents:
        out = ROOT / 'artifacts/stage-b-agents'
    if args.out:
        out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    pin = json.loads((ROOT / "fixtures/busybox-aarch64.json").read_text())
    package = out / "busybox.apk"
    if not package.exists():
        with urllib.request.urlopen(pin["url"], timeout=120) as response:
            data = response.read(4 * 1024 * 1024 + 1)
        if len(data) > 4 * 1024 * 1024:
            raise RuntimeError("busybox package exceeds fixture budget")
        package.write_bytes(data)
    if hashlib.sha256(package.read_bytes()).hexdigest() != pin["sha256"]:
        raise RuntimeError("busybox package hash mismatch")
    # APK contains concatenated gzip/tar streams. Extract one regular member to
    # an assigned destination; no archive-owned paths or links reach the host.
    with tarfile.open(fileobj=io.BytesIO(gzip.decompress(package.read_bytes())),
                      mode="r:", ignore_zeros=True) as archive:
        members = [m for m in archive if m.name == pin["member"]]
        if len(members) != 1 or not members[0].isfile() or members[0].size > 4 * 1024 * 1024:
            raise RuntimeError("unexpected busybox payload")
        busybox = out / "busybox"
        busybox.write_bytes(archive.extractfile(members[0]).read())
    keeper = crate / "target/aarch64-unknown-linux-musl/release/devcroft-guest-keeper"
    build(out, "aarch64", zig, keeper=keeper, busybox=busybox, workload=args.workload, nix=args.nix,
          adaptable=args.adaptable, setup=args.setup_window, agents=args.agents,
          pid1_exit_probe=args.pid1_exit_probe, setup_fault_probe=args.setup_fault_probe)
    receipt = json.loads((out / "guest.json").read_text())
    sources = [*sorted((ROOT.parent.parent / "src/keeper").glob("*.rs")),
               ROOT.parent.parent / "src/ssh/server.rs", ROOT.parent.parent / "src/ssh/sftp.rs",
               *sorted((crate / "src").glob("*.rs")), crate / "Cargo.toml", crate / "Cargo.lock",
               crate / "zig-linker.sh", crate / "zig-ar.sh", ROOT / "build_keeper_guest.py",
               ROOT / "build_guest.py", ROOT / "guest/init.c"]
    sources.extend([ROOT / 'fixtures/ssh-consumption-windows.json', ROOT / 'freeze_ssh_sources.py', *sorted(expected)])
    if args.adaptable:
        sources.extend([ROOT / 'fixtures/adaptable-recipe-aarch64.json', ROOT / 'adaptable/recipe.py'])
    receipt["sources"] = {str(p.relative_to(ROOT.parent.parent)): hashlib.sha256(p.read_bytes()).hexdigest()
                          for p in sources}
    receipt["busybox_package"] = pin
    receipt["rustc"] = subprocess.check_output(["rustc", "--version"], text=True).strip()
    (out / "guest.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
