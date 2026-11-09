#!/usr/bin/env python3
"""Prepare the exact upstream revision plus the reviewed patch, then build/sign.

The source checkout is an ignored artifact, never a change to a Cargo registry
cache. --from may seed it from an existing checkout containing the exact pin.
"""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import subprocess

ROOT = Path(__file__).resolve().parent
SOURCE = ROOT / "artifacts/hvi-source"
PATCH = ROOT / "patches/hvi-vsock-fixed-ports.patch"
REVISION = "351d1a15728e998486232e4815ba075a22e44f56"
URL = "https://github.com/brig-sh/hvi-vmm"
STAMP = ".devcroft-patches.json"


def git(*args, capture=False):
    command = ["git", "-C", str(SOURCE), *args]
    return subprocess.check_output(command) if capture else subprocess.run(command, check=True)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_source():
    stamp = json.loads((SOURCE / STAMP).read_text())
    if git("rev-parse", "HEAD", capture=True).decode().strip() != REVISION:
        raise ValueError("upstream checkout is not at the recorded revision")
    if stamp["patch_sha256"] != digest(PATCH):
        raise ValueError("patch changed; prepare a fresh source artifact explicitly")
    dirty = set(git("diff", "HEAD", "--name-only", capture=True).decode().splitlines())
    if dirty != set(stamp["files"]):
        raise ValueError("unexpected upstream modifications outside the recorded patch")
    if git("diff", "--cached", "--name-only", capture=True).strip():
        raise ValueError("unexpected staged changes in source artifact")
    actual = git("diff", "HEAD", "--binary", "--no-ext-diff", "--no-color",
                 "--src-prefix=a/", "--dst-prefix=b/", capture=True)
    # Object-id abbreviations depend on local Git settings. All patch content,
    # modes, paths and hunks must match the reviewed file byte-for-byte.
    normalize = lambda data: b"\n".join(line for line in data.splitlines()
                                       if not line.startswith(b"index "))
    if normalize(actual) != normalize(PATCH.read_bytes()):
        raise ValueError("effective upstream diff does not match the reviewed patch")
    for file, expected in stamp["files"].items():
        if digest(SOURCE / file) != expected:
            raise ValueError(f"patched source drift: {file}")
    return stamp


def prepare(seed):
    if not SOURCE.exists():
        SOURCE.parent.mkdir(parents=True, exist_ok=True)
        if seed:
            subprocess.run(["git", "clone", "--no-checkout", str(seed.resolve()), str(SOURCE)], check=True)
        else:
            SOURCE.mkdir()
            git("init")
            git("fetch", "--depth", "1", URL, REVISION)
        git("checkout", "--detach", REVISION)
        git("apply", "--check", str(PATCH))
        git("apply", str(PATCH))
        files = git("diff", "--name-only", capture=True).decode().splitlines()
        stamp = {"schema": 1, "revision": REVISION, "patch_sha256": digest(PATCH),
                 "files": {file: digest(SOURCE / file) for file in files}}
        (SOURCE / STAMP).write_text(json.dumps(stamp, indent=2) + "\n")
    return verify_source()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--from", dest="seed", type=Path)
    parser.add_argument("--prepare-only", action="store_true")
    args = parser.parse_args()
    stamp = prepare(args.seed)
    if not args.prepare_only:
        subprocess.run(["cargo", "build", "--manifest-path", str(ROOT / "hvi/Cargo.toml"), "--locked"], check=True)
        if platform.system() == "Darwin":
            subprocess.run(["codesign", "--force", "--sign", "-", "--entitlements",
                            str(ROOT / "artifacts/hvi-source/hvi.entitlements"),
                            str(ROOT / "hvi/target/debug/devcroft-hvi-spike")], check=True)
    print(json.dumps(stamp, indent=2))


if __name__ == "__main__":
    main()
