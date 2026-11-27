## 1. Materialize immutable runtime objects

- [ ] 1.1 Add `runtime` store helpers over `src/provider/artifacts.rs` and define `RuntimePlan` in `src/runtime/mod.rs`, publishing `~/.local/share/devcroft-artifacts/runtime/<id>/` atomically with `complete.sha256` verification and `artifacts::header_dependencies` inspection.
- [ ] 1.2 Record `runtime_object_id` in `Meta`, hold the `runtime` store lease across `up`, and include `runtime` objects in `rm` / `up --recreate` garbage-collection sweeps when no sandbox `Meta` references them.

## 2. Wire symlinks, views and policy grants

- [ ] 2.1 Point `<project>/.devcroft/<name>/runtime/*` symlinks, Linux `RuntimeView` mounts (`src/fleet/mount.rs`) and macOS exec-redirect paths (`src/runtime/redirect.rs`) to `RuntimePlan` inside the immutable object instead of mutable `current_exe()`, granting the object read-only with origin `runtime:gen-<N>`.
- [ ] 2.2 Add integration tests confirming that replacing the outer `devcroft` binary path on disk while a sandbox is running leaves `sh` and coreutils inside the sandbox executing the pinned object recorded at `up`, that two sandboxes share one runtime object, and that `rm` of the last consumer sweeps it. Update `docs/base-runtime.md`.
- [ ] 4.1 Depend on `validate-before-recreate`’s `rm` critical section before adding `runtime` to `rm`’s sweep, so a concurrent `up` cannot lose its runtime object.
