#!/usr/bin/env python3
"""Linux-only offline materialization after every APK input has been hash-pinned."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile


def run(*args):
    subprocess.run(list(map(str, args)), check=True, timeout=300)


def digest(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--freeze-inputs", action="store_true")
    args = p.parse_args()
    work = Path("/tmp/devcroft-workload")
    work.mkdir(exist_ok=True)
    os.chdir(work)
    apks = work / "apks"
    apks.mkdir(exist_ok=True)
    pin_path = work / "packages.json"
    if args.freeze_inputs:
        urls = subprocess.check_output(["apk", "fetch", "--recursive", "--url",
            "gcc", "musl-dev", "make", "redis", "alpine-baselayout"], text=True).splitlines()
        records = []
        for url in sorted(set(urls)):
            if not url.startswith("https://dl-cdn.alpinelinux.org/alpine/v3.23/"):
                raise RuntimeError(f"unexpected package source: {url}")
            file = apks / url.rsplit("/", 1)[1]
            run("curl", "--fail", "--location", "--max-time", "120", url, "-o", file)
            records.append({"url": url, "file": file.name, "sha256": digest(file), "bytes": file.stat().st_size})
        pin_path.write_text(json.dumps({"schema": 1, "architecture": "aarch64", "packages": records}, indent=2) + "\n")
    pins = json.loads(pin_path.read_text())
    for item in pins["packages"]:
        file = apks / item["file"]
        if not file.exists():
            run("curl", "--fail", "--location", "--max-time", "120", item["url"], "-o", file)
        if digest(file) != item["sha256"]:
            raise RuntimeError(f"package hash mismatch: {file.name}")
    source = json.loads((work / "source.json").read_text())
    archive = work / "xxhash.tar.gz"
    if digest(archive) != source["sha256"]:
        raise RuntimeError("source hash mismatch")
    root = work / "root"
    root.mkdir(exist_ok=False)
    # All dependency resolution has already happened. This stage has no network.
    run("sudo", "apk", "--root", root, "--initdb", "--no-scripts", "--no-network", "--force-non-repository",
        "--keys-dir", "/etc/apk/keys", "add", *[apks / x["file"] for x in pins["packages"]])
    run("sudo", "chown", "-R", f"{os.getuid()}:{os.getgid()}", root)
    fixture = root / "fixture"
    (fixture / "xxHash").mkdir(parents=True)
    run("tar", "-xzf", archive, "--strip-components=1", "-C", fixture / "xxHash")
    small = fixture / "small"
    small.mkdir()
    checksums = []
    for n in range(source["small_files"]):
        name = f"d{n % 32:02d}/file-{n:04d}.txt"
        file = small / name
        file.parent.mkdir(exist_ok=True)
        file.write_text(f"microvm-small-file-v1 {n}\n" + hashlib.sha256(str(n).encode()).hexdigest() * 4 + "\n")
        checksums.append(f"{digest(file)}  {name}\n")
    (small / "SHA256SUMS").write_text("".join(checksums))
    run("truncate", "-s", "512M", work / "toolchain.img")
    run("mkfs.ext4", "-F", "-q", "-m", "0", "-O", "^orphan_file",
        "-U", "d31c0001-0000-4000-8000-000000000001", "-d", root, work / "toolchain.img")
    run("truncate", "-s", "128M", work / "data.seed.img")
    run("mkfs.ext4", "-F", "-q", "-m", "0", "-O", "^orphan_file",
        "-U", "d31c0002-0000-4000-8000-000000000002", work / "data.seed.img")
    with tarfile.open(work / "packages.tar", "w") as out:
        for file in sorted(apks.glob("*.apk")):
            out.add(file, arcname=file.name)
    metadata = {"schema": 1, "architecture": "aarch64", "source": source,
                "packages": pins, "build_network": "apk install uses --no-network after complete hash verification",
                "filesystem": "ext4; immutable toolchain plus per-instance writable workspace",
                "reproducibility": "reconstruction recipe and exact source/APK inputs; not a bit-reproducible filesystem claim"}
    for name in ("toolchain.img", "data.seed.img", "packages.tar"):
        metadata[name] = {"sha256": digest(work / name), "bytes": (work / name).stat().st_size}
    metadata["builder_packages"] = subprocess.check_output(["apk", "info", "-v"], text=True).splitlines()
    (work / "build.json").write_text(json.dumps(metadata, indent=2) + "\n")


if __name__ == "__main__":
    main()
