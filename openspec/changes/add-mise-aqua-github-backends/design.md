## Context

`add-mise-provider`'s sentinel probes on `macos-arm64` and `linux-arm64` (`mise` 2026.11.9) already demonstrated that `aqua:`, `github:` and `conda:` installations leave marker-writing payloads and Conda `post-link` scripts unexecuted, whereas `core:node`, `core:go` and `core:python` unconditionally execute the downloaded runtime on the host. `src/provider/mise.rs` currently hardcodes `name.starts_with("http:")` and requires `{ version, url }` in `mise.toml`, and `diagnose()` still prints `Path::new("/bin/sh").canonicalize()` even though `embed-brush-runtime` (`GENERATION = 2`) removed all host-shell fallback.

## Measurement gate (Group 0)

Before admitting each backend in production code, measure on pinned `mise` (`>= 2026.11.9`) across `macos-arm64` and `linux-aarch64`:
1. Exact `mise.lock` v4 structure written by `mise lock` for `aqua:`, `github:`, bare registry shorthands (`ripgrep`, `process-compose`) and `conda:` (including whether `platforms.<target>` is emitted as a quoted dotted key or nested table, and where `conda:` records transitive dependency URLs/checksums, e.g. `mise.lock` vs `.mise/locks/`).
2. Whether `mise install --locked` under `isolated_env` attempts external `cosign`, `slsa-verifier` or `gh` invocations or GitHub API calls when URLs and `sha256:` checksums are already present in `mise.lock`, and which `MISE_*` settings keep locked restore offline-capable from the locked artifact URLs alone without weakening SHA-256 verification.
3. Whether `mise install --locked` leaves `mise.lock` (and any admitted lock sidecar) byte-identical on cold restore. Any backend or option that rewrites locks or executes downloaded binaries stays refused.

## Decisions

### M1. Backend admission is decided from both `mise.toml` and `mise.lock`

`Input::load` accepts `[tools]` entries in string form (`ripgrep = "14.1.1"`, `"aqua:F1bonacc1/process-compose" = "1.90.0"`) or table form (`{ version = "..." }` plus only explicitly qualified non-executable options such as bounded relative `exe`/`bin_path` free of `..`, leading `/` and templates; `http:` continues to accept literal `url`). Options that run code or alter install environments (`postinstall`, `install_env`, `os` mismatches) fail before any subprocess.

In `check_lock`, devcroft resolves each `[tools]` key's matching `mise.lock` entry (supporting both quoted `"platforms.<target>"` keys and nested `platforms.<target>` tables):
- Explicit prefixes in `mise.toml` must match the `backend` prefix in `mise.lock`.
- Bare registry names in `mise.toml` are admitted **only** when `mise.lock` records an admitted `backend` (`aqua:*`, `github:*`, `conda:*`, `http:*`). A bare name such as `node` or `python` whose lock resolves to `core:node` or `core:python` fails with a remedy naming `conda:nodejs` / `conda:python` (or an explicit `http:` archive).
- Every artifact URL for the current target (including any locked `conda:` dependency set) must be `https://` without `@` or `#` and carry a 64-hex `sha256:` checksum.
- For `conda:`, every locked URL host must be in `["conda.anaconda.org", "repo.prefix.dev"]`, matching `src/provider/pixi.rs` so macOS keychain credentials cannot be sent to private channels.

### M2. Derived manifest and `ADAPTER` version bump

The derived directory passed to `mise install --locked` and `mise env --json` carries `[tools]`, `[settings]` (`locked = true`, `lockfile = true`, plus the Group 0 settings that disable external host verifier binaries while enforcing lockfile checksums), `mise.lock` and any qualified read-only lock sidecars, all hashed into the artifact object ID under a bumped `ADAPTER` constant (`mise-locked-3`). After `mise install --locked`, devcroft verifies `mise.lock` (and any sidecar) is byte-identical before publishing `complete.sha256`.

### M3. Runtime library walk and `doctor` shell alignment

`artifacts::header_dependencies` continues to derive minimal read-only host loader/libc grants (`provider:mise`) and warns (without granting) on non-baseline host libraries, matching `pixi`. `diagnose()` resolves the shell story without touching host `/bin/sh`: it reports an environment-provided shell when already materialized, or devcroft's embedded runtime shell (`brush`, `runtime:gen-2`) otherwise.
