#!/usr/bin/env python3
"""Build the pinned ARM64 guest kernel in a dedicated Linux VM without host mounts.

Lima is an experimental build prerequisite, not a devcroft runtime dependency.
The builder is stopped on success/failure; outputs and a build receipt are retained.
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
SOURCE_SHA = "a9e8c51fcb1e695d1d35dde5886cba579cb6f29c9646c5889f39d63841d4b9f6"
PACKAGES = ["build-base", "bc", "bison", "flex", "perl", "python3", "curl", "xz", "linux-headers"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lima-home", type=Path, default=Path("/private/tmp/devcroft-kernel-builder"))
    parser.add_argument("--name", default="kernel-build")
    parser.add_argument("--out", type=Path, default=ROOT / "artifacts/kernel-landlock")
    parser.add_argument("--collect-only", action="store_true", help="collect an already completed build and stop its VM")
    args = parser.parse_args()
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        parser.error("this builder profile requires macOS ARM64; no emulation fallback")
    env = dict(os.environ, LIMA_HOME=str(args.lima_home.resolve()))
    frozen = tempfile.TemporaryDirectory(prefix="devcroft-kernel-inputs-")
    fragment = Path(frozen.name) / "aarch64.config"
    recipe = Path(frozen.name) / "build.sh"
    fragment.write_bytes((ROOT / "kernel/aarch64.config").read_bytes())
    recipe.write_bytes((ROOT / "kernel/build.sh").read_bytes())

    def lima(*cmd, capture=False):
        call = ["limactl", *cmd]
        return (subprocess.check_output(call, env=env, text=True).strip() if capture
                else subprocess.run(call, env=env, check=True))

    try:
        if not args.collect_only:
            if (args.lima_home / args.name / "lima.yaml").exists():
                lima("start", "--tty=false", args.name)
            else:
                lima("start", "--tty=false", f"--name={args.name}", str(ROOT / "kernel/builder.yaml"))
            lima("shell", args.name, "sudo", "apk", "add", "--no-cache", *PACKAGES)
            if lima("shell", args.name, "stat", "-f", "-c", "%T", "/tmp", capture=True) != "tmpfs":
                lima("shell", args.name, "sudo", "mount", "-t", "tmpfs", "-o", "size=3G", "tmpfs", "/tmp")
            lima("copy", str(fragment), str(recipe), f"{args.name}:/tmp/")
            lima("shell", args.name, "sh", "/tmp/build.sh", "/tmp/aarch64.config")
        args.out.mkdir(parents=True, exist_ok=True)
        for file in ["Image", "config", "KVER", "SHA256SUMS", "toolchain.txt"]:
            lima("copy", f"{args.name}:/tmp/devcroft-kernel/output/{file}", str(args.out / file))
        packages = lima("shell", args.name, "apk", "info", "-v", capture=True)
        (args.out / "packages.txt").write_text(packages + "\n")
        built_config = (args.out / "config").read_text().splitlines()
        for required in ("CONFIG_SECURITY_LANDLOCK=y", "CONFIG_PROC_PAGE_MONITOR=y", "CONFIG_ADVISE_SYSCALLS=y", "CONFIG_INOTIFY_USER=y"):
            if required not in built_config:
                raise RuntimeError(f"output kernel lacks required capability: {required}")
        receipt = {"schema": 1, "kernel_version": (args.out / "KVER").read_text().strip(),
                   "source_url": "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.12.95.tar.xz",
                   "source_sha256": SOURCE_SHA,
                   "source_checksum_reference": "https://cdn.kernel.org/pub/linux/kernel/v6.x/sha256sums.asc",
                   "architecture": "aarch64", "builder": args.name,
                   "builder_image": "alpine-3.23.3-aarch64-uefi-cloudinit-r0",
                   "reproducibility": "source/config/output identities recorded; installed toolchain recorded, not a bit-reproducibility claim"}
        for key, file in [("kernel", args.out / "Image"), ("config", args.out / "config"),
                          ("config_fragment", fragment)]:
            receipt[f"{key}_sha256"] = hashlib.sha256(file.read_bytes()).hexdigest()
        (args.out / "build.json").write_text(json.dumps(receipt, indent=2) + "\n")
        print(json.dumps(receipt, indent=2))
    finally:
        if (args.lima_home / args.name / "ha.pid").exists():
            lima("stop", args.name)
        frozen.cleanup()


if __name__ == "__main__":
    main()
