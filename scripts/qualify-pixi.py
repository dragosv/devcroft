#!/usr/bin/env python3
"""Probe pixi's install and activation behaviour with local sentinel packages.

Usage: python3 -I scripts/qualify-pixi.py /absolute/path/to/pixi --output result.json

Answers add-pixi-provider tasks 0.1, 0.3 and 0.4 for the host platform:

- whether `pixi lock`, `pixi install --locked` and the *text* form of
  `pixi shell-hook` execute anything a package or the project ships
  (package `activate.d`, post-link scripts, a package's own executables, the
  environment's Python interpreter, project activation scripts);
- whether `run-post-link-scripts = "insecure"` in the project's
  `.pixi/config.toml` changes that (it always merges, even under --no-config);
- whether the design-P4 rewrite of a staged hook keeps pixi's order and leaves
  no staging path outside the prefix, and whether pixi needs the activation
  scripts to exist when it emits the hook;
- whether prefix files are hardlinks, clones or copies of the package cache.

Every package comes from a channel served on loopback from a temporary
directory. The fake `python` writes a marker and exits 1: it never delegates to
a real interpreter, because an installer that ran it would then target
whatever prefix that interpreter reported. The PyPI case downloads one real
pure wheel (`six`) from pypi.org. All pixi state stays in the temporary
directory; no user or system configuration is read.
"""

import argparse
import bz2
import functools
import hashlib
import http.server
import io
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tarfile
import tempfile
import threading
import time

PY_VERSION = "3.12.0"
PY_MM = "3.12"


def subdir():
    machine = platform.machine()
    if platform.system() == "Darwin":
        return "osx-arm64" if machine == "arm64" else "osx-64"
    return {"aarch64": "linux-aarch64", "x86_64": "linux-64"}[machine]


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def tar_bz2(files):
    """files: list of (path, bytes | ('link', target), mode)."""
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w") as tar:
        for path, content, mode in files:
            info = tarfile.TarInfo(path)
            info.mtime = 0
            if isinstance(content, tuple):
                info.type = tarfile.SYMTYPE
                info.linkname = content[1]
                tar.addfile(info)
            else:
                info.size = len(content)
                info.mode = mode
                tar.addfile(info, io.BytesIO(content))
    return bz2.compress(raw.getvalue())


def package(name, version, sub, payload, depends=()):
    """payload: list of (path, bytes | ('link', target), mode)."""
    paths = []
    for path, content, mode in payload:
        if isinstance(content, tuple):
            paths.append({"_path": path, "path_type": "softlink"})
        else:
            paths.append({
                "_path": path, "path_type": "hardlink",
                "sha256": sha256(content), "size_in_bytes": len(content),
            })
    index = {
        "name": name, "version": version, "build": "0_sentinel",
        "build_number": 0, "depends": list(depends), "subdir": sub,
        "license": "MIT", "timestamp": 0,
    }
    meta = [
        ("info/index.json", json.dumps(index).encode(), 0o644),
        ("info/paths.json", json.dumps({"paths": paths, "paths_version": 1}).encode(), 0o644),
        ("info/files", "\n".join(p for p, _, _ in payload).encode() + b"\n", 0o644),
    ]
    data = tar_bz2(meta + list(payload))
    entry = dict(index, md5=hashlib.md5(data).hexdigest(), sha256=sha256(data), size=len(data))
    return f"{name}-{version}-0_sentinel.tar.bz2", data, entry


def marker_script(markers, label, extra=""):
    return (f"#!/bin/sh\necho \"{label} $*\" >> '{markers}/{label}'\n{extra}").encode()


def build_channel(root, markers):
    sub = subdir()
    channel = root / "channel"
    packages = [
        package("sentinel-tool", "1.0", sub, [
            ("bin/sentinel-tool", marker_script(markers, "tool-exec"), 0o755),
            ("etc/conda/activate.d/sentinel.sh",
             f"echo pkg-activate >> '{markers}/order'\n"
             f"echo pkg >> '{markers}/pkg-activate'\n"
             "export SENTINEL_PKG_ACTIVATED=1\n".encode(), 0o644),
            ("bin/.sentinel-tool-post-link.sh", marker_script(markers, "post-link"), 0o755),
            ("bin/.sentinel-tool-pre-unlink.sh", marker_script(markers, "pre-unlink"), 0o755),
        ]),
        package("python", PY_VERSION, sub, [
            (f"bin/python{PY_MM}", marker_script(markers, "python-exec", "exit 1\n"), 0o755),
            ("bin/python", ("link", f"python{PY_MM}"), 0o777),
            ("bin/python3", ("link", f"python{PY_MM}"), 0o777),
            (f"lib/python{PY_MM}/site-packages/README.txt", b"sentinel\n", 0o644),
        ]),
    ]
    repodata = {"info": {"subdir": sub}, "packages": {}, "packages.conda": {}}
    (channel / sub).mkdir(parents=True)
    (channel / "noarch").mkdir(parents=True)
    for filename, data, entry in packages:
        (channel / sub / filename).write_bytes(data)
        repodata["packages"][filename] = entry
    (channel / sub / "repodata.json").write_text(json.dumps(repodata))
    (channel / "noarch" / "repodata.json").write_text(
        json.dumps({"info": {"subdir": "noarch"}, "packages": {}, "packages.conda": {}}))
    return channel


def isolated_env(root, create=True):
    env = {
        "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
        "HOME": str(root / "home"),
        "TMPDIR": str(root / "tmp"),
        "PIXI_HOME": str(root / "pixi-home"),
        "PIXI_CACHE_DIR": str(root / "cache"),
        "RATTLER_CACHE_DIR": str(root / "cache"),
        "PIXI_NO_CONFIG": "true",
        "PIXI_COLOR": "never",
        "PIXI_NO_PROGRESS": "true",
    }
    for key in ("HOME", "TMPDIR", "PIXI_HOME", "PIXI_CACHE_DIR"):
        if create:
            Path(env[key]).mkdir(parents=True, exist_ok=True)
    for key in ("HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY", "SSL_CERT_FILE"):
        if key in os.environ:
            env[key] = os.environ[key]
    return env


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_args):
        pass


def collect(markers):
    out = {p.name: p.read_text().splitlines() for p in sorted(markers.iterdir())}
    for p in markers.iterdir():
        p.unlink()
    return out


def digests(workspace, names):
    return {n: sha256((workspace / n).read_bytes()) for n in names if (workspace / n).exists()}


def run(binary, args, cwd, env):
    p = subprocess.run([binary, *args], cwd=cwd, env=env, capture_output=True, text=True,
                       stdin=subprocess.DEVNULL, timeout=900)
    return {"argv": ["pixi", *args], "exit": p.returncode,
            "stderr_tail": p.stderr.strip().splitlines()[-6:], "stdout": p.stdout}


PROJECT_SCRIPTS = {
    "activate-workspace.sh": "workspace",
    "activate-wtarget.sh": "workspace-target",
    "activate-feature.sh": "feature",
    "activate-ftarget.sh": "feature-target",
}


def write_conda_workspace(ws, channel_url, markers, post_link=None):
    sub = subdir()
    ws.mkdir(parents=True)
    (ws / "pixi.toml").write_text(f"""[workspace]
name = "sentinel"
channels = ["{channel_url}"]
platforms = ["{sub}"]

[dependencies]
sentinel-tool = "1.0.*"

[activation]
scripts = ["activate-workspace.sh"]
env = {{ PROJECT_ENV = "from-activation-env" }}

[target.{sub}.activation]
scripts = ["activate-wtarget.sh"]

[feature.extra.activation]
scripts = ["activate-feature.sh"]

[feature.extra.target.{sub}.activation]
scripts = ["activate-ftarget.sh"]

[environments]
dev = ["extra"]
""")
    for script, label in PROJECT_SCRIPTS.items():
        (ws / script).write_text(
            f"echo {label} >> '{markers}/order'\n"
            f"echo \"$PWD|$PIXI_PROJECT_ROOT\" >> '{markers}/project-{label}'\n")
    if post_link:
        (ws / ".pixi").mkdir()
        (ws / ".pixi" / "config.toml").write_text(f'run-post-link-scripts = "{post_link}"\n')


def link_kind(prefix_file, cache_root):
    if not prefix_file.exists():
        return {"file": prefix_file.name, "missing": True}
    st = prefix_file.stat()
    matches = [p for p in cache_root.rglob(prefix_file.name)
               if p.is_file() and p != prefix_file and "envs" not in p.parts]
    same_inode = any(p.stat().st_ino == st.st_ino for p in matches)
    return {"file": prefix_file.name, "st_nlink": st.st_nlink, "cache_copies": len(matches),
            "same_inode_as_cache": same_inode}


def conda_case(binary, root, channel_url, markers, post_link=None):
    name = "conda-insecure-post-link" if post_link else "conda-only"
    ws = root / name
    write_conda_workspace(ws, channel_url, markers, post_link)
    env = isolated_env(root / f"{name}-state")
    result = {"case": name, "phases": {}}
    collect(markers)

    lock = run(binary, ["lock"], ws, env)
    result["phases"]["lock"] = {"exit": lock["exit"], "markers": collect(markers),
                                "stderr_tail": lock["stderr_tail"]}
    inputs = ["pixi.toml", "pixi.lock"]
    before = digests(ws, inputs)
    install = run(binary, ["install", "--locked", "--environment", "dev"], ws, env)
    result["phases"]["install"] = {"exit": install["exit"], "markers": collect(markers),
                                   "stderr_tail": install["stderr_tail"],
                                   "inputs_unchanged": digests(ws, inputs) == before}
    if post_link:
        return result
    hook = run(binary, ["shell-hook", "--shell", "bash", "--as-is", "--no-completions",
                        "--environment", "dev", "--manifest-path", str(ws / "pixi.toml")], ws, env)
    result["phases"]["shell-hook-text"] = {
        "exit": hook["exit"], "markers": collect(markers),
        "inputs_unchanged": digests(ws, inputs) == before,
        "text": hook["stdout"].replace(str(root), "<tmp>"),
    }
    js = run(binary, ["shell-hook", "--json", "--as-is", "--environment", "dev",
                      "--manifest-path", str(ws / "pixi.toml")], ws, env)
    result["phases"]["shell-hook-json-control"] = {"exit": js["exit"], "markers": collect(markers)}
    prefix = ws / ".pixi" / "envs" / "dev"
    result["prefix_files"] = [link_kind(prefix / "bin" / "sentinel-tool", root / f"{name}-state" / "cache")]
    return result


def staged_rewrite(binary, root, channel_url, markers):
    """Design P4: install and emit the hook in a staging copy, then rewrite."""
    project = root / "project"
    write_conda_workspace(project, channel_url, markers)
    env = isolated_env(root / "staged-state")
    run(binary, ["lock"], project, env)
    collect(markers)
    obj = root / "store" / ("f" * 64)
    obj.mkdir(parents=True)
    for name in ("pixi.toml", "pixi.lock"):
        shutil.copy2(project / name, obj / name)
    install = run(binary, ["install", "--locked", "--environment", "dev"], obj, env)
    out = {"install_exit": install["exit"], "install_markers": collect(markers)}
    hook_args = ["shell-hook", "--shell", "bash", "--as-is", "--no-completions",
                 "--environment", "dev", "--manifest-path", str(obj / "pixi.toml")]
    hook = run(binary, hook_args, obj, env)
    out["without_scripts_staged"] = {
        "exit": hook["exit"], "stderr_tail": hook["stderr_tail"], "markers": collect(markers),
        "project_scripts_in_text": sorted(s for s in PROJECT_SCRIPTS if s in hook["stdout"]),
    }
    # Pixi drops a missing script from the text, so the staging copy carries
    # byte copies of the scripts; the rewrite then points at the real ones.
    for script in PROJECT_SCRIPTS:
        shutil.copy2(project / script, obj / script)
    hook = run(binary, hook_args, obj, env)
    out["hook_exit"] = hook["exit"]
    out["hook_markers"] = collect(markers)
    out["project_scripts_in_text"] = sorted(s for s in PROJECT_SCRIPTS if s in hook["stdout"])
    text = hook["stdout"]
    staging = str(obj)
    rewritten = re.sub(re.escape(staging) + r"(?!/\.pixi/)", str(project), text)
    residue = [line for line in rewritten.splitlines()
               if staging in line and f"{staging}/.pixi/" not in line]
    out["residue_outside_prefix"] = residue
    out["hook_text_rewritten"] = rewritten.replace(str(root), "<tmp>")
    script = f"cd '{project}'\n{rewritten}\nunset PIXI_EXE\nenv\n"
    p = subprocess.run(["/bin/bash", "--noprofile", "--norc", "-c", script],
                       env={"PATH": "/usr/bin:/bin", "HOME": str(root / "home")},
                       capture_output=True, text=True)
    exported = dict(line.split("=", 1) for line in p.stdout.splitlines() if "=" in line)
    out["confined_run"] = {
        "exit": p.returncode,
        "markers": collect(markers),
        "PIXI_PROJECT_ROOT": exported.get("PIXI_PROJECT_ROOT", "").replace(str(root), "<tmp>"),
        "PIXI_PROJECT_MANIFEST": exported.get("PIXI_PROJECT_MANIFEST", "").replace(str(root), "<tmp>"),
        "CONDA_PREFIX": exported.get("CONDA_PREFIX", "").replace(str(root), "<tmp>"),
        "PROJECT_ENV": exported.get("PROJECT_ENV"),
        "SENTINEL_PKG_ACTIVATED": exported.get("SENTINEL_PKG_ACTIVATED"),
        "PIXI_EXE_unset": "PIXI_EXE" not in exported,
        "staging_in_env": sorted(k for k, v in exported.items()
                                 if staging in v and f"{staging}/.pixi/" not in v),
    }
    return out


def pypi_case(binary, root, channel_url, markers):
    sub = subdir()
    ws = root / "pypi"
    ws.mkdir()
    (ws / "pixi.toml").write_text(f"""[workspace]
name = "sentinel-pypi"
channels = ["{channel_url}"]
platforms = ["{sub}"]

[dependencies]
python = "=={PY_VERSION}"

[pypi-dependencies]
six = "==1.16.0"
""")
    env = isolated_env(root / "pypi-state")
    collect(markers)
    result = {"case": "pypi-wheel", "phases": {}}
    lock = run(binary, ["lock"], ws, env)
    result["phases"]["lock"] = {"exit": lock["exit"], "markers": collect(markers),
                                "stderr_tail": lock["stderr_tail"]}
    if lock["exit"] != 0:
        return result
    inputs = ["pixi.toml", "pixi.lock"]
    before = digests(ws, inputs)
    install = run(binary, ["install", "--locked"], ws, env)
    result["phases"]["install"] = {"exit": install["exit"], "markers": collect(markers),
                                   "stderr_tail": install["stderr_tail"],
                                   "inputs_unchanged": digests(ws, inputs) == before}
    site = ws / ".pixi" / "envs" / "default" / "lib" / f"python{PY_MM}" / "site-packages"
    result["six_installed"] = (site / "six.py").exists()
    result["lock_pypi_entry"] = [l for l in (ws / "pixi.lock").read_text().splitlines()
                                 if "six" in l]
    return result


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("pixi")
    ap.add_argument("--output", required=True)
    args = ap.parse_args()
    binary = str(Path(args.pixi).resolve())
    version = subprocess.run([binary, "--version"], capture_output=True, text=True,
                             env={"PATH": "/usr/bin:/bin"}).stdout.strip()
    result = {
        "pixi": version,
        "pixi_sha256": sha256(Path(binary).read_bytes()),
        "platform": subdir(),
        "measured_at": time.strftime("%Y-%m-%d"),
        "isolation": sorted(isolated_env(Path("/unused"), create=False).keys()),
    }
    with tempfile.TemporaryDirectory(prefix="devcroft-pixi-qual-") as tmp:
        root = Path(tmp).resolve()
        markers = root / "markers"
        markers.mkdir()
        channel = build_channel(root, markers)
        handler = functools.partial(QuietHandler, directory=str(channel))
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        url = f"http://127.0.0.1:{server.server_address[1]}"
        try:
            result["cases"] = [
                conda_case(binary, root, url, markers),
                conda_case(binary, root, url, markers, post_link="insecure"),
                pypi_case(binary, root, url, markers),
            ]
            result["staged_rewrite"] = staged_rewrite(binary, root, url, markers)
        finally:
            server.shutdown()
    Path(args.output).write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
