# Separate Helper & Guest Artifact Distribution Channel (`add-vm-project-runtime` Task 1.4)

This document defines the distribution, licensing, signing, and dependency boundaries for the opt-in macOS ARM64 `libkrun` runtime (`devcroft-vm-helper` and its verified Linux guest artifacts).

## 1. Separation from crates.io (`devcroft`)

- **Default crate (`Cargo.toml`) stays crates.io-publishable and Apache-2.0:**
  - `cargo install devcroft` / `crates.io` contains **zero** Git VMM dependencies (`libkrun` is never added to root `Cargo.toml` or `Cargo.lock`).
  - `src/vm/` defines pure-Rust qualification, launch/handshake/state schemas, trusted-manifest validation, and user-owned runtime grants (`0` external crates added).
- **Isolated helper workspace:**
  - `devcroft-vm-helper` (successor to `experiments/microvm/libkrun`) builds in its own Cargo workspace with its own `Cargo.lock` against the pinned `libkrun` revision (`6151710f4ea627cf0e5dbce78e4192fb4d64c2e9`) and the four SHA-256-verified local patches recorded in `experiments/microvm/fixtures/upstream.json`:
    1. `libkrun-vsock-host-half-close.patch`
    2. `libkrun-macos-reset-exit.patch`
    3. `libkrun-vsock-host-backpressure.patch`
    4. `libkrun-vsock-hardening.patch`
- **Dedicated artifact channel:**
  - Helper binaries and guest artifacts (Linux kernel, initramfs, base runtime ext4 image, preparation VM image, license inventory) are delivered through a signed release channel indexed by `TrustedArtifactManifestV1` (`src/vm/manifest.rs`) verified by SHA-256 + Ed25519 signature under user-trusted storage (`~/.local/share/devcroft/vm-artifacts/<manifest_version>/`), never from repository-controlled paths.

## 2. Licence Inventory & GPL Source Obligations

`scripts/gen-third-party-licenses.py` inspects only Rust/Cargo dependencies (`cargo metadata`) and does **not** cover distributed guest binaries. Therefore every VM artifact release ships a mandatory companion inventory:

- **`licenses/vm-artifact-licenses.json`** (listed in `TrustedArtifactsV1::license_inventory`, hashed and signed identically to code artifacts):
  - **Linux kernel (`vmlinux-6.12.95`, `GPL-2.0-only`):** exact upstream tarball URL, SHA-256, kernel `.config`, and written-offer / corresponding-source archive reference.
  - **Guest userland & images (`Alpine`, `BusyBox`, `OpenSSH`, `Nix` closure tools, `runtime-base.img`, `prep-base.img`):** package-by-package SPDX licence list and upstream source/pin receipts (`experiments/microvm/fixtures/*-packages-*.json`, `nix-aarch64-closure.json`).
  - **`libkrun` 2.0 & `devcroft-vm-helper` (`Apache-2.0`):** pinned upstream commit + patch inventory.
- `scripts/gen-third-party-licenses.py` remains dedicated to the Rust crate; VM artifact compliance is checked separately against `vm-artifact-licenses.json` before release publication.

## 3. macOS Entitlement & Signing (`com.apple.security.hypervisor`)

On macOS ARM64 (`macOS 14+`), `Hypervisor.framework` fails (`0xfae94007`) without `com.apple.security.hypervisor` signed onto the helper executable:

- **`devcroft-vm-helper` entitlements:** minimal `entitlements.plist` containing `<key>com.apple.security.hypervisor</key><true/>` (no JIT, no debugging, no unrestricted filesystem exceptions).
- **Signing modes:**
  - Release artifacts: signed (Developer ID / hardened runtime with `com.apple.security.hypervisor`) alongside checksum verification in `TrustedArtifactManifestV1`.
  - Local development/verification builds: ad-hoc signed (`codesign -s - --entitlements ... --force`) as verified in `docs/decisions.md` (`0x0 SUCCESS`) because `cargo install` cannot run post-build `codesign`.
- **Pre-launch verification:** `PackagedLayout` refuses missing signatures/entitlements or mismatched helper SHA-256 before any guest boot; no silent fallback to native execution occurs.

## 4. Compatibility Window & Fail-Closed Versioning

- `TrustedArtifactManifestV1` bounds `min_compatible_controller..=max_compatible_controller` (`check_controller_compatible`) and `LaunchRecordV1`/`HandshakeV1` bind `schema = 1`, `instance_generation`, `protocol_version`, `target`, and `boot_artifact_sha256`.
- Unqualified platforms (`linux-arm64`, `linux-x86_64`, `macos-x86_64`) remain disabled in `src/vm/qualification.rs` until dedicated native runners pass E3/E4 and signed release manifests exist for those targets.
