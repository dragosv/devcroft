#!/usr/bin/env python3
"""Regenerate THIRD-PARTY-LICENSES.md from the current dependency tree.

Why this exists rather than `cargo about`: devcroft links hundreds of Apache-2.0
crates (nono, russh and the whole sigstore family among them), and
Apache-2.0 section 4 requires retaining license texts and notices when the
work is redistributed. A statically-linked `devcroft` binary redistributes
all of them, so a release needs this file. Generating it with a committed
script rather than an installed tool keeps the output reviewable, keeps
regeneration reproducible for anyone with a checkout, and adds no build
dependency.

Scope: **normal dependencies only**. `cargo tree -e normal` excludes
dev- and build-dependencies, which is correct here — neither is present
in a shipped binary, so neither carries a redistribution obligation.

Run from the repository root:

    python3 scripts/gen-third-party-licenses.py

and commit the result. Re-run whenever `Cargo.lock` changes.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

# Filenames crates conventionally use. Ordered: an explicit per-license
# file beats a combined one, so an Apache-2.0-only crate is attributed
# with the Apache text rather than whatever `LICENSE` happens to hold.
LICENSE_FILENAMES = [
    "LICENSE-APACHE",
    "LICENSE-MIT",
    "LICENSE.md",
    "LICENSE.txt",
    "LICENSE",
    "COPYING",
    "UNLICENSE",
    "LICENSE-CC0",
    "NOTICE",
]

# Crates that ship no license file, with the text their own repository
# publishes at the revision the crate was packaged from (`.cargo_vcs_info.json`),
# in scripts/license-texts/. Keyed by exact version, so an upgrade fails until
# someone has looked again.
SUPPLIED = {
    ("base64-simd", "0.8.0"): "nugine-simd.txt",
    ("vsimd", "0.8.0"): "nugine-simd.txt",
    ("tonic-prost", "0.14.9"): "grpc-rust.txt",
    ("objc2", "0.6.5"): "objc2.txt",
    ("objc2-encode", "4.1.0"): "objc2.txt",
    ("block2", "0.6.2"): "objc2.txt",
}

# Some texts are byte-identical across hundreds of crates (the Apache-2.0
# boilerplate especially). Emitting each one would produce a multi-megabyte
# file nobody reads, so identical texts are pooled and referenced.
def sha(text: str) -> str:
    import hashlib

    return hashlib.sha256(text.encode("utf-8", "replace")).hexdigest()[:16]


# Every target devcroft ships for. `cargo tree` resolves for the host alone
# unless told otherwise, so the file used to depend on where it was
# generated: a Mac dropped the Linux keyring/dbus tail (48 crates), and an
# aarch64 Linux host dropped `curve25519-dalek-derive`, which only
# x86_64's SIMD backend uses. The union is the same file everywhere.
TARGETS = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
]


def shipped_packages() -> set[str]:
    """`name vX.Y.Z` ids for normal (non-dev, non-build) dependencies."""
    targets = [arg for t in TARGETS for arg in ("--target", t)]
    out = subprocess.run(
        ["cargo", "tree", "-e", "normal", *targets, "--prefix", "none", "--format", "{p}"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    ids = set()
    for line in out.splitlines():
        line = line.strip()
        if not line or line.startswith("["):
            continue
        # Lines look like "name v1.2.3" or "name v1.2.3 (*)" or
        # "name v1.2.3 (/local/path)". Keep name+version only.
        parts = line.split()
        if len(parts) >= 2 and parts[1].startswith("v"):
            ids.add(f"{parts[0]} {parts[1]}")
    return ids


def main() -> int:
    root = Path(__file__).resolve().parent.parent
    meta = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1"],
            capture_output=True,
            text=True,
            check=True,
            cwd=root,
        ).stdout
    )

    shipped = shipped_packages()
    texts: dict[str, str] = {}          # hash -> license text
    entries: list[tuple[str, str, str, str | None]] = []  # name, ver, spdx, hash
    missing: list[tuple[str, str, str]] = []
    substituted: list[tuple[str, str, str]] = []

    for pkg in meta["packages"]:
        name, version = pkg["name"], pkg["version"]
        if name == "devcroft":
            continue
        if f"{name} v{version}" not in shipped:
            continue

        spdx = pkg.get("license") or "(not declared)"
        manifest_dir = Path(pkg["manifest_path"]).parent

        found_hash = None
        # By listing, so the match ignores case on every host: opening the
        # name found `LICENSE-Apache` on a Mac's filesystem and not on Linux.
        present = {f.name.upper(): f for f in sorted(manifest_dir.iterdir())}
        for filename in LICENSE_FILENAMES:
            candidate = present.get(filename.upper())
            if candidate is not None and candidate.is_file():
                text = candidate.read_text(encoding="utf-8", errors="replace")
                h = sha(text)
                texts.setdefault(h, text)
                found_hash = h
                break

        if found_hash is None and (name, version) in SUPPLIED:
            supplied = root / "scripts" / "license-texts" / SUPPLIED[(name, version)]
            text = supplied.read_text(encoding="utf-8")
            h = sha(text)
            texts.setdefault(h, text)
            found_hash = h

        if found_hash is None and "Apache-2.0" in spdx:
            # The crates that most need attribution turned out to be
            # exactly the ones that vendor no license file: nono, russh
            # and the whole sigstore family are Apache-2.0-only and ship
            # no text. Apache-2.0 section 4(a) requires giving recipients
            # "a copy of this License", so listing them as absent would
            # leave the obligation undischarged. The Apache-2.0 text is
            # invariant boilerplate — unlike MIT, it carries no
            # per-holder copyright line — so substituting the canonical
            # copy is exact, not an approximation. For a dual-licensed
            # crate this amounts to taking the Apache-2.0 option, which
            # the license grant explicitly permits.
            canonical = root / "LICENSE-APACHE"
            text = canonical.read_text(encoding="utf-8", errors="replace")
            h = sha(text)
            texts.setdefault(h, text)
            found_hash = h
            substituted.append((name, version, spdx))
        elif found_hash is None:
            # Non-Apache licenses are not substituted: MIT and BSD texts embed
            # a copyright holder, so a canonical copy would attribute the
            # wrong party. Fatal below.
            missing.append((name, version, spdx))
        entries.append((name, version, spdx, found_hash))

    if missing:
        print(
            "no license text for:\n"
            + "".join(f"  {n} {v} ({l})\n" for n, v, l in sorted(missing))
            + "add each crate's own text to SUPPLIED and scripts/license-texts/",
            file=sys.stderr,
        )
        return 1

    entries.sort(key=lambda e: (e[0].lower(), e[1]))

    lines: list[str] = []
    lines.append("# Third-party licenses")
    lines.append("")
    lines.append(
        "devcroft is distributed under Apache-2.0 (see `LICENSE-APACHE` and "
        "`NOTICE`). A compiled `devcroft` binary statically links the "
        "dependencies below, whose own licenses are reproduced here."
    )
    lines.append("")
    lines.append(
        "**Generated file — do not edit by hand.** Regenerate with "
        "`python3 scripts/gen-third-party-licenses.py` after any `Cargo.lock` "
        "change. Scope is normal dependencies only; dev- and "
        "build-dependencies are excluded because they are not present in a "
        "shipped binary."
    )
    lines.append("")
    lines.append(f"{len(entries)} dependencies, {len(texts)} distinct license texts.")
    lines.append("")
    lines.append("## Dependencies")
    lines.append("")
    lines.append("| Crate | Version | License (SPDX) | Text |")
    lines.append("|---|---|---|---|")
    for name, version, spdx, h in entries:
        ref = f"[{h}](#license-text-{h})"
        lines.append(f"| `{name}` | {version} | {spdx} | {ref} |")
    lines.append("")

    if substituted:
        lines.append("## Dependencies attributed with the canonical Apache-2.0 text")
        lines.append("")
        lines.append(
            "These declare Apache-2.0 but ship no license file in the published "
            "crate. Apache-2.0 section 4(a) requires that recipients receive a "
            "copy of the License, so the canonical text is supplied above on "
            "their behalf. The Apache-2.0 text carries no per-holder copyright "
            "line, so this is exact rather than approximate."
        )
        lines.append("")
        for name, version, spdx in sorted(substituted, key=lambda m: m[0].lower()):
            lines.append(f"- `{name}` {version} — {spdx}")
        lines.append("")

    lines.append("## License texts")
    lines.append("")
    for h in sorted(texts):
        users = [f"`{n}`" for n, _, _, hh in entries if hh == h]
        lines.append(f"### License text {h}")
        lines.append("")
        shown = ", ".join(users[:12])
        if len(users) > 12:
            shown += f", and {len(users) - 12} more"
        lines.append(f"Applies to: {shown}")
        lines.append("")
        lines.append("```")
        lines.append(texts[h].rstrip())
        lines.append("```")
        lines.append("")

    out = root / "THIRD-PARTY-LICENSES.md"
    out.write_text("\n".join(lines), encoding="utf-8")
    print(
        f"wrote {out.relative_to(root)}: {len(entries)} dependencies, "
        f"{len(texts)} distinct texts, {len(substituted)} Apache-2.0 substituted"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
