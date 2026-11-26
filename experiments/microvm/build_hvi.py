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
SOURCE = ROOT / "artifacts/hvi-source"
PATCH = ROOT / "patches/hvi-vsock-fixed-ports.patch"
PENDING_PATCH = ROOT / "patches/hvi-vsock-pending-bound.patch"
PATCHES = (PATCH, PENDING_PATCH)
REVISION = "351d1a15728e998486232e4815ba075a22e44f56"
URL = "https://github.com/brig-sh/hvi-vmm"
STAMP = ".devcroft-patches.json"


def git(*args, capture=False, env=None):
    command = ["git", "-C", str(SOURCE), *args]
    if capture:
        return subprocess.check_output(command, env=env)
    return subprocess.run(command, check=True, env=env)


def expected_diff():
    # Patches touch the same file, so compare against the applied series.
    with tempfile.TemporaryDirectory() as tmp:
        env = {**os.environ, "GIT_INDEX_FILE": str(Path(tmp) / "index")}
        git("read-tree", "HEAD", env=env)
        for patch in PATCHES:
            git("apply", "--cached", str(patch), env=env)
        return git("diff", "--cached", "HEAD", "--binary", "--no-ext-diff", "--no-color",
                   "--src-prefix=a/", "--dst-prefix=b/", capture=True, env=env)


def patch_records():
    return [{"path": str(p.relative_to(ROOT)), "sha256": digest(p)} for p in PATCHES]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_source():
    stamp = json.loads((SOURCE / STAMP).read_text())
    head = git("rev-parse", "HEAD", capture=True).decode().strip()
    if stamp.get("revision") != REVISION or head != REVISION:
        raise ValueError("upstream checkout is not at the recorded revision")
    patches = patch_records()
    if (stamp.get("schema") != 2 or stamp.get("patches") != patches
            or stamp.get("patch_sha256") != patches[0]["sha256"]):
        raise ValueError("patch series changed; prepare a fresh source artifact explicitly")
    dirty = set(git("diff", "HEAD", "--name-only", capture=True).decode().splitlines())
    if dirty != set(stamp["files"]):
        raise ValueError("unexpected upstream modifications outside the recorded patch")
    if git("diff", "--cached", "--name-only", capture=True).strip():
        raise ValueError("unexpected staged changes in source artifact")
    actual = git("diff", "HEAD", "--binary", "--no-ext-diff", "--no-color",
                 "--src-prefix=a/", "--dst-prefix=b/", capture=True)
    # Object-id abbreviations depend on local Git settings. All patch content,
    # modes, paths and hunks must match the applied patch series byte-for-byte.
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
        patches = patch_records()
        stamp = {"schema": 2, "revision": REVISION, "patch_sha256": patches[0]["sha256"],
                 "patches": patches,
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
