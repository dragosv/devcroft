# Pin the embedded base runtime in an immutable content-addressed object

## Why

`runtime::materialize` (`src/runtime/mod.rs`) currently writes `<project>/.devcroft/<name>/runtime/{sh,ls,cat,...}` as symlinks pointing directly at `current_exe()` on the host. If the user upgrades `devcroft` on the host (`cargo install` or package upgrade replacing `~/.cargo/bin/devcroft` or `/usr/local/bin/devcroft`) while sandboxes are running or stopped, existing sandboxes silently begin executing the new binary's shell and applets even though `Meta` and `policy --render` still report the generation recorded at `up`. In addition, future cross-OS microVM sandboxes (macOS host driving a Linux guest) require a target-addressed runtime artifact seam rather than assuming `current_exe()` matches the sandbox target ABI.

## What Changes

- Materialize the embedded runtime binary once per `(generation, target, executable_sha256)` into an immutable read-only object under `~/.local/share/devcroft-artifacts/runtime/<object_id>/` using `provider::artifacts` (shared lock, atomic publication receipt, lease during `up`, sweep on `rm` when unreferenced by any recorded sandbox `Meta`).
- Point `<project>/.devcroft/<name>/runtime/*` symlinks (and the Linux mount view / macOS exec-redirect helper where applicable) at the immutable runtime object instead of mutable `current_exe()`, recording `runtime_object_id` in `Meta`.
- Grant the immutable runtime object read-only with origin `runtime:gen-<N>` in `policy --render`, replacing the mutable host `current_exe()` path grant for session runtime utilities and `sh`.
- Introduce a target-explicit `RuntimeArtifact` / `RuntimePlan` descriptor (`generation`, `target`, `object_id`, `binary_path`) so native sandboxes and future Linux microVM guests consume one abstraction.

## Capabilities

### Modified Capabilities

- `base-runtime` pins a sandbox's runtime applets and `sh` to an immutable content-addressed artifact object unaffected by host `devcroft` upgrades until `--recreate`.
- `policy` attributes the read-only runtime object (and macOS redirect dylib if materialized alongside it) to `runtime:gen-<N>` without granting a mutable host binary path for userland execution.

## Impact

Affects `src/runtime/{mod,namespace,redirect}.rs`, `src/provider/artifacts.rs`, `src/lifecycle/{up,state}.rs`, `src/fleet/mount.rs` and `tests/`. Existing `[runtime] generation` pinning in `devcroft.toml` is preserved and strengthened: live host upgrades can no longer swap the bytes backing a running sandbox's runtime.
