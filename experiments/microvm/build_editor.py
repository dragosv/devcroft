#!/usr/bin/env python3
"""Prepare pinned code-server for the Alpine fixture; no host project tools installed."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent
REMOTE = "/home/dragos.guest/devcroft-editor"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze-inputs", action="store_true")
    args = parser.parse_args()
    out = ROOT / "artifacts/editor"
    out.mkdir(parents=True, exist_ok=True)
    pin = json.loads((ROOT / "fixtures/editor-source.json").read_text())
    archive = out / "code-server.tgz"
    if not archive.exists():
        subprocess.run(["curl", "--fail", "--location", "--max-time", "180", pin["url"], "-o", str(archive)], check=True)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != pin["sha256"]:
        raise RuntimeError("code-server source mismatch")
    pins = ROOT / "fixtures/editor-packages-aarch64.json"
    if not pins.exists() and not args.freeze_inputs:
        parser.error("initial closure resolution requires --freeze-inputs")
    env = dict(os.environ, LIMA_HOME="/private/tmp/devcroft-kernel-builder")

    def lima(*args):
        subprocess.run(["limactl", *map(str, args)], env=env, check=True)

    try:
        lima("start", "--tty=false", "kernel-build")
        lima("shell", "kernel-build", "sudo", "apk", "add", "--no-cache", "nodejs", "nodejs-dev", "npm", "g++",
             "python3", "pkgconf", "bash", "openssl-dev", "krb5-dev", "libx11-dev", "libxkbfile-dev", "libsecret-dev", "e2fsprogs")
        lima("shell", "kernel-build", "mkdir", "-p", REMOTE)
        for file, name in [(ROOT / "editor/build.py", "build.py"), (ROOT / "fixtures/editor-source.json", "source.json"),
                           (archive, "code-server.tgz"), (ROOT / "artifacts/workload/toolchain.img", "base.img")]:
            lima("copy", file, f"kernel-build:{REMOTE}/{name}")
        if pins.exists() and not args.freeze_inputs:
            lima("copy", pins, f"kernel-build:{REMOTE}/packages.json")
        lima("shell", "kernel-build", "python3", f"{REMOTE}/build.py", *(["--freeze-inputs"] if args.freeze_inputs else []))
        for name in ("editor.img", "build.json", "packages.json", "npm-cache.tar"):
            lima("copy", f"kernel-build:{REMOTE}/{name}", out / name)
        if args.freeze_inputs:
            pins.write_text((out / "packages.json").read_text())
        print((out / "build.json").read_text())
    finally:
        lima("stop", "kernel-build")


if __name__ == "__main__":
    main()
