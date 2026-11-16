#!/usr/bin/env python3
"""Linux-only Alpine/npm editor preparation on a private copy of the base image."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tarfile
import tempfile

WORK = Path("/home/dragos.guest/devcroft-editor")
BUILDER = Path("/home/dragos.guest/devcroft-editor-builder")  # outside WORK, which is bound into it
INNER = Path("/work")  # WORK as seen inside BUILDER
ALPINE = "https://dl-cdn.alpinelinux.org/alpine/v3.23/"
# Everything the npm/native build executes comes from this pinned closure, installed
# into BUILDER and entered with chroot. The Lima system only orchestrates (python3,
# mount, apk, e2fsprogs); none of its compilers or runtimes reach the build.
BUILDER_PACKAGES = ["busybox", "bash", "ca-certificates", "nodejs", "nodejs-dev", "npm", "g++", "make", "python3",
                    "pkgconf", "openssl-dev", "krb5-dev", "libx11-dev", "libxkbfile-dev", "libsecret-dev"]


def run(*args, cwd=None, env=None, timeout=900):
    subprocess.run(list(map(str, args)), check=True, cwd=cwd, env=env, timeout=timeout)


def digest(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def resolve_closure(names, dest, label):
    """Resolve an APK closure into dest; return the url/file/sha256 manifest."""
    # apk fetch --url omits archives already present in its output dir.
    # Resolve into a fresh directory so a repeated freeze cannot erase pins.
    with tempfile.TemporaryDirectory(prefix="resolve-", dir=WORK) as fresh:
        urls = subprocess.check_output(["apk", "fetch", "--recursive", "--url", "--output", fresh, *names],
                                       text=True).splitlines()
        packages = []
        for url in sorted(set(urls)):
            if not url.startswith(ALPINE):
                raise RuntimeError("unexpected APK origin")
            file = dest / url.rsplit("/", 1)[1]
            shutil.copyfile(Path(fresh) / file.name, file)
            packages.append({"url": url, "file": file.name, "sha256": digest(file)})
    if not packages:
        raise RuntimeError(f"APK resolution returned an empty {label} closure")
    return packages


def fetch_pinned(packages, dest):
    for item in packages:
        file = dest / item["file"]
        if not file.exists():
            run("curl", "--fail", "--location", "--max-time", "120", item["url"], "-o", file)
        if digest(file) != item["sha256"]:
            raise RuntimeError(f"APK mismatch: {file.name}")


def apk_name(file):
    """`name-ver-rel` as `apk info -v` prints it, from `name-ver-rel.apk`."""
    return file.removesuffix(".apk")


def make_builder(packages):
    """A fresh chroot holding exactly the pinned builder closure, with its install scripts run."""
    if BUILDER.exists():
        run("sudo", "rm", "-rf", BUILDER)  # This script owns only this explicitly named build root.
    BUILDER.mkdir()
    run("sudo", "apk", "--root", BUILDER, "--initdb", "--no-network", "--no-cache", "--force-non-repository",
        "--keys-dir", "/etc/apk/keys", "add", *[WORK / "builder-apks" / x["file"] for x in packages])
    installed = sorted(subprocess.check_output(["apk", "--root", str(BUILDER), "info", "-v"], text=True).split())
    if installed != sorted(apk_name(x["file"]) for x in packages):
        raise RuntimeError(("builder root differs from the pinned closure", installed))
    return installed


class Mounted:
    """Bind WORK and the mounted editor image into the builder root, with /dev, /proc and a resolver.

    Each bind is separate, private and unmounted explicitly: a recursive bind kept the image's
    filesystem mounted through the builder after WORK/root was unmounted, so the exported image
    still needed journal recovery and the read-only guest mount refused it.
    """
    def __init__(self, image_root):
        self.mounts = [(WORK, BUILDER / "work"), (image_root, BUILDER / "work" / image_root.relative_to(WORK)),
                       (Path("/dev"), BUILDER / "dev")]

    def __enter__(self):
        for path in ("work", "dev", "proc", "etc"):
            run("sudo", "mkdir", "-p", BUILDER / path)
        self.done = []
        for source, target in self.mounts:
            run("sudo", "mount", "--bind", source, target)
            run("sudo", "mount", "--make-private", target)
            self.done.append(target)
        run("sudo", "mount", "-t", "proc", "proc", BUILDER / "proc")
        self.done.append(BUILDER / "proc")
        run("sudo", "cp", "/etc/resolv.conf", BUILDER / "etc/resolv.conf")
        return self

    def __exit__(self, *exc):
        for target in reversed(self.done):
            run("sudo", "umount", target)


def in_builder(cwd, env, *argv, offline=False, timeout=1200, capture=False):
    """Run argv as the invoking user inside the builder chroot (optionally with no network)."""
    inner = " ".join(f"{k}={shlex.quote(v)}" for k, v in sorted(env.items()))
    script = f"cd {shlex.quote(str(cwd))} && exec env -i {inner} " + " ".join(map(shlex.quote, map(str, argv)))
    cmd = ["sudo", *(["unshare", "--net"] if offline else []), "chroot", f"--userspec={os.getuid()}:{os.getgid()}",
           str(BUILDER), "/bin/sh", "-c", script]
    if capture:
        return subprocess.check_output(cmd, text=True, timeout=timeout).strip()
    subprocess.run(cmd, check=True, timeout=timeout)


def extract_editor(prefix, pin):
    run("tar", "-xzf", WORK / "code-server.tgz", "--strip-components=1", "-C", prefix)
    script = prefix / "postinstall.sh"
    before = digest(script)
    text = script.read_text()
    original = "npm install --omit=dev"
    replacement = "npm ci --omit=dev --no-audit --no-fund"
    if text.count(original) != 1:
        raise RuntimeError("upstream nested-install patch no longer matches")
    script.write_text(text.replace(original, replacement))
    patches = [{"file": "postinstall.sh", "before_sha256": before, "after_sha256": digest(script),
                "replace": original, "with": replacement}]
    # The published lock omits applicationinsights-core-js's wildcard tslib
    # peer. Add the explicitly frozen resolution instead of resolving at build.
    for name, additions in pin["shrinkwrap_additions"].items():
        file = prefix / name
        before = digest(file)
        lock = json.loads(file.read_text())
        for key, value in additions.items():
            if key in lock["packages"]:
                raise RuntimeError("upstream shrinkwrap patch no longer matches")
            lock["packages"][key] = value
        file.write_text(json.dumps(lock, indent=2) + "\n")
        patches.append({"file": name, "before_sha256": before, "after_sha256": digest(file), "additions": additions})
    return patches


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze-inputs", action="store_true", help="re-resolve and repin the runtime APK closure")
    parser.add_argument("--freeze-builder", action="store_true", help="re-resolve and repin the builder APK closure")
    args = parser.parse_args()
    os.chdir(WORK)
    pin = json.loads((WORK / "source.json").read_text())
    if digest(WORK / "base.img") != pin["base_toolchain_sha256"] or digest(WORK / "code-server.tgz") != pin["sha256"]:
        raise RuntimeError("editor/base input mismatch")
    apks, builder_apks = WORK / "apks", WORK / "builder-apks"
    apks.mkdir(exist_ok=True)
    builder_apks.mkdir(exist_ok=True)
    if args.freeze_inputs:
        packages = resolve_closure(["nodejs", "bash", "krb5-libs", "libxkbfile", "libsecret"], apks, "runtime")
        (WORK / "packages.json").write_text(json.dumps({"schema": 1, "packages": packages}, indent=2) + "\n")
    if args.freeze_builder:
        builder = resolve_closure(BUILDER_PACKAGES, builder_apks, "builder")
        (WORK / "builder-packages.json").write_text(json.dumps(
            {"schema": 1, "requested": BUILDER_PACKAGES, "packages": builder}, indent=2) + "\n")
    packages = json.loads((WORK / "packages.json").read_text())["packages"]
    manifest = json.loads((WORK / "builder-packages.json").read_text())
    builder = manifest["packages"]
    if not packages or not builder or manifest["requested"] != BUILDER_PACKAGES:
        raise RuntimeError("empty runtime/builder APK manifest, or builder request changed without --freeze-builder")
    fetch_pinned(packages, apks)
    fetch_pinned(builder, builder_apks)
    builder_installed = make_builder(builder)
    image = WORK / "editor.img"
    if image.exists():
        image.unlink()  # This script owns only this explicitly named build output.
    shutil.copyfile(WORK / "base.img", image)
    run("truncate", "-s", "1536M", image)
    run("resize2fs", image)
    root = WORK / "root"
    root.mkdir(exist_ok=True)
    run("sudo", "mount", "-o", "loop", image, root)
    try:
        run("sudo", "apk", "--root", root, "--no-scripts", "--no-network", "--force-non-repository",
            "--keys-dir", "/etc/apk/keys", "add", *[apks / x["file"] for x in packages])
        runtime_node = subprocess.check_output(["sudo", "chroot", str(root), "/usr/bin/node", "--version"], text=True).strip()
        if not runtime_node.startswith(f"v{pin['node_major']}."):
            raise RuntimeError("unexpected Node runtime in exported filesystem")
        prefix = root / "editor/node_modules/code-server"
        inner_prefix = INNER / prefix.relative_to(WORK)
        run("sudo", "mkdir", "-p", prefix)
        run("sudo", "chown", "-R", f"{os.getuid()}:{os.getgid()}", root / "editor")
        (WORK / "home").mkdir(exist_ok=True)
        patches = extract_editor(prefix, pin)
        lockfiles = [prefix / "npm-shrinkwrap.json", prefix / "lib/vscode/npm-shrinkwrap.json",
                     prefix / "lib/vscode/extensions/npm-shrinkwrap.json"]
        before = {str(f.relative_to(prefix)): digest(f) for f in lockfiles}
        # A fixed environment: nothing is inherited from the Lima session (env -i).
        env = {"PATH": "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin", "HOME": str(INNER / "home"),
               "npm_config_cache": str(INNER / "npm-cache"), "npm_config_nodedir": "/usr",
               "npm_config_build_from_source": "true", "MAKEFLAGS": "-j2", "npm_config_jobs": "2",
               "CFLAGS": "-fno-lto", "CXXFLAGS": "-fno-lto", "LDFLAGS": "-fno-lto"}
        npm_ci = ["npm", "ci", "--omit=dev", "--foreground-scripts", "--no-audit", "--no-fund"]
        with Mounted(root):
            tools = {"node": in_builder(INNER, env, "node", "--version", capture=True),
                     "npm": in_builder(INNER, env, "npm", "--version", capture=True),
                     "gcc": in_builder(INNER, env, "gcc", "-dumpfullversion", capture=True),
                     "python3": in_builder(INNER, env, "python3", "--version", capture=True)}
            in_builder(inner_prefix, env, *npm_ci)
            # Reconstruct from a fresh source tree, not generated files left by the
            # first install. A separate network namespace also covers native scripts
            # which might ignore npm's offline setting.
            shutil.rmtree(prefix)
            prefix.mkdir()
            if extract_editor(prefix, pin) != patches:
                raise RuntimeError("fresh-source patch mismatch")
            in_builder(inner_prefix, dict(env, npm_config_offline="true"), *npm_ci, "--offline", offline=True)
        after = {str(f.relative_to(prefix)): digest(f) for f in lockfiles}
        if before != after:
            raise RuntimeError("published shrinkwrap changed during installation")
        run("sudo", "chroot", root, "/usr/bin/node", "/editor/node_modules/code-server/out/node/entry.js", "--version")
        installed = subprocess.check_output(["apk", "--root", str(root), "info", "-v"], text=True).splitlines()
        run("sync")
    finally:
        run("sudo", "umount", root)
    if subprocess.run(["sudo", "grep", "-q", str(root), "/proc/mounts"]).returncode == 0:
        raise RuntimeError("editor image is still mounted after unmount")
    # The guest mounts this image read-only; a filesystem needing journal recovery will not mount.
    run("e2fsck", "-fn", image)
    with tarfile.open(WORK / "npm-cache.tar", "w") as out:
        out.add(WORK / "npm-cache", arcname="npm-cache")
    receipt = {"schema": 2, "source": pin, "runtime_apks": packages, "npm_shrinkwrap_sha256": before,
        "patches": patches,
        "offline_dependency_reconstruction": True,
        "offline_verification": {"fresh_source_tree": True, "network_namespace": "unshare --net"},
        "build_script_sha256": digest(Path(__file__)),
        "builder_closure": {"requested": BUILDER_PACKAGES, "apks": builder, "installed": builder_installed,
                            "installed_equals_pinned": True, "entered_with": "chroot, env -i",
                            "tools": tools},
        "exported_runtime_node_version": runtime_node,
        "native_build_flags": {k: env[k] for k in ("CFLAGS", "CXXFLAGS", "LDFLAGS", "MAKEFLAGS", "npm_config_nodedir")},
        "installed_packages": installed,
        "orchestration_host_packages": subprocess.check_output(["apk", "info", "-v"], text=True).splitlines(),
        "editor.img": {"sha256": digest(image), "bytes": image.stat().st_size},
        "npm-cache.tar": {"sha256": digest(WORK / "npm-cache.tar")},
        "reproducibility": "fixed source/shrinkwrap/runtime and builder APK closures, offline reinstall; "
                           "not bit-identical filesystem output; the builder kernel is the Lima VM's"}
    (WORK / "build.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
