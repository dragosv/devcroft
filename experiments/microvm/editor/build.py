#!/usr/bin/env python3
"""Linux-only Alpine/npm editor preparation on a private copy of the base image."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile

WORK = Path("/home/dragos.guest/devcroft-editor")


def run(*args, cwd=None, env=None, timeout=900):
    subprocess.run(list(map(str, args)), check=True, cwd=cwd, env=env, timeout=timeout)


def digest(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


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
    parser.add_argument("--freeze-inputs", action="store_true")
    args = parser.parse_args()
    os.chdir(WORK)
    pin = json.loads((WORK / "source.json").read_text())
    if digest(WORK / "base.img") != pin["base_toolchain_sha256"] or digest(WORK / "code-server.tgz") != pin["sha256"]:
        raise RuntimeError("editor/base input mismatch")
    apks = WORK / "apks"
    apks.mkdir(exist_ok=True)
    if args.freeze_inputs:
        # apk fetch --url omits archives already present in its output dir.
        # Resolve into a fresh directory so a repeated freeze cannot erase pins.
        with tempfile.TemporaryDirectory(prefix="resolve-", dir=WORK) as fresh:
            urls = subprocess.check_output(["apk", "fetch", "--recursive", "--url", "--output", fresh,
                "nodejs", "bash", "krb5-libs", "libxkbfile", "libsecret"], text=True).splitlines()
            packages = []
            for url in sorted(set(urls)):
                if not url.startswith("https://dl-cdn.alpinelinux.org/alpine/v3.23/"):
                    raise RuntimeError("unexpected APK origin")
                file = apks / url.rsplit("/", 1)[1]
                shutil.copyfile(Path(fresh) / file.name, file)
                packages.append({"url": url, "file": file.name, "sha256": digest(file)})
        if not packages:
            raise RuntimeError("APK resolution returned an empty runtime closure")
        (WORK / "packages.json").write_text(json.dumps({"schema": 1, "packages": packages}, indent=2) + "\n")
    packages = json.loads((WORK / "packages.json").read_text())["packages"]
    if not packages:
        raise RuntimeError("empty runtime APK manifest")
    for item in packages:
        file = apks / item["file"]
        if not file.exists():
            run("curl", "--fail", "--location", "--max-time", "120", item["url"], "-o", file)
        if digest(file) != item["sha256"]:
            raise RuntimeError(f"APK mismatch: {file.name}")
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
        run("sudo", "mkdir", "-p", prefix)
        run("sudo", "chown", "-R", f"{os.getuid()}:{os.getgid()}", root / "editor")
        patches = extract_editor(prefix, pin)
        lockfiles = [prefix / "npm-shrinkwrap.json", prefix / "lib/vscode/npm-shrinkwrap.json",
                     prefix / "lib/vscode/extensions/npm-shrinkwrap.json"]
        before = {str(f.relative_to(prefix)): digest(f) for f in lockfiles}
        env = dict(os.environ, npm_config_cache=str(WORK / "npm-cache"), npm_config_nodedir="/usr",
                   npm_config_build_from_source="true", MAKEFLAGS="-j2", npm_config_jobs="2",
                   CFLAGS="-fno-lto", CXXFLAGS="-fno-lto", LDFLAGS="-fno-lto")
        run("npm", "ci", "--omit=dev", "--foreground-scripts", "--no-audit", "--no-fund", cwd=prefix, env=env, timeout=1200)
        # Reconstruct from a fresh source tree, not generated files left by the
        # first install. A separate network namespace also covers native scripts
        # which might ignore npm's offline setting.
        shutil.rmtree(prefix)
        prefix.mkdir()
        if extract_editor(prefix, pin) != patches:
            raise RuntimeError("fresh-source patch mismatch")
        offline = dict(env, npm_config_offline="true")
        run("sudo", "unshare", "--net", "sudo", "-u", f"#{os.getuid()}", "-g", f"#{os.getgid()}",
            "--", "env", *[f"{k}={v}" for k, v in sorted(offline.items())],
            "npm", "ci", "--offline", "--omit=dev", "--foreground-scripts", "--no-audit", "--no-fund",
            cwd=prefix, timeout=1200)
        after = {str(f.relative_to(prefix)): digest(f) for f in lockfiles}
        if before != after:
            raise RuntimeError("published shrinkwrap changed during installation")
        run("sudo", "chroot", root, "/usr/bin/node", "/editor/node_modules/code-server/out/node/entry.js", "--version")
        installed = subprocess.check_output(["apk", "--root", str(root), "info", "-v"], text=True).splitlines()
        run("sync")
    finally:
        run("sudo", "umount", root)
    with tarfile.open(WORK / "npm-cache.tar", "w") as out:
        out.add(WORK / "npm-cache", arcname="npm-cache")
    receipt = {"schema": 1, "source": pin, "runtime_apks": packages, "npm_shrinkwrap_sha256": before,
        "patches": patches,
        "offline_dependency_reconstruction": True,
        "offline_verification": {"fresh_source_tree": True, "network_namespace": "unshare --net"},
        "build_script_sha256": digest(Path(__file__)),
        "node_version": subprocess.check_output(["node", "--version"], text=True).strip(),
        "exported_runtime_node_version": runtime_node,
        "npm_version": subprocess.check_output(["npm", "--version"], text=True).strip(),
        "native_build_flags": {k: env[k] for k in ("CFLAGS", "CXXFLAGS", "LDFLAGS", "MAKEFLAGS", "npm_config_nodedir")},
        "installed_packages": installed, "builder_packages": subprocess.check_output(["apk", "info", "-v"], text=True).splitlines(),
        "editor.img": {"sha256": digest(image), "bytes": image.stat().st_size},
        "npm-cache.tar": {"sha256": digest(WORK / "npm-cache.tar")},
        "reproducibility": "fixed source/shrinkwrap/APKs and offline reinstall; not bit-identical filesystem output"}
    (WORK / "build.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
