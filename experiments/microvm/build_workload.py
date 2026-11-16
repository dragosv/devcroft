#!/usr/bin/env python3
"""Prepare the real build/Redis fixture in the existing mount-free Linux builder.

workload/build.py runs inside the pinned orchestration root (see orchestration/__init__.py);
nothing is `apk add`ed to the Lima VM.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile
import urllib.request

import orchestration

ROOT = Path(__file__).resolve().parent
REMOTE = "/tmp/devcroft-workload"


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--freeze-inputs", action="store_true", help="explicitly freeze a new Alpine APK closure")
    p.add_argument("--pins", type=Path, default=ROOT / "fixtures/workload-packages-aarch64.json")
    p.add_argument("--out", type=Path, default=ROOT / "artifacts/workload")
    p.add_argument("--extra-package", action="append", default=[])
    p.add_argument("--merge-bin-into-usr", action="store_true")
    p.add_argument("--compare-with", type=Path,
                   help="an earlier output directory whose toolchain/seed image trees the new ones must match")
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
    image = orchestration.check_inputs(False)
    env = dict(os.environ, LIMA_HOME=str(orchestration.LIMA_HOME))

    def lima(*args):
        subprocess.run(["limactl", *map(str, args)], env=env, check=True)

    try:
        if (Path(env["LIMA_HOME"]) / "kernel-build/lima.yaml").exists():
            lima("start", "--tty=false", "kernel-build")
        else:
            lima("start", "--tty=false", "--name=kernel-build", ROOT / "kernel/builder.yaml")
        bb = "/bin/busybox"
        lima("shell", "kernel-build", "sudo", bb, "mkdir", "-p", REMOTE)
        # A dedicated tmpfs bounds disposable preparation and avoids inflating
        # the builder disk with toolchain/source copies.
        lima("shell", "kernel-build", "sudo", bb, "mount", "-t", "tmpfs", "-o", "size=3G", "tmpfs", REMOTE)
        lima("shell", "kernel-build", "sudo", bb, "chmod", "0777", REMOTE)
        files = [(ROOT / "workload/build.py", "build.py"), (ROOT / "fixtures/workload-source.json", "source.json"),
                 (archive, "xxhash.tar.gz")]
        compare = []
        if args.compare_with:
            for name in ("toolchain.img", "data.seed.img"):
                files.append((args.compare_with / name, f"reference-{name}"))
                compare += ["--compare-with", f"{name}=reference-{name}"]
        for file, name in files:
            lima("copy", file, f"kernel-build:{REMOTE}/{name}")
        if pins.exists() and not args.freeze_inputs:
            lima("copy", pins, f"kernel-build:{REMOTE}/packages.json")
        wanted = {x["file"] for x in json.loads(pins.read_text())["packages"]} if pins.exists() else set()
        tools_pins = json.loads(orchestration.PINS.read_text())
        wanted |= {x["file"] for x in [*tools_pins["bootstrap"], *tools_pins["packages"]]}
        orchestration.stage_store(lima, REMOTE, out, ("apks", "tools-apks"), wanted)
        tools, _ = orchestration.run(lima, REMOTE, out, "build.py", [
            *(["--freeze-inputs"] if args.freeze_inputs else []),
            *(f for name in args.extra_package for f in ("--extra-package", name)),
            *(["--merge-bin-into-usr"] if args.merge_bin_into_usr else []), *compare], False)
        for name in ("toolchain.img", "data.seed.img", "packages.tar", "build.json", "packages.json"):
            lima("copy", f"kernel-build:{REMOTE}/{name}", out / name)
        lima("shell", "kernel-build", bb, "sh", "-c", f"cd {REMOTE} && {bb} tar -cf used-apks.tar tools-apks")
        lima("copy", f"kernel-build:{REMOTE}/used-apks.tar", out / "used-apks.tar")
        with tarfile.open(out / "used-apks.tar") as tar:
            orchestration.keep_tools(tar, tools)
        (out / "used-apks.tar").unlink()
        with tarfile.open(out / "packages.tar") as tar:
            orchestration.keep_in_store(tar, [(x["file"], x)
                                              for x in json.loads((out / "packages.json").read_text())["packages"]])
        receipt = json.loads((out / "build.json").read_text())
        receipt["lima_image"] = image
        (out / "build.json").write_text(json.dumps(receipt, indent=2) + "\n")
        if args.freeze_inputs:
            pins.write_text((out / "packages.json").read_text())
        print((out / "build.json").read_text())
    finally:
        lima("stop", "kernel-build")


if __name__ == "__main__":
    main()
