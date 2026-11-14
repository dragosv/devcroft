#!/usr/bin/env python3
"""Prepare the real build/Redis fixture in the existing mount-free Linux builder."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import urllib.request

ROOT = Path(__file__).resolve().parent


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--freeze-inputs", action="store_true", help="explicitly freeze a new Alpine APK closure")
    p.add_argument("--pins", type=Path, default=ROOT / "fixtures/workload-packages-aarch64.json")
    p.add_argument("--out", type=Path, default=ROOT / "artifacts/workload")
    p.add_argument("--extra-package", action="append", default=[])
    p.add_argument("--merge-bin-into-usr", action="store_true")
    args = p.parse_args()
    if (args.extra_package or args.merge_bin_into_usr) and args.pins == ROOT / "fixtures/workload-packages-aarch64.json":
        p.error("extra packages need their own --pins file; the measured workload pin set is unchanged")
    out = args.out
    out.mkdir(parents=True, exist_ok=True)
    source = json.loads((ROOT / "fixtures/workload-source.json").read_text())
    archive = out / "xxhash.tar.gz"
    if not archive.exists():
        with urllib.request.urlopen(source["url"], timeout=120) as response:
            archive.write_bytes(response.read(8 * 1024 * 1024 + 1))
    if hashlib.sha256(archive.read_bytes()).hexdigest() != source["sha256"]:
        raise RuntimeError("xxHash source identity mismatch")
    pins = args.pins
    if not pins.exists() and not args.freeze_inputs:
        p.error("initial package resolution requires --freeze-inputs")
    env = dict(os.environ, LIMA_HOME="/private/tmp/devcroft-kernel-builder")

    def lima(*args):
        subprocess.run(["limactl", *map(str, args)], env=env, check=True)

    try:
        if (Path(env["LIMA_HOME"]) / "kernel-build/lima.yaml").exists():
            lima("start", "--tty=false", "kernel-build")
        else:
            lima("start", "--tty=false", "--name=kernel-build", ROOT / "kernel/builder.yaml")
        lima("shell", "kernel-build", "sudo", "apk", "add", "--no-cache", "python3", "curl", "e2fsprogs")
        lima("shell", "kernel-build", "sudo", "mkdir", "-p", "/tmp/devcroft-workload")
        # A dedicated tmpfs bounds disposable preparation and avoids inflating
        # the builder disk with toolchain/source copies.
        lima("shell", "kernel-build", "sudo", "mount", "-t", "tmpfs", "-o", "size=2G", "tmpfs", "/tmp/devcroft-workload")
        lima("shell", "kernel-build", "sudo", "chmod", "0777", "/tmp/devcroft-workload")
        for file, name in [(ROOT / "workload/build.py", "build.py"),
                           (ROOT / "fixtures/workload-source.json", "source.json"), (archive, "xxhash.tar.gz")]:
            lima("copy", file, f"kernel-build:/tmp/devcroft-workload/{name}")
        if pins.exists() and not args.freeze_inputs:
            lima("copy", pins, "kernel-build:/tmp/devcroft-workload/packages.json")
        lima("shell", "kernel-build", "python3", "/tmp/devcroft-workload/build.py",
             *(["--freeze-inputs"] if args.freeze_inputs else []),
             *(f for name in args.extra_package for f in ("--extra-package", name)),
             *(["--merge-bin-into-usr"] if args.merge_bin_into_usr else []))
        for name in ("toolchain.img", "data.seed.img", "packages.tar", "build.json", "packages.json"):
            lima("copy", f"kernel-build:/tmp/devcroft-workload/{name}", out / name)
        if args.freeze_inputs:
            pins.write_text((out / "packages.json").read_text())
        print((out / "build.json").read_text())
    finally:
        lima("stop", "kernel-build")


if __name__ == "__main__":
    main()
