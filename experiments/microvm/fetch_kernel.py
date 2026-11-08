#!/usr/bin/env python3
"""Fetch hvi CI's public ARM64 kernel as a diagnostic Stage-A fixture.

An initial --resolve records the OCI manifest and layer digest. Later invocations
use only that recorded digest. This is NOT the purpose-built qualifying kernel
of task 1.3: its complete configuration has not been qualified for devcroft.
"""
import argparse
import hashlib
import json
from pathlib import Path
import urllib.request

ROOT = Path(__file__).resolve().parent
REPO = "nofireai/urunc-kernel"
BASE = f"https://ghcr.io/v2/{REPO}"
PIN = ROOT / "fixtures/kernel-aarch64.json"
LIMIT = 128 * 1024 * 1024


def fetch(url, headers=None, limit=LIMIT):
    with urllib.request.urlopen(urllib.request.Request(url, headers=headers or {}), timeout=60) as r:
        data = r.read(limit + 1)
    if len(data) > limit:
        raise ValueError("download exceeds size limit")
    return data


def verify(data, expected):
    actual = "sha256:" + hashlib.sha256(data).hexdigest()
    if actual != expected:
        raise ValueError(f"digest mismatch: expected {expected}, got {actual}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--resolve", action="store_true", help="record first pin; refuses to overwrite")
    parser.add_argument("--out", type=Path, default=ROOT / "artifacts/kernel/Image")
    args = parser.parse_args()
    if args.resolve and PIN.exists():
        parser.error("kernel pin already exists; use the recorded digest")
    if not args.resolve and not PIN.exists():
        parser.error("first fetch requires an explicit --resolve")
    token_url = f"https://ghcr.io/token?scope=repository:{REPO}:pull&service=ghcr.io"
    token = json.loads(fetch(token_url, limit=65536))["token"]
    headers = {"Authorization": f"Bearer {token}",
               "Accept": "application/vnd.oci.image.manifest.v1+json"}
    if args.resolve:
        manifest = fetch(f"{BASE}/manifests/minimal", headers, limit=1048576)
        parsed = json.loads(manifest)
        layers = {layer.get("annotations", {}).get("org.opencontainers.image.title"): layer
                  for layer in parsed.get("layers", [])}
        if set(layers) != {"Image", "config", "KVER"}:
            raise ValueError("expected Image/config/KVER layers; review changed artifact layout")
        pin = {"schema": 1, "repository": REPO, "resolved_tag": "minimal",
               "manifest_digest": "sha256:" + hashlib.sha256(manifest).hexdigest(),
               "kernel_digest": layers["Image"]["digest"],
               "config_digest": layers["config"]["digest"],
               "version_digest": layers["KVER"]["digest"],
               "qualification": "diagnostic-only; kernel configuration not qualified"}
    else:
        pin = json.loads(PIN.read_text())
        manifest = fetch(f"{BASE}/manifests/{pin['manifest_digest']}", headers, limit=1048576)
        verify(manifest, pin["manifest_digest"])
        if json.loads(manifest)["layers"][0]["digest"] != pin["kernel_digest"]:
            raise ValueError("recorded kernel/manifest mismatch")
    kernel = fetch(f"{BASE}/blobs/{pin['kernel_digest']}", headers)
    verify(kernel, pin["kernel_digest"])
    if len(kernel) < 64 or kernel[56:60] != b"ARM\x64":
        raise ValueError("expected an uncompressed Linux ARM64 Image")
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_bytes(kernel)
    for key, name in [("config_digest", "config"), ("version_digest", "KVER")]:
        data = fetch(f"{BASE}/blobs/{pin[key]}", headers, limit=1048576)
        verify(data, pin[key])
        (args.out.parent / name).write_bytes(data)
    if args.resolve:
        PIN.parent.mkdir(parents=True, exist_ok=True)
        with PIN.open("x") as f:
            f.write(json.dumps(pin, indent=2) + "\n")
    print(json.dumps({"kernel": str(args.out.resolve()), **pin}, indent=2))


if __name__ == "__main__":
    main()
