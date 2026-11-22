#!/usr/bin/env python3
"""Probe install-time execution using local sentinel archives, never real tools.

Usage: python3 scripts/qualify-mise-install.py /absolute/path/to/mise --output result.json
The result is qualification evidence, not an admission decision. A missing
marker with a failed install does not prove a backend safe. All mise state and
downloads stay in a temporary directory; no user configuration is inherited.
"""

import argparse
import functools
import hashlib
import http.server
import io
import json
from pathlib import Path
import platform
import subprocess
import tarfile
import tempfile
import threading


def digest(data):
    return hashlib.sha256(data).hexdigest()


def isolated_env(root):
    env = {"PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "HOME": str(root / "home")}
    for key, suffix in {
        "MISE_DATA_DIR": "data", "MISE_CONFIG_DIR": "config",
        "MISE_STATE_DIR": "state", "MISE_CACHE_DIR": "cache",
        "MISE_SYSTEM_CONFIG_DIR": "system-config",
        "MISE_SYSTEM_DATA_DIR": "system-data", "MISE_TMP_DIR": "tmp",
    }.items():
        path = root / suffix
        path.mkdir(parents=True, exist_ok=True)
        env[key] = str(path)
    Path(env["HOME"]).mkdir(exist_ok=True)
    global_config = root / "empty.toml"
    global_config.write_text("")
    env.update({
        "MISE_GLOBAL_CONFIG_FILE": str(global_config),
        "MISE_SYSTEM_CONFIG_FILE": str(global_config),
        "MISE_CEILING_PATHS": str(root.parent),
        "MISE_TRUSTED_CONFIG_PATHS": str(root),
        "MISE_LOCKED": "1", "MISE_LOCKED_REQUIRE_CHECKSUM": "1",
        "MISE_COLOR": "0", "MISE_USE_VERSIONS_HOST": "0",
        "MISE_PARANOID": "0",
    })
    return env


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_args):
        pass


def probe(binary, root, server_url, tool, version, backend, executables, extra="", trace=False):
    case = root / tool
    case.mkdir()
    marker = case / "executed.jsonl"
    payload = (
        "#!/bin/sh\n"
        f"printf '%s\\n' \"$0 $*\" >> '{marker}'\n"
        "printf '%s\\n' 'sentinel 1.0.0'\n"
    ).encode()
    conda = backend.startswith("conda:")
    archive = root / ("sentinel-1.0.0-0.tar.bz2" if conda else f"{tool}.tar.gz")
    with tarfile.open(archive, "w:bz2" if conda else "w:gz") as tar:
        if conda:
            metadata_files = {
                "info/index.json": json.dumps({
                    "name": "sentinel", "version": "1.0.0", "build": "0",
                    "build_number": 0, "depends": [],
                    "subdir": {
                        ("Darwin", "arm64"): "osx-arm64",
                        ("Darwin", "x86_64"): "osx-64",
                        ("Linux", "aarch64"): "linux-aarch64",
                        ("Linux", "x86_64"): "linux-64",
                    }[(platform.system(), platform.machine())],
                }).encode(),
                "info/files": ("\n".join(executables) + "\n").encode(),
            }
            for name, data in metadata_files.items():
                info = tarfile.TarInfo(name)
                info.size, info.mode = len(data), 0o644
                tar.addfile(info, io.BytesIO(data))
        if tool == "python":
            info = tarfile.TarInfo("python/lib/python3.13")
            info.type, info.mode = tarfile.DIRTYPE, 0o755
            tar.addfile(info)
            metadata = (b"# system configuration generated and used by the sysconfig module\n"
                        b"build_time_vars = {}\n")
            info = tarfile.TarInfo("python/lib/python3.13/_sysconfigdata__fixture.py")
            info.size, info.mode = len(metadata), 0o644
            tar.addfile(info, io.BytesIO(metadata))
        for name in executables:
            info = tarfile.TarInfo(name)
            info.size, info.mode, info.mtime = len(payload), 0o755, 0
            tar.addfile(info, io.BytesIO(payload))
    url = f"{server_url}/{archive.name}"
    if tool == "go":
        # core:go constructs the archive URL from its mirror, even in locked mode.
        suffix = "darwin-arm64" if platform.system() == "Darwin" else "linux-arm64"
        if platform.machine() == "x86_64":
            suffix = suffix.replace("arm64", "amd64")
        alias = root / f"go{version}.{suffix}.tar.gz"
        alias.write_bytes(archive.read_bytes())
        url = f"{server_url}/{alias.name}"
        extra += f'[settings.go]\ndownload_mirror = "{server_url}"\n'
    checksum = digest(archive.read_bytes())
    (root / f"{archive.name}.sha256").write_text(checksum)
    if tool == "go":
        (root / f"{alias.name}.sha256").write_text(checksum)
    config = f'[tools]\n{json.dumps(tool)} = "{version}"\n'
    if not backend.startswith("core:"):
        config = f'[tools]\n{json.dumps(backend)} = "{version}"\n'
    if backend.startswith("http:"):
        config = f'[tools]\n{json.dumps(backend)} = {{ version = "{version}", url = "{url}" }}\n'
    config += extra
    key = tool if backend.startswith("core:") else backend
    (case / "mise.toml").write_text(config)
    target = f'{"macos" if platform.system() == "Darwin" else "linux"}-' + {
        "arm64": "arm64", "aarch64": "arm64", "x86_64": "x64",
    }[platform.machine()]
    lock = (
        'lockfile_version = 2\n'
        f'[[tools.{json.dumps(key)}]]\nversion = "{version}"\n'
        f'backend = "{backend}"\nspecifiers = ["{version}"]\n'
        + ('options = { channel = "conda-forge" }\n' if conda else '')
        + f'[tools.{json.dumps(key)}.{json.dumps("platforms." + target)}]\n'
        f'url = "{url}"\nchecksum = "sha256:{checksum}"\n'
    )
    (case / "mise.lock").write_text(lock)
    env = isolated_env(case)
    trace_path = case / "file-access.trace"
    prefix = ["/usr/bin/strace", "-f", "-qq", "-yy", "-e", "trace=%file", "-o", str(trace_path)] if trace else []
    result = subprocess.run([*prefix, str(binary), "install", "--locked"], cwd=case,
                            env=env, capture_output=True, text=True, timeout=60)
    capture = subprocess.run([str(binary), "env", "--json"], cwd=case,
                             env=env, capture_output=True, text=True, timeout=60)
    home_accesses = []
    if trace:
        home_accesses = [line.replace(str(root), "<probe>") for line in trace_path.read_text().splitlines()
                         if str(case / "home") in line]
    return {
        "tool": tool, "backend": backend, "target": target,
        "exit_code": result.returncode,
        "executed": marker.read_text().splitlines() if marker.exists() else [],
        "lock_unchanged": (case / "mise.lock").read_text() == lock,
        "config_unchanged": (case / "mise.toml").read_text() == config,
        "capture_exit_code": capture.returncode,
        "home_file_access_trace": home_accesses if trace else None,
        "stdout": result.stdout.replace(str(root), "<probe>"),
        "stderr": result.stderr.replace(str(root), "<probe>"),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mise", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--trace", action="store_true", help="trace Linux install file syscalls with /usr/bin/strace")
    args = parser.parse_args()
    binary = args.mise.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="devcroft-mise-probe-") as tmp:
        root = Path(tmp).resolve()
        version_root = root / "version"
        version_root.mkdir()
        version = subprocess.run([str(binary), "--version"], cwd=version_root,
                                 env=isolated_env(version_root), capture_output=True,
                                 text=True, timeout=20)
        handler = functools.partial(QuietHandler, directory=str(root))
        with http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler) as server:
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            url = f"http://127.0.0.1:{server.server_port}"
            try:
                cases = [
                    ("node", "24.0.0", "core:node", ["node/bin/node", "node/bin/npm"],
                     "[settings.node]\ngpg_verify = false\nnpm_shim = false\n"),
                    ("go", "1.24.0", "core:go", ["go/bin/go"], ""),
                    ("python", "3.13.0", "core:python", ["python/bin/python"],
                     "[settings.python]\ngithub_attestations = false\n"),
                    ("http-probe", "1.0.0", "http:sentinel", ["bin/sentinel"], ""),
                    ("github-probe", "1.0.0", "github:sentinel/sentinel", ["bin/sentinel"], ""),
                    ("aqua-probe", "0.60.0", "aqua:junegunn/fzf", ["fzf"], ""),
                    ("conda-probe", "1.0.0", "conda:sentinel",
                     ["bin/sentinel", "bin/.sentinel-post-link.sh"], ""),
                ]
                results = [probe(binary, root, url, *case, trace=args.trace) for case in cases]
            finally:
                server.shutdown()
                thread.join()
        # The synthetic fixture's marker path varies; it is not a reusable package.
        for result in results:
            result["executed"] = [line.replace(str(root), "<probe>") for line in result["executed"]]
        output = {
            "mise_version": version.stdout.strip(),
            "mise_binary_sha256": digest(binary.read_bytes()),
            "method": "local synthetic archives with shell sentinel executables",
            "scope": "native install only; not a runtime or cross-platform qualification",
            "results": results,
        }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, indent=2) + "\n")
    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()
