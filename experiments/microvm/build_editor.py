#!/usr/bin/env python3
"""Prepare pinned code-server for the Alpine fixture; no host project tools installed."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parent
REMOTE = "/home/dragos.guest/devcroft-editor"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze-inputs", action="store_true")
    parser.add_argument("--freeze-builder", action="store_true")
    parser.add_argument("--out", type=Path, default=ROOT / "artifacts/editor")
    args = parser.parse_args()
    out = args.out
    out.mkdir(parents=True, exist_ok=True)
    pin = json.loads((ROOT / "fixtures/editor-source.json").read_text())
    archive = ROOT / "artifacts/editor/code-server.tgz"
    archive.parent.mkdir(parents=True, exist_ok=True)
    if not archive.exists():
        subprocess.run(["curl", "--fail", "--location", "--max-time", "180", pin["url"], "-o", str(archive)], check=True)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != pin["sha256"]:
        raise RuntimeError("code-server source mismatch")
    pins = ROOT / "fixtures/editor-packages-aarch64.json"
    if not pins.exists() and not args.freeze_inputs:
        parser.error("initial closure resolution requires --freeze-inputs")
    builder_pins = ROOT / "fixtures/editor-builder-packages-aarch64.json"
    if not builder_pins.exists() and not args.freeze_builder:
        parser.error("initial builder closure resolution requires --freeze-builder")
    env = dict(os.environ, LIMA_HOME="/private/tmp/devcroft-kernel-builder")
    store = ROOT / "artifacts/apk-store"

    def lima(*args):
        subprocess.run(["limactl", *map(str, args)], env=env, check=True)

    try:
        lima("start", "--tty=false", "kernel-build")
        # Orchestration only: the npm/native build runs in a chroot of the pinned builder closure.
        lima("shell", "kernel-build", "sudo", "apk", "add", "--no-cache", "python3", "curl", "e2fsprogs", "coreutils",
             "util-linux-misc")
        lima("shell", "kernel-build", "mkdir", "-p", REMOTE)
        for file, name in [(ROOT / "editor/build.py", "build.py"), (ROOT / "fixtures/editor-source.json", "source.json"),
                           (archive, "code-server.tgz"), (ROOT / "artifacts/workload/toolchain.img", "base.img")]:
            lima("copy", file, f"kernel-build:{REMOTE}/{name}")
        if pins.exists() and not args.freeze_inputs:
            lima("copy", pins, f"kernel-build:{REMOTE}/packages.json")
        # Alpine branch mirrors keep only the newest revision of each package, so a hash pin to a
        # dl-cdn URL stops resolving once the package is updated. Every APK a build used is kept in a
        # host store and staged back in; build.py still checks each one against its pinned SHA-256.
        store.mkdir(parents=True, exist_ok=True)
        stored = sorted(store.glob("*.apk"))
        if stored:
            bundle = out / "apk-store.tar"
            with tarfile.open(bundle, "w") as tar:
                for apk in stored:
                    tar.add(apk, arcname=apk.name)
            lima("copy", bundle, f"kernel-build:{REMOTE}/apk-store.tar")
            bundle.unlink()
            for sub in ("apks", "builder-apks"):
                lima("shell", "kernel-build", "sh", "-c",
                     f"mkdir -p {REMOTE}/{sub} && tar -xf {REMOTE}/apk-store.tar -C {REMOTE}/{sub}")
        if builder_pins.exists() and not args.freeze_builder:
            lima("copy", builder_pins, f"kernel-build:{REMOTE}/builder-packages.json")
        flags = [*(["--freeze-inputs"] if args.freeze_inputs else []), *(["--freeze-builder"] if args.freeze_builder else [])]
        lima("shell", "kernel-build", "python3", f"{REMOTE}/build.py", *flags)
        for name in ("editor.img", "build.json", "packages.json", "builder-packages.json", "npm-cache.tar"):
            lima("copy", f"kernel-build:{REMOTE}/{name}", out / name)
        lima("shell", "kernel-build", "sh", "-c", f"cd {REMOTE} && tar -cf used-apks.tar apks builder-apks")
        lima("copy", f"kernel-build:{REMOTE}/used-apks.tar", out / "used-apks.tar")
        with tarfile.open(out / "used-apks.tar") as tar:
            for sub, manifest in (("apks", "packages.json"), ("builder-apks", "builder-packages.json")):
                for item in json.loads((out / manifest).read_text())["packages"]:
                    target = store / item["file"]
                    if not target.exists():
                        target.write_bytes(tar.extractfile(f"{sub}/{item['file']}").read())
                    if hashlib.sha256(target.read_bytes()).hexdigest() != item["sha256"]:
                        raise RuntimeError(f"stored APK mismatch: {item['file']}")
        (out / "used-apks.tar").unlink()
        if args.freeze_inputs:
            pins.write_text((out / "packages.json").read_text())
        if args.freeze_builder:
            builder_pins.write_text((out / "builder-packages.json").read_text())
        print((out / "build.json").read_text())
    finally:
        lima("stop", "kernel-build")


if __name__ == "__main__":
    main()
