## Context

`src/runtime/mod.rs:47-55` creates `<project>/.devcroft/<name>/runtime/<applet>` symlinks pointing directly to `exe` (`current_exe()`), and `up` grants that host binary path so sessions can exec `sh`, `env`, `ls`, etc. On macOS (and on Linux whenever an installer atomically replaces the binary path via `rename`), upgrading `devcroft` while a sandbox is running causes every subsequent `sh` or applet invocation inside that sandbox to run the new binary, violating `Meta`'s recorded `runtime:gen-<N>` provenance and risking ABI or generation mismatches mid-session. Furthermore, on macOS hosts driving a Linux microVM guest, `current_exe()` is Mach-O and cannot serve the guest's `RuntimePlan`.

## Decisions

### R1. Content-addressed runtime store via `provider::artifacts`

Store runtime objects under `~/.local/share/devcroft-artifacts/runtime/<id>/`, where `<id>` hashes `("runtime-v1", GENERATION, target_triple, sha256(current_exe_bytes), optional_redirect_dylib_sha256)`. Under `artifacts::lock`, copy `current_exe()` (and on macOS the exec-redirect dylib if embedded/built beside it) to staging with `0755`/`0644` permissions, write `complete.sha256` atomically, and verify the digest on reuse. `up` acquires the `runtime` store lease before sweeping or recording `Meta.runtime_object_id`; `rm` and `up --recreate` sweep `runtime` objects referenced by no recorded `Meta`.

### R2. `RuntimePlan` decouples target ABI from `current_exe()`

Define `RuntimePlan { generation: u32, target: String, object_id: String, binary: PathBuf, redirect_lib: Option<PathBuf> }`. Native `up` resolves `RuntimePlan` for the host target from the current binary (or from `Meta.runtime_object_id` on plain `up` recovery when the object still exists). `<project>/.devcroft/<name>/runtime/{sh,env,ls,...}` symlinks target `plan.binary` inside the immutable store object rather than `current_exe()`.

### R3. Policy grants and `artifact_store_write_overlap`

Grant `~/.local/share/devcroft-artifacts/runtime/<id>` read-only (with `process-exec*` on macOS via `scope_exec_to_read_grants`) under origin `runtime:gen-<N>`, plus any host dynamic-loader/libc dependencies returned by `artifacts::header_dependencies(&plan.binary)`. Because the object sits under `~/.local/share/devcroft-artifacts/`, `policy::artifact_store_write_overlap` automatically refuses any manifest write grant overlapping it.
