#!/usr/bin/env python3
"""Linux-only offline materialization after every APK input has been hash-pinned.

Runs as root inside the pinned orchestration root that orchestration/bootstrap.sh builds, with
the work directory bound at /work; nothing here comes from the Lima VM's own packages.
"""
import argparse
import difflib
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import tarfile
import urllib.request


def run(*args):
    subprocess.run(list(map(str, args)), check=True, timeout=300)


def digest(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def owner():
    return f"{os.environ['BUILD_UID']}:{os.environ['BUILD_GID']}"


def fetch(url, file):
    with urllib.request.urlopen(url, timeout=120) as response:
        file.write_bytes(response.read())


def tree(top):
    """Every entry under top: type, mode, owner, and content digest or link target; times excluded."""
    entries = {}
    for directory, dirs, files in os.walk(top):
        dirs.sort()
        for name in [*dirs, *sorted(files)]:
            path = Path(directory, name)
            st = path.lstat()
            if stat.S_ISLNK(st.st_mode):
                content = "-> " + os.readlink(path)
            elif stat.S_ISREG(st.st_mode):
                content = digest(path)
            else:
                content = ""
            entries[str(path.relative_to(top))] = (stat.S_IFMT(st.st_mode), stat.S_IMODE(st.st_mode),
                                                   st.st_uid, st.st_gid, content)
    return entries


def compare(new_image, reference, work):
    """Mount both images read-only and compare their trees entry by entry."""
    result = {"reference_sha256": digest(reference)}
    trees, logs = [], []
    for label, image in (("new", new_image), ("reference", reference)):
        mnt = work / f"mnt-{label}"
        mnt.mkdir(exist_ok=True)
        run("mount", "-o", "ro,loop", image, mnt)
        try:
            trees.append(tree(mnt))
            # Logs are expected to differ; keep their text so the difference is shown, not assumed.
            logs.append({str(f.relative_to(mnt)): f.read_text(errors="replace").splitlines()
                         for f in sorted((mnt / "var/log").rglob("*")) if f.is_file()})
        finally:
            run("umount", mnt)
    new, ref = trees
    differences = sorted([f"only in new: {k}" for k in new.keys() - ref.keys()] +
                         [f"only in reference: {k}" for k in ref.keys() - new.keys()] +
                         [f"differs: {k}: {new[k]} != {ref[k]}" for k in new.keys() & ref.keys() if new[k] != ref[k]])
    result.update(entries=len(new), identical_excluding_times=not differences, differences=differences[:50],
                  difference_count=len(differences))
    result["log_diffs"] = {name: list(difflib.unified_diff(logs[1].get(name, []), lines, "reference", "new",
                                                           lineterm="", n=0))[:40]
                           for name, lines in logs[0].items() if lines != logs[1].get(name)}
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--freeze-inputs", action="store_true")
    p.add_argument("--extra-package", action="append", default=[],
                   help="additional top-level APK resolved into a separate pin set")
    p.add_argument("--merge-bin-into-usr", action="store_true",
                   help="link every /bin entry under /usr/bin (merged-/usr layout); the guest's /bin is its initramfs")
    p.add_argument("--compare-with", action="append", default=[], metavar="IMAGE=REFERENCE",
                   help="compare an output image's tree with a reference image in the work directory")
    args = p.parse_args()
    work = Path("/work")
    os.chdir(work)
    tools = json.loads((work / "tools-packages.json").read_text())
    tools_installed = (work / "tools.actual").read_text().split()
    if (work / "tools.installed").read_text().split() != tools_installed:
        raise RuntimeError("orchestration root differs from its pinned closure")
    apks = work / "apks"
    apks.mkdir(exist_ok=True)
    pin_path = work / "packages.json"
    if args.freeze_inputs:
        run("apk", "update")
        urls = subprocess.check_output(["apk", "fetch", "--recursive", "--url",
            "gcc", "musl-dev", "make", "redis", "alpine-baselayout", *args.extra_package], text=True).splitlines()
        records = []
        for url in sorted(set(urls)):
            if not url.startswith("https://dl-cdn.alpinelinux.org/alpine/v3.23/"):
                raise RuntimeError(f"unexpected package source: {url}")
            file = apks / url.rsplit("/", 1)[1]
            fetch(url, file)
            records.append({"url": url, "file": file.name, "sha256": digest(file), "bytes": file.stat().st_size})
        pin_path.write_text(json.dumps({"schema": 1, "architecture": "aarch64", "packages": records}, indent=2) + "\n")
    pins = json.loads(pin_path.read_text())
    for item in pins["packages"]:
        file = apks / item["file"]
        if not file.exists():
            fetch(item["url"], file)
        if digest(file) != item["sha256"]:
            raise RuntimeError(f"package hash mismatch: {file.name}")
    source = json.loads((work / "source.json").read_text())
    archive = work / "xxhash.tar.gz"
    if digest(archive) != source["sha256"]:
        raise RuntimeError("source hash mismatch")
    root = work / "root"
    root.mkdir(exist_ok=False)
    # All dependency resolution has already happened. This stage has no network.
    run("apk", "--root", root, "--initdb", "--no-scripts", "--no-network", "--force-non-repository",
        "--keys-dir", "/etc/apk/keys", "add", *[apks / x["file"] for x in pins["packages"]])
    merged = []
    if args.merge_bin_into_usr:
        # --no-scripts skipped busybox's trigger (`busybox --install -s`); recreate its
        # applet links, pointing at this root's own busybox rather than the builder's.
        applets = subprocess.check_output([str(root / "bin/busybox"), "--list-full"], text=True).split()
        for applet in applets:
            link = root / applet
            if applet.startswith("/") or ".." in applet or link.exists() or link.is_symlink():
                continue
            link.parent.mkdir(parents=True, exist_ok=True)
            link.symlink_to(os.path.relpath(root / "bin/busybox", link.parent))
        for entry in sorted((root / "bin").iterdir()):
            target = root / "usr/bin" / entry.name
            if not target.exists() and not target.is_symlink():
                # Relative: /usr resolves to /toolchain/usr in the guest, so this reaches /toolchain/bin.
                target.symlink_to(f"../../bin/{entry.name}")
                merged.append(entry.name)
    fixture = root / "fixture"
    (fixture / "xxHash").mkdir(parents=True)
    # As root, tar keeps the archive's modes (0664); the published images applied the 022 umask.
    run("tar", "--no-same-permissions", "--no-same-owner", "-xzf", archive, "--strip-components=1",
        "-C", fixture / "xxHash")
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
    # Everything above was written by this (root) process. In the published images the installed
    # tree, the merge links and the fixture all belonged to the Lima user, so hand the whole root over.
    run("chown", "-R", owner(), root)
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
                "extra_packages": args.extra_package, "merged_bin_into_usr": merged,
                "packages": pins, "build_network": "apk install uses --no-network after complete hash verification",
                "filesystem": "ext4; immutable toolchain plus per-instance writable workspace",
                "reproducibility": "reconstruction recipe and exact source/APK inputs; not a bit-reproducible filesystem claim"}
    for name in ("toolchain.img", "data.seed.img", "packages.tar"):
        metadata[name] = {"sha256": digest(work / name), "bytes": (work / name).stat().st_size}
    metadata["orchestration_closure"] = {
        "bootstrap": tools["bootstrap"], "requested": tools["requested"], "apks": tools["packages"],
        "installed": tools_installed, "installed_equals_pinned": True,
        "installed_with": "apk.static from the pinned apk-tools-static",
        "entered_with": "pivot_root in a private mount namespace, env -i; build.py runs inside it as root"}
    metadata["orchestration_host"] = {"contract": (work / "host.actual").read_text().split(),
                                      "checked_by": "bootstrap.sh",
                                      "executes": "busybox sh/tar/sha256sum/awk/mount/unshare/pivot_root, sudo, "
                                                  "the kernel"}
    metadata["reference_comparison"] = {}
    for spec in args.compare_with:
        name, reference = spec.split("=", 1)
        metadata["reference_comparison"][name] = compare(work / name, work / reference, work)
    (work / "build.json").write_text(json.dumps(metadata, indent=2) + "\n")
    run("chown", owner(), *[work / n for n in ("build.json", "packages.json", "toolchain.img", "data.seed.img",
                                                "packages.tar")])


if __name__ == "__main__":
    main()
