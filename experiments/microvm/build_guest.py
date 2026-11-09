#!/usr/bin/env python3
"""Build an unprivileged, deterministic Stage-A initramfs using an installed Zig.

The kernel is a separate pinned input. No host project tooling is copied in.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess

ROOT = Path(__file__).resolve().parent


def entry(out, name, data=b"", mode=stat.S_IFDIR | 0o755, ino=1, major=0, minor=0):
    encoded = name.encode() + b"\0"
    fields = [ino, mode, 0, 0, 1, 0, len(data), 0, 0, major, minor, len(encoded), 0]
    out.write(b"070701" + b"".join(f"{x:08x}".encode() for x in fields))
    out.write(encoded)
    out.write(b"\0" * (-out.tell() % 4))
    out.write(data)
    out.write(b"\0" * (-out.tell() % 4))


def build(out_dir, arch, zig, keeper=None, busybox=None, workload=False, nix=False):
    if nix and not workload:
        raise ValueError('Nix fixture requires the workload block devices')
    out_dir.mkdir(parents=True, exist_ok=True)
    init = out_dir / "init"
    env = dict(os.environ, ZIG_GLOBAL_CACHE_DIR=str(ROOT / ".zig-cache"))
    command = [zig, "cc", "-target", f"{arch}-linux-musl", "-static", "-O2",
               "-Wall", "-Wextra", "-Werror", str(ROOT / "guest/init.c"), "-o", str(init)]
    if keeper:
        command.append("-DENABLE_KEEPER=1")
    if workload:
        command.append("-DENABLE_WORKLOAD=1")
    subprocess.run(command, check=True, env=env)
    archive = io.BytesIO()
    for ino, name in enumerate(["dev", "proc", "sys", "run", "tmp"], 1):
        entry(archive, name, ino=ino)
    entry(archive, "dev/console", mode=stat.S_IFCHR | 0o600, ino=6, major=5, minor=1)
    entry(archive, "init", init.read_bytes(), stat.S_IFREG | 0o755, ino=7)
    if keeper:
        for ino, name in enumerate(["bin", "workspace", "home", "home/devcroft", "private"], 8):
            entry(archive, name, ino=ino)
        entry(archive, "guest-keeper", keeper.read_bytes(), stat.S_IFREG | 0o755, ino=13)
        entry(archive, "bin/busybox", busybox.read_bytes(), stat.S_IFREG | 0o755, ino=14)
        for ino, name in enumerate(["sh", "cat", "sleep", "printf", "stty", "env", "test", "wc", "head", "kill", "mkdir", "rm", "nc", "cp", "find", "sha256sum", "sort", "xargs", "grep", "cmp", "tr", "sync", "uname", "sed", "awk", "ln", "mv", "install", "tar", "cut", "tail"], 50):
            entry(archive, f"bin/{name}", b"busybox", stat.S_IFLNK | 0o777, ino=ino)
        entry(archive, "private/credential", b"guest-private-sentinel\n", stat.S_IFREG | 0o600, ino=30)
        entry(archive, "bin/readonly-data", b"runtime-read-control\n", stat.S_IFREG | 0o644, ino=32)
        if workload:
            entry(archive, "toolchain", ino=100)
            for ino, name in enumerate(["usr", "lib", "etc"], 101):
                entry(archive, name, f"toolchain/{name}".encode(), stat.S_IFLNK | 0o777, ino=ino)
            if nix:
                entry(archive, "nix", b"toolchain/nix", stat.S_IFLNK | 0o777, ino=105)
    entry(archive, "TRAILER!!!", mode=0, ino=31)
    path = out_dir / "initramfs.cpio"
    path.write_bytes(archive.getvalue())
    runtime = out_dir / "runtime.img"
    with runtime.open("wb") as f:
        f.write(b"DEVCROFT_RUNTIME_V1\0")
        f.truncate(16 * 1024 * 1024)
    result = {"schema": 1, "arch": arch, "fixture": "stage-a-raw-disks",
              "zig_version": subprocess.check_output([zig, "version"], text=True).strip(),
              "source_sha256": hashlib.sha256((ROOT / "guest/init.c").read_bytes()).hexdigest()}
    if keeper:
        result.update(fixture="stage-b-real-keeper", nono="0.77.0",
                      keeper_sha256=hashlib.sha256(keeper.read_bytes()).hexdigest(),
                      busybox_sha256=hashlib.sha256(busybox.read_bytes()).hexdigest())
    if workload:
        result.update(fixture="stage-b-xxhash-redis-ext4", workload=True)
    if nix:
        result.update(fixture="stage-b-nix-closure-ext4", nix=True)
    for key, file in [("initramfs", path), ("runtime", runtime)]:
        result[key] = {"path": str(file.resolve()), "sha256": hashlib.sha256(file.read_bytes()).hexdigest()}
    (out_dir / "guest.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=ROOT / "artifacts/stage-a")
    parser.add_argument("--arch", choices=["aarch64", "x86_64"], default="aarch64")
    parser.add_argument("--zig", default=shutil.which("zig"))
    args = parser.parse_args()
    if not args.zig:
        parser.error("an installed Zig compiler is required")
    build(args.out.resolve(), args.arch, args.zig)
