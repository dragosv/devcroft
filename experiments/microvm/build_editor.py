#!/usr/bin/env python3
"""Prepare pinned code-server for the Alpine fixture; no host project tools installed.

editor/build.py runs inside the pinned orchestration root (see orchestration/__init__.py);
nothing is `apk add`ed to the Lima VM.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tarfile

import orchestration
from orchestration import digest

ROOT = Path(__file__).resolve().parent
REMOTE = "/home/dragos.guest/devcroft-editor"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze-inputs", action="store_true")
    parser.add_argument("--freeze-builder", action="store_true")
    parser.add_argument("--freeze-tools", action="store_true",
                        help="repin the orchestration closure and record the Lima host contract")
    parser.add_argument("--out", type=Path, default=ROOT / "artifacts/editor")
    args = parser.parse_args()
    out = args.out
    out.mkdir(parents=True, exist_ok=True)
    pin = json.loads((ROOT / "fixtures/editor-source.json").read_text())
    archive = ROOT / "artifacts/editor/code-server.tgz"
    archive.parent.mkdir(parents=True, exist_ok=True)
    if not archive.exists():
        subprocess.run(["curl", "--fail", "--location", "--max-time", "180", pin["url"], "-o", str(archive)], check=True)
    if digest(archive) != pin["sha256"]:
        raise RuntimeError("code-server source mismatch")
    pins = ROOT / "fixtures/editor-packages-aarch64.json"
    if not pins.exists() and not args.freeze_inputs:
        parser.error("initial closure resolution requires --freeze-inputs")
    builder_pins = ROOT / "fixtures/editor-builder-packages-aarch64.json"
    if not builder_pins.exists() and not args.freeze_builder:
        parser.error("initial builder closure resolution requires --freeze-builder")
    image = orchestration.check_inputs(args.freeze_tools)
    env = dict(os.environ, LIMA_HOME=str(orchestration.LIMA_HOME))

    def lima(*args):
        subprocess.run(["limactl", *map(str, args)], env=env, check=True)

    try:
        lima("start", "--tty=false", "kernel-build")
        lima("shell", "kernel-build", "/bin/busybox", "mkdir", "-p", REMOTE)
        for file, name in [(ROOT / "editor/build.py", "build.py"), (ROOT / "fixtures/editor-source.json", "source.json"),
                           (archive, "code-server.tgz"), (ROOT / "artifacts/workload/toolchain.img", "base.img")]:
            lima("copy", file, f"kernel-build:{REMOTE}/{name}")
        if pins.exists() and not args.freeze_inputs:
            lima("copy", pins, f"kernel-build:{REMOTE}/packages.json")
        if builder_pins.exists() and not args.freeze_builder:
            lima("copy", builder_pins, f"kernel-build:{REMOTE}/builder-packages.json")
        orchestration.stage_store(lima, REMOTE, out, ("apks", "builder-apks", "tools-apks"))
        flags = [*(["--freeze-inputs"] if args.freeze_inputs else []),
                 *(["--freeze-builder"] if args.freeze_builder else [])]
        tools, host = orchestration.run(lima, REMOTE, out, "build.py", flags, args.freeze_tools)
        for name in ("editor.img", "build.json", "packages.json", "builder-packages.json", "npm-cache.tar"):
            lima("copy", f"kernel-build:{REMOTE}/{name}", out / name)
        lima("shell", "kernel-build", "/bin/busybox", "sh", "-c",
             f"cd {REMOTE} && /bin/busybox tar -cf used-apks.tar apks builder-apks tools-apks")
        lima("copy", f"kernel-build:{REMOTE}/used-apks.tar", out / "used-apks.tar")
        with tarfile.open(out / "used-apks.tar") as tar:
            for sub, manifest in (("apks", "packages.json"), ("builder-apks", "builder-packages.json")):
                orchestration.keep_in_store(tar, [(f"{sub}/{x['file']}", x)
                                                  for x in json.loads((out / manifest).read_text())["packages"]])
            orchestration.keep_tools(tar, tools)
        (out / "used-apks.tar").unlink()
        receipt = json.loads((out / "build.json").read_text())
        receipt["lima_image"] = image
        (out / "build.json").write_text(json.dumps(receipt, indent=2) + "\n")
        if args.freeze_inputs:
            pins.write_text((out / "packages.json").read_text())
        if args.freeze_builder:
            builder_pins.write_text((out / "builder-packages.json").read_text())
        if args.freeze_tools:
            orchestration.write_pins(tools, host, image)
        print((out / "build.json").read_text())
    finally:
        lima("stop", "kernel-build")


if __name__ == "__main__":
    main()
