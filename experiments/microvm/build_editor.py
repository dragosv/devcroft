#!/usr/bin/env python3
"""Prepare pinned code-server for the Alpine fixture; no host project tools installed.

The Lima VM's image is digest-pinned and checked; on it, editor/bootstrap.sh checks the
few host components it executes against a pinned contract, then builds a SHA-256 pinned
orchestration root and runs editor/build.py inside it. Nothing is `apk add`ed to the VM.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parent
REMOTE = "/home/dragos.guest/devcroft-editor"
LIMA_HOME = Path("/private/tmp/devcroft-kernel-builder")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def lima_image():
    """The instance's base image, as Lima's own config pins it (location and digest)."""
    text = (LIMA_HOME / "kernel-build/lima.yaml").read_text()
    location = re.search(r"^\s*- location:\s*(\S+)", text, re.M)
    image_digest = re.search(r"^\s*digest:\s*(\S+)", text, re.M)
    if not location or not image_digest:
        raise RuntimeError("Lima instance config has no digest-pinned image")
    return {"location": location.group(1), "digest": image_digest.group(1)}


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
    tools_pins = ROOT / "fixtures/editor-tools-packages-aarch64.json"
    if not tools_pins.exists() and not args.freeze_tools:
        parser.error("initial orchestration closure resolution requires --freeze-tools")
    image = lima_image()
    if not args.freeze_tools and json.loads(tools_pins.read_text())["lima_image"] != image:
        raise RuntimeError(("Lima base image differs from the pinned one", image))
    env = dict(os.environ, LIMA_HOME=str(LIMA_HOME))
    store = ROOT / "artifacts/apk-store"

    def lima(*args):
        subprocess.run(["limactl", *map(str, args)], env=env, check=True)

    try:
        lima("start", "--tty=false", "kernel-build")
        lima("shell", "kernel-build", "mkdir", "-p", REMOTE)
        for file, name in [(ROOT / "editor/build.py", "build.py"), (ROOT / "editor/bootstrap.sh", "bootstrap.sh"),
                           (ROOT / "fixtures/editor-source.json", "source.json"),
                           (archive, "code-server.tgz"), (ROOT / "artifacts/workload/toolchain.img", "base.img")]:
            lima("copy", file, f"kernel-build:{REMOTE}/{name}")
        if pins.exists() and not args.freeze_inputs:
            lima("copy", pins, f"kernel-build:{REMOTE}/packages.json")
        if builder_pins.exists() and not args.freeze_builder:
            lima("copy", builder_pins, f"kernel-build:{REMOTE}/builder-packages.json")
        # Alpine branch mirrors keep only the newest revision of each package, so a hash pin to a
        # dl-cdn URL stops resolving once the package is updated. Every APK a build used is kept in a
        # host store and staged back in; each is still checked against its pinned SHA-256 on use.
        store.mkdir(parents=True, exist_ok=True)
        stored = sorted(store.glob("*.apk"))
        if stored:
            bundle = out / "apk-store.tar"
            with tarfile.open(bundle, "w") as tar:
                for apk in stored:
                    tar.add(apk, arcname=apk.name)
            lima("copy", bundle, f"kernel-build:{REMOTE}/apk-store.tar")
            bundle.unlink()
            for sub in ("apks", "builder-apks", "tools-apks"):
                lima("shell", "kernel-build", "sh", "-c",
                     f"mkdir -p {REMOTE}/{sub} && tar -xf {REMOTE}/apk-store.tar -C {REMOTE}/{sub}")
        if args.freeze_tools:
            # Resolution only, with the VM's own apk and python3: it produces pins and runs nothing they name.
            lima("shell", "kernel-build", "python3", f"{REMOTE}/build.py", "--freeze-tools")
            lima("copy", f"kernel-build:{REMOTE}/tools-packages.json", out / "tools-packages.json")
            tools = json.loads((out / "tools-packages.json").read_text())
        else:
            tools = json.loads(tools_pins.read_text())
            (out / "tools-packages.json").write_text(json.dumps(
                {k: tools[k] for k in ("schema", "bootstrap", "requested", "packages")}, indent=2) + "\n")
        everything = [*tools["bootstrap"], *tools["packages"]]
        lists = {
            "tools.sha256": "".join(f"{x['sha256']}  {x['file']}\n" for x in everything),
            "bootstrap.apk": tools["bootstrap"][0]["file"] + "\n",
            "keys.apk": tools["bootstrap"][1]["file"] + "\n",
            "tools.files": "".join(f"{x['file']}\n" for x in tools["packages"]),
            "tools.installed": "".join(f"{n}\n" for n in sorted(x["file"].removesuffix(".apk")
                                                                for x in tools["packages"])),
            "tools-packages.json": (out / "tools-packages.json").read_text(),
        }
        if not args.freeze_tools:
            lists["host.expected"] = "".join(f"{line}\n" for line in tools["host"])
        for name, text in lists.items():
            (out / name).write_text(text)
            lima("copy", out / name, f"kernel-build:{REMOTE}/{name}")
        flags = [*(["--freeze-tools"] if args.freeze_tools else []),
                 *(["--freeze-inputs"] if args.freeze_inputs else []),
                 *(["--freeze-builder"] if args.freeze_builder else [])]
        lima("shell", "kernel-build", "/bin/busybox", "sh", f"{REMOTE}/bootstrap.sh", *flags)
        for name in ("editor.img", "build.json", "packages.json", "builder-packages.json", "npm-cache.tar",
                     "host.actual"):
            lima("copy", f"kernel-build:{REMOTE}/{name}", out / name)
        lima("shell", "kernel-build", "sh", "-c", f"cd {REMOTE} && tar -cf used-apks.tar apks builder-apks tools-apks")
        lima("copy", f"kernel-build:{REMOTE}/used-apks.tar", out / "used-apks.tar")
        with tarfile.open(out / "used-apks.tar") as tar:
            for sub, items in (("apks", json.loads((out / "packages.json").read_text())["packages"]),
                               ("builder-apks", json.loads((out / "builder-packages.json").read_text())["packages"]),
                               ("tools-apks", everything)):
                for item in items:
                    target = store / item["file"]
                    if not target.exists():
                        target.write_bytes(tar.extractfile(f"{sub}/{item['file']}").read())
                    if digest(target) != item["sha256"]:
                        raise RuntimeError(f"stored APK mismatch: {item['file']}")
        (out / "used-apks.tar").unlink()
        receipt = json.loads((out / "build.json").read_text())
        receipt["lima_image"] = image
        (out / "build.json").write_text(json.dumps(receipt, indent=2) + "\n")
        if args.freeze_inputs:
            pins.write_text((out / "packages.json").read_text())
        if args.freeze_builder:
            builder_pins.write_text((out / "builder-packages.json").read_text())
        if args.freeze_tools:
            tools_pins.write_text(json.dumps(dict(tools, host=(out / "host.actual").read_text().split(),
                                                  lima_image=image), indent=2) + "\n")
        print((out / "build.json").read_text())
    finally:
        lima("stop", "kernel-build")


if __name__ == "__main__":
    main()
