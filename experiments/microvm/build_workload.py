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
    args = p.parse_args()
    out = ROOT / "artifacts/workload"
    out.mkdir(parents=True, exist_ok=True)
    source = json.loads((ROOT / "fixtures/workload-source.json").read_text())
    archive = out / "xxhash.tar.gz"
    if not archive.exists():
        with urllib.request.urlopen(source["url"], timeout=120) as response:
            archive.write_bytes(response.read(8 * 1024 * 1024 + 1))
    if hashlib.sha256(archive.read_bytes()).hexdigest() != source["sha256"]:
        raise RuntimeError("xxHash source identity mismatch")
    pins = ROOT / "fixtures/workload-packages-aarch64.json"
    if not pins.exists() and not args.freeze_inputs:
        p.error("initial package resolution requires --freeze-inputs")
    env = dict(os.environ, LIMA_HOME="/private/tmp/devcroft-kernel-builder")

    def lima(*args):
        subprocess.run(["limactl", *map(str, args)], env=env, check=True)

    try:
        lima("start", "--tty=false", "kernel-build")
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
             *(["--freeze-inputs"] if args.freeze_inputs else []))
        for name in ("toolchain.img", "data.seed.img", "packages.tar", "build.json", "packages.json"):
            lima("copy", f"kernel-build:/tmp/devcroft-workload/{name}", out / name)
        if args.freeze_inputs:
            pins.write_text((out / "packages.json").read_text())
        print((out / "build.json").read_text())
    finally:
        lima("stop", "kernel-build")


if __name__ == "__main__":
    main()
