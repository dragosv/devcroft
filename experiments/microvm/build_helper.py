#!/usr/bin/env python3
"""Prepare the exact upstream revision plus reviewed patches, then build/sign.

The source checkout is an ignored artifact, never a change to a Cargo registry
cache. --from may seed it from an existing checkout containing the exact pin.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent
SOURCE = ROOT / "artifacts/libkrun-source"
PATCH = ROOT / "patches/libkrun-vsock-host-half-close.patch"
RESET_PATCH = ROOT / "patches/libkrun-macos-reset-exit.patch"
BACKPRESSURE_PATCH = ROOT / "patches/libkrun-vsock-host-backpressure.patch"
PATCHES = (PATCH, RESET_PATCH, BACKPRESSURE_PATCH)
REVISION = "6151710f4ea627cf0e5dbce78e4192fb4d64c2e9"
URL = "https://github.com/containers/libkrun"
STAMP = ".devcroft-patches.json"


def git(*args, capture=False, env=None):
    command = ["git", "-C", str(SOURCE), *args]
    if capture:
        return subprocess.check_output(command, env=env)
    return subprocess.run(command, check=True, env=env)


def expected_diff():
    # Patches may touch the same file, so their concatenation is not a diff of
    # the result. Apply them in order to a throwaway index built from HEAD.
    with tempfile.TemporaryDirectory() as tmp:
        env = {**os.environ, "GIT_INDEX_FILE": str(Path(tmp) / "index")}
        git("read-tree", "HEAD", env=env)
        for patch in PATCHES:
            git("apply", "--cached", str(patch), env=env)
        return git("diff", "--cached", "HEAD", "--binary", "--no-ext-diff", "--no-color",
                   "--src-prefix=a/", "--dst-prefix=b/", capture=True, env=env)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_source():
    stamp = json.loads((SOURCE / STAMP).read_text())
    if git("rev-parse", "HEAD", capture=True).decode().strip() != REVISION:
        raise ValueError("upstream checkout is not at the recorded revision")
    expected_patches = [{"path": str(p.relative_to(ROOT)), "sha256": digest(p)} for p in PATCHES]
    if stamp.get("patches") != expected_patches:
        raise ValueError("patch series changed; prepare a fresh source artifact explicitly")
    dirty = set(git("diff", "HEAD", "--name-only", capture=True).decode().splitlines())
    if dirty != set(stamp["files"]):
        raise ValueError("unexpected upstream modifications outside the recorded patch")
    if git("diff", "--cached", "--name-only", capture=True).strip():
        raise ValueError("unexpected staged changes in source artifact")
    actual = git("diff", "HEAD", "--binary", "--no-ext-diff", "--no-color",
                 "--src-prefix=a/", "--dst-prefix=b/", capture=True)
    # Object-id abbreviations depend on local Git settings. All content,
    # modes, paths and hunks must match the reviewed series byte-for-byte.
    normalize = lambda data: b"\n".join(line for line in data.splitlines()
                                       if not line.startswith(b"index "))
    if normalize(actual) != normalize(expected_diff()):
        raise ValueError("effective upstream diff does not match the reviewed patch series")
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
        for patch in PATCHES:
            git("apply", "--check", str(patch))
            git("apply", str(patch))
        files = git("diff", "--name-only", capture=True).decode().splitlines()
        stamp = {"schema": 2, "revision": REVISION, "patch_sha256": digest(PATCH),
                 "patches": [{"path": str(p.relative_to(ROOT)), "sha256": digest(p)} for p in PATCHES],
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
        subprocess.run(["cargo", "build", "--manifest-path", str(ROOT / "libkrun/Cargo.toml"), "--locked"], check=True)
        if platform.system() == "Darwin":
            subprocess.run(["codesign", "--force", "--sign", "-", "--entitlements",
                            str(ROOT / "libkrun/entitlements.plist"),
                            str(ROOT / "libkrun/target/debug/devcroft-krun-spike")], check=True)
    print(json.dumps(stamp, indent=2))


if __name__ == "__main__":
    main()
