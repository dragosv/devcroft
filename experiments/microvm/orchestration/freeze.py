#!/usr/bin/env python3
"""On the Lima system: resolve the bootstrap packages and the orchestration closure.

Resolution only, with the VM's own apk and python3: it writes pins and fetches the APKs
they name into WORKDIR/tools-apks, and executes none of them.
"""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ALPINE = "https://dl-cdn.alpinelinux.org/alpine/v3.23/"
BOOTSTRAP_PACKAGES = ["apk-tools-static", "alpine-keys"]
# Everything that prepares an image: this interpreter, apk, mount, e2fsprogs, chroot, unshare, tar.
TOOLS_PACKAGES = ["busybox", "python3", "apk-tools", "alpine-keys", "ca-certificates", "coreutils", "tar", "mount",
                  "umount", "util-linux-misc", "e2fsprogs", "e2fsprogs-extra"]


def resolve(names, dest, work):
    # apk fetch --url omits archives already present in its output dir, so resolve into a fresh one.
    with tempfile.TemporaryDirectory(prefix="resolve-", dir=work) as fresh:
        urls = subprocess.check_output(["apk", "fetch", "--recursive", "--url", "--output", fresh, *names],
                                       text=True).splitlines()
        packages = []
        for url in sorted(set(urls)):
            if not url.startswith(ALPINE):
                raise RuntimeError("unexpected APK origin")
            file = dest / url.rsplit("/", 1)[1]
            shutil.copyfile(Path(fresh) / file.name, file)
            packages.append({"url": url, "file": file.name, "sha256": hashlib.sha256(file.read_bytes()).hexdigest()})
    if not packages:
        raise RuntimeError(f"APK resolution returned an empty closure for {names}")
    return packages


def main():
    work = Path(sys.argv[1])
    out = work / "tools-apks"
    out.mkdir(parents=True, exist_ok=True)
    bootstrap = [resolve([name], out, work)[0] for name in BOOTSTRAP_PACKAGES]
    if [x["file"].rsplit("-", 2)[0] for x in bootstrap] != BOOTSTRAP_PACKAGES:
        raise RuntimeError(("bootstrap packages pulled dependencies", bootstrap))
    tools = resolve(TOOLS_PACKAGES, out, work)
    (work / "tools-packages.json").write_text(json.dumps(
        {"schema": 1, "bootstrap": bootstrap, "requested": TOOLS_PACKAGES, "packages": tools}, indent=2) + "\n")


if __name__ == "__main__":
    main()
