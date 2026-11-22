#!/usr/bin/env python3
"""Restore a real conda-forge environment with pixi and run it under policy.

Usage:
  python3 -I scripts/qualify-pixi-runtime.py /abs/pixi --root /abs/fresh-dir \
      --helper target/debug/examples/mise_qualification --output result.json

add-pixi-provider task 0.5, native target only. The root must lie outside
/tmp: the baseline grants /tmp for writing, so a prefix under it could never
show that the store is read-only.

1. `pixi lock` a Conda-only workspace (python, numpy, go; a C compiler on
   Linux) and `pixi install --locked` it into a staging object, as design P2.
2. Emit the text hook there (scripts staged), rewrite staging paths to the
   project (design P4), and drop the host PATH tail.
3. Find, by reading headers only, the libraries executables in the prefix
   need from outside it, and the files the host shell needs.
4. Apply the real policy compiler (`provider:pixi` grants only) and run, under
   it: numpy, a Go build, a C build on Linux, and the denial probes (host
   `cc`/`gcc`/`ld`/`git`, a write into the prefix).
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import time

ISOLATED = ("PIXI_NO_CONFIG", "true")


def env_for(state):
    env = {"PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "HOME": str(state / "home"),
           "TMPDIR": str(state / "tmp"), "PIXI_HOME": str(state / "pixi-home"),
           "PIXI_CACHE_DIR": str(state / "cache"), "RATTLER_CACHE_DIR": str(state / "cache"),
           "PIXI_NO_CONFIG": "true", "PIXI_COLOR": "never", "PIXI_NO_PROGRESS": "true"}
    for key in ("HOME", "TMPDIR", "PIXI_HOME", "PIXI_CACHE_DIR"):
        Path(env[key]).mkdir(parents=True, exist_ok=True)
    for key in ("HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY", "SSL_CERT_FILE"):
        if key in os.environ:
            env[key] = os.environ[key]
    return env


def pixi(binary, args, cwd, env):
    p = subprocess.run([binary, *args], cwd=cwd, env=env, capture_output=True, text=True,
                       stdin=subprocess.DEVNULL, timeout=1800)
    if p.returncode:
        raise RuntimeError(f"pixi {' '.join(args)}: {p.stderr[-2000:]}")
    return p.stdout


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def native():
    if platform.system() == "Darwin":
        return "osx-arm64" if platform.machine() == "arm64" else "osx-64"
    return {"aarch64": "linux-aarch64", "x86_64": "linux-64"}[platform.machine()]


MANIFEST = """[workspace]
name = "pixiprobe"
channels = ["conda-forge"]
platforms = ["{platform}"]

[dependencies]
python = "3.12.*"
numpy = "*"
go = "*"

[target.linux-aarch64.dependencies]
c-compiler = "*"

[target.linux-64.dependencies]
c-compiler = "*"
"""

GO_MAIN = 'package main\n\nimport "fmt"\n\nfunc main() { fmt.Println("go-build-ok") }\n'
C_MAIN = '#include <stdio.h>\nint main(void){puts("c-build-ok");return 0;}\n'


def needed_outside(prefix):
    """Libraries and interpreters needed from outside the prefix, by reading."""
    found = {}
    files = [p for p in prefix.rglob("*") if p.is_file() and not p.is_symlink()
             and os.access(p, os.X_OK) or p.suffix in (".so", ".dylib")]
    for f in files:
        try:
            head = f.read_bytes()[:4]
        except OSError:
            continue
        if platform.system() == "Darwin" and head in (b"\xcf\xfa\xed\xfe",):
            out = subprocess.run(["/usr/bin/otool", "-L", str(f)], capture_output=True, text=True).stdout
            for line in out.splitlines()[1:]:
                lib = line.strip().split(" (")[0]
                if lib.startswith("/") and not lib.startswith(str(prefix)):
                    found.setdefault(lib, 0)
                    found[lib] += 1
        elif platform.system() == "Linux" and head == b"\x7fELF":
            out = subprocess.run(["readelf", "-dl", str(f)], capture_output=True, text=True).stdout
            for m in re.finditer(r"Requesting program interpreter: (\S+)\]", out):
                found.setdefault(m.group(1), 0)
                found[m.group(1)] += 1
            for m in re.finditer(r"\(NEEDED\)\s+Shared library: \[(\S+)\]", out):
                lib = m.group(1)
                if not list(prefix.rglob(lib)):
                    found.setdefault(lib, 0)
                    found[lib] += 1
    return dict(sorted(found.items()))


def bin_closure_outside(prefix):
    """What the prefix's own executables load from outside it, transitively.

    Seeds are the files in `bin/`, the entry points a session runs. Shared
    objects reached only through dlopen (a Python extension such as tkinter's
    libtk) or target files under a cross sysroot are not seeds, so they cannot
    make the host inventory grow; they fail at use time instead.
    """
    outside, seen = {}, set()
    queue = [p for p in (prefix / "bin").iterdir() if p.is_file()]
    libdirs = [prefix / "lib"]
    while queue:
        f = Path(os.path.realpath(queue.pop()))
        if f in seen or not str(f).startswith(str(prefix)):
            continue
        seen.add(f)
        try:
            head = f.read_bytes()[:4]
        except OSError:
            continue
        if platform.system() == "Darwin" and head == b"\xcf\xfa\xed\xfe":
            out = subprocess.run(["/usr/bin/otool", "-L", str(f)], capture_output=True, text=True).stdout
            for line in out.splitlines()[1:]:
                lib = line.strip().split(" (")[0]
                if lib.startswith(("@rpath/", "@loader_path/", "@executable_path/")):
                    name = lib.split("/", 1)[1]
                    hits = [d / name for d in libdirs if (d / name).exists()]
                    queue.extend(hits)
                elif lib.startswith(str(prefix)):
                    queue.append(Path(lib))
                elif lib.startswith("/"):
                    outside[lib] = outside.get(lib, 0) + 1
        elif platform.system() == "Linux" and head == b"\x7fELF":
            out = subprocess.run(["readelf", "-dl", str(f)], capture_output=True, text=True).stdout
            for m in re.finditer(r"Requesting program interpreter: (\S+)\]", out):
                outside[m.group(1)] = outside.get(m.group(1), 0) + 1
            for m in re.finditer(r"\(NEEDED\)\s+Shared library: \[(\S+)\]", out):
                hits = [d / m.group(1) for d in libdirs if (d / m.group(1)).exists()]
                if hits:
                    queue.extend(hits)
                else:
                    outside[m.group(1)] = outside.get(m.group(1), 0) + 1
    return dict(sorted(outside.items()))


def shell_files():
    """The host shells and, on Linux, what they load, by reading."""
    if platform.system() == "Darwin":
        return {"grants": ["/bin", "/var/select"], "note": "macOS /bin/sh is a selector reading /var/select"}
    out = {}
    for sh in ("/bin/sh", "/bin/bash"):
        real = os.path.realpath(sh)
        deps = subprocess.run(["readelf", "-dl", real], capture_output=True, text=True).stdout
        interp = re.findall(r"Requesting program interpreter: (\S+)\]", deps)
        libs = re.findall(r"\(NEEDED\)\s+Shared library: \[(\S+)\]", deps)
        out[sh] = {"canonical": real, "interpreter": interp, "needed": libs}
    return out


def linux_lib_paths(names):
    paths = []
    for name in names:
        if name.startswith("/"):
            paths.append(os.path.realpath(name))
            continue
        for d in ("/lib/aarch64-linux-gnu", "/usr/lib/aarch64-linux-gnu",
                  "/lib/x86_64-linux-gnu", "/usr/lib/x86_64-linux-gnu", "/lib", "/usr/lib"):
            if os.path.exists(os.path.join(d, name)):
                paths.append(os.path.realpath(os.path.join(d, name)))
                break
    return sorted(set(paths))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("pixi")
    ap.add_argument("--root", required=True, type=Path)
    ap.add_argument("--helper", required=True, type=Path)
    ap.add_argument("--output", required=True)
    args = ap.parse_args()
    binary = str(Path(args.pixi).resolve(strict=True))
    root = args.root.resolve()
    if str(root).startswith(("/tmp", "/private/tmp", "/var/folders", "/private/var/folders")):
        ap.error("the root must lie outside the baseline's scratch grants")
    if root.exists():
        shutil.rmtree(root)
    project, store, state = root / "project", root / "store", root / "state"
    project.mkdir(parents=True)
    plat = native()
    (project / "pixi.toml").write_text(MANIFEST.format(platform=plat))
    (project / "gohello").mkdir()
    (project / "gohello" / "go.mod").write_text("module gohello\n\ngo 1.21\n")
    (project / "gohello" / "main.go").write_text(GO_MAIN)
    (project / "chello.c").write_text(C_MAIN)
    env = env_for(state)
    version = pixi(binary, ["--version"], project, env).strip()

    pixi(binary, ["lock"], project, env)
    lock_digest = sha(project / "pixi.lock")
    obj = store / lock_digest
    obj.mkdir(parents=True)
    for name in ("pixi.toml", "pixi.lock"):
        shutil.copy2(project / name, obj / name)
    t0 = time.time()
    pixi(binary, ["install", "--locked", "--environment", "default"], obj, env)
    install_seconds = round(time.time() - t0, 1)
    inputs_unchanged = sha(obj / "pixi.lock") == lock_digest and \
        sha(obj / "pixi.toml") == sha(project / "pixi.toml")
    prefix = obj / ".pixi" / "envs" / "default"
    hook = pixi(binary, ["shell-hook", "--shell", "bash", "--as-is", "--no-completions",
                         "--change-ps1", "false", "--environment", "default",
                         "--manifest-path", str(obj / "pixi.toml")], obj, env)
    staging = str(obj)
    hook = re.sub(re.escape(staging) + r"(?!/\.pixi/)", str(project), hook)
    hook = hook.replace(":/usr/bin:/bin:/usr/sbin:/sbin", "")
    residue = [l for l in hook.splitlines() if staging in l and f"{staging}/.pixi/" not in l]

    outside_all = needed_outside(prefix)
    outside = bin_closure_outside(prefix)
    shells = shell_files()
    if platform.system() == "Darwin":
        host_grants = ["/bin", "/var/select"]
        for lib in outside:
            if lib.startswith("/usr/lib/"):
                host_grants.append("/usr/lib")
            elif "/System/Library/Frameworks/" in lib:
                host_grants.append(lib.split(".framework/")[0] + ".framework")
            else:
                host_grants.append(lib)
    else:
        names = list(outside)
        for info in shells.values():
            names += info["interpreter"] + info["needed"]
        host_grants = [info["canonical"] for info in shells.values()] + linux_lib_paths(names)
    host_grants = sorted(set(host_grants))
    grants = [str(prefix)] + host_grants

    c_build = 'cc -o chello chello.c && ./chello\n'
    script = f"""set -u
cd '{project}'
{hook}
unset PIXI_EXE
python -c 'import numpy, sys; print("numpy-ok", int(numpy.arange(10).sum()), sys.prefix == "{prefix}")'
(cd gohello && GOCACHE="$PWD/.gocache" GOPATH="$PWD/.gopath" CGO_ENABLED=0 GOTOOLCHAIN=local GOFLAGS=-buildvcs=false go build -o hello . && ./hello)
{c_build if plat.startswith('linux') else 'echo c-build-not-attempted-on-macos'}
for t in cc gcc ld git; do
  w=$(command -v "$t" 2>/dev/null || true)
  case "$w" in ""|"$CONDA_PREFIX"/*) ;; *) echo "resolves:$t:$w";; esac
  [ -n "$w" ] && echo "from-environment:$t"
  if /usr/bin/$t --version >/dev/null 2>&1; then echo "ran:/usr/bin/$t"; fi
done
if ( echo x >> "$CONDA_PREFIX/bin/python3.12" ) 2>/dev/null; then echo prefix-write-allowed; else echo prefix-write-denied; fi
echo policy-probe-done
"""
    bash = "/bin/bash" if platform.system() == "Darwin" else shells["/bin/bash"]["canonical"]
    request = {"provider": "pixi", "grants": grants,
               "env": {"PATH": "/bin" if platform.system() == "Darwin" else "/nonexistent",
                       "HOME": str(project / ".home")},
               "argv": [bash, "--noprofile", "--norc", "-c", script]}
    (project / ".home").mkdir()
    req = root / "request.json"
    req.write_text(json.dumps(request))
    p = subprocess.run([str(args.helper.resolve()), str(project), str(req)],
                       capture_output=True, text=True, timeout=900)
    lines = p.stdout.splitlines()
    result = {
        "pixi": version, "pixi_sha256": sha(binary), "platform": plat,
        "measured_at": time.strftime("%Y-%m-%d"),
        "lock_version": (project / "pixi.lock").read_text().splitlines()[0],
        "lock_packages": sum(1 for l in (project / "pixi.lock").read_text().splitlines()
                             if l.startswith("- conda:")),
        "install_seconds": install_seconds,
        "inputs_unchanged": inputs_unchanged,
        "hook_residue_outside_prefix": residue,
        "libraries_from_outside_prefix_bin_closure": outside,
        "libraries_from_outside_prefix_all_files": outside_all,
        "shell": shells,
        "host_grants": host_grants,
        "helper_exit": p.returncode,
        "stdout": lines,
        "stderr_tail": p.stderr.strip().splitlines()[-15:],
        "passed": {
            "numpy": any(l.startswith("numpy-ok 45 True") for l in lines),
            "go_build": "go-build-ok" in lines,
            "c_build": ("c-build-ok" in lines) if plat.startswith("linux") else None,
            "host_tools_absent": not any(l.startswith(("resolves:", "ran:")) for l in lines),
            "prefix_read_only": "prefix-write-denied" in lines,
        },
    }
    text = json.dumps(result, indent=2).replace(str(root), "<root>")
    Path(args.output).write_text(text + "\n")
    print(text)


if __name__ == "__main__":
    main()
