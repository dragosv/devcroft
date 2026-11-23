## 0. Preconditions and measurement

- [x] 0.1 `add-runtime-provenance` is implemented; its provider-row test is the witness every task below updates explicitly.
  - Implemented and passing on Linux (nix, flox, devbox rows; 2026-11-22). Its macOS runs (its tasks 3.1, 3.3, 3.4, 5.1) are still open; this change's Linux work does not depend on them.
- [x] 0.2 Commit the spike evidence this change cites (`spikes/brush-uutils-native/` from the owner's Linux host), selectively, or record in `design.md` that it is unavailable.
  - Unavailable: not in this checkout or on this host (recorded in `design.md`).
- [x] 0.3 Measure the baseline: stripped size of `devcroft` built from this revision, and the shipped-dependency inventory as `scripts/gen-third-party-licenses.py` counts it (four targets).
  - `37c605d`, aarch64 Linux, `cargo build --release`: 35,692,272 bytes, 25,555,840 stripped; 381 shipped dependencies (recorded in `design.md`).

## 1. Runtime core

- [x] 1.1 `src/runtime/`: the inventory (names, upstream crate, version), the generation constant, and the published `docs/base-runtime.md` (extends the page from `add-runtime-provenance`).
- [x] 1.2 Promote the 39 coreutils `uu_*` crates and `uucore` to `[dependencies]` at one pinned generation; keep the packaging allowlist and add no `[[bin]]`.
  - All at `=0.13.0` (current on crates.io 2026-11-22). The three `uu_*` 0.6.0 dev-dependencies of `examples/test-row-sh.rs` collided by name and were dropped; the example uses the shipped crates. `cargo package --list` gains only `src/runtime/applets.rs`.
- [x] 1.3 `argv[0]` dispatch as the first statement of `main` in `src/bin/devcroft.rs`, through the upstream wrapper of the pinned version; unknown basenames fall through. `tests/cli_help_and_version.rs` still passes unchanged.
- [x] 1.4 Tests: dispatch per applet; `devcroft ls` is an unknown command; `ls | head` dies by `SIGPIPE` without a panic; `cat` on a missing file exits 1 with a translated message (not a translation key); `--help` text is real text.
  - `tests/runtime_applets.rs`. `test --version` is true by POSIX (GNU reads it only as `[`), so `test` is checked by status. `ls | head` needs more than a pipe buffer of output to see the closed pipe: with 5000 short names `ls` exits 0 (measured), with 20000 long ones it dies by `SIGPIPE`, as GNU `ls` does.

## 2. Generation and config

- [x] 2.1 `[runtime] generation` in `src/config/mod.rs`, `deny_unknown_fields`; validation errors at layer `config`, exit 2.
- [x] 2.2 `up` refuses a mismatch naming both generations; prints the pin line when absent; records the generation in `Meta`.
- [x] 2.3 `status` shows the sandbox's generation and the installed binary's; a mismatch with a live keeper says `up --recreate`.
- [x] 2.4 Tests for each of the three cases.

## 3. PATH and the common layer

- [x] 3.1 Materialise the runtime directory at `up` (links to the `EXE_LINK` binary, beside it under `<project>/.devcroft/<name>/`), idempotent like `link_keeper_exe`; read grant with origin `runtime:gen-N`.
  - Links name the keeper binary directly, not `EXE_LINK`: that link sits in the artifact directory, which a view contains only when the manifest grants `.`.
- [ ] 3.2 Generalise `path_ending_in_shell_dir` into the shared `PATH` builder: provider `PATH`, shell dir, runtime dir; drop the canonical host tail. Use it from `up` and `fleet::project::prepare`.
  - `up`: done (`sandbox_path`, `project_path`); one value feeds both the record and the keeper. "Host tail" is implemented as "entries outside every resolved grant", which keeps swift's declared `/usr/bin`. Fleet: with 4.3, when agents get a runtime directory.
- [ ] 3.3 The common runtime layer after provider resolution; move the shell fallback out of `mise.rs` and `pixi.rs` into it; host shell only as one file grant, recorded `host:` and warned.
  - Implemented (`src/runtime/layer.rs`): `up` takes the shell through `resolve_shell`; mise and pixi no longer carry a fallback (pixi keeps linking its own `bash` as `sh`). The host shell is a link `.devcroft/<name>/host-shell/sh`, so `PATH` gains `sh` alone, never the host directory it sits in; its grants carry their own origin `host-shell` (recorded in `Meta.host_shell_grants`, reconstructed by `--render`). Linux: the shell file plus the glibc files its headers name; macOS: `/bin` + `/var/select` as before (D5).
  - Measured on macOS 2026-11-23: `mise_provider_e2e` and `pixi_provider_e2e` pass with no skips; mise's render shows `host-shell`, `sh (by name)` is `host:`, and `PATH` holds the link directory and no host directory. **Open:** the Linux run (the library walk is unmeasured), and `fleet::project::prepare`, which still refuses a shell-less environment — fleet refuses mise and pixi anyway, so it moves with 4.3.
- [x] 3.4 `native` declaration on the provider seam with a reason string; swift declares it; `up`, `status`, `policy --render` show it; provenance `native:swift`.
  - `ProviderKind::native` (`SWIFT_NATIVE_REASON`) and a defaulted `ProviderEntry::native` (forgetting it fails safe: `host:` and warned). `Provenance::Native`; the reason travels in the record for `--render`. Measured on macOS 2026-11-22 (`swift_provider_e2e`, no skips): all three surfaces state it, `/bin/sh` renders `native:swift:`, no per-name host warning; the nix control prints no native line.
- [x] 3.5 Refuse a manifest grant of an individual host developer tool made to obtain a base name in a non-native sandbox, naming native mode.
  - `ConfigError::HostToolGrant`, layer `config`: a `filesystem.allow`/`read` entry that is a base name directly under `/bin`, `/usr/bin`, `/sbin`, `/usr/sbin`, `/usr/local/bin` or `/opt/homebrew/bin`, unless the provider is native. Lexical; no sample or test manifest granted one.
- [ ] 3.6 Update the provenance test's expectations: closure providers' utilities by name now `closure:` or `runtime:gen-N`, never `absent`; mise/pixi utilities `runtime:gen-N`, shell still `host:`.
  - Linux, measured 2026-11-22 (in-sandbox `-ef` probe, 0 mismatches each): nix/devbox rows `hostname` → runtime, everything else closure; flox row the same plus grep…gzip absent; flox-services-sample (no coreutils) 39 runtime (was 0); pixi 39 runtime, `sh` host; mise 38 runtime, `sh` and `env` host — its fallback puts the host's `/usr/bin` on `PATH` as the shell's directory (pre-existing, 3.3's job).

## 4. The namespace (rule B) and the Linux view

- [ ] 4.1 Compute the target of each base name once at `up` (provider `PATH` within grants → shell dir → runtime) and write the namespace directory (`<project>/.devcroft/<name>/ns/`), idempotently; share the function with `fleet::project::prepare`.
  - `up`: done (`runtime::namespace`; macOS). **Open:** fleet, which has no runtime directory until 4.3. Runtime, namespace and host-shell directories are now written through `runtime::owned_dir` (no component may be a link): the by-path writer followed a link planted by the sandbox and deleted the target's files (measured 2026-11-23). Other by-path writers under `.devcroft/<name>/` (`link_keeper_exe`, the policy artifact) were not audited.
- [ ] 4.2 View construction: tmpfs `/bin` and `/usr/bin` populated from the namespace directory, one bind of `devcroft` for runtime targets; `Identity`'s `/bin/sh` and `/usr/bin/env` become namespace entries.
- [ ] 4.3 Fleet agent and provisioning views get the same entries (today their `identity` is `None`).
- [ ] 4.4 Refuse at `up` where the view cannot be constructed; no partial namespace.
- [ ] 4.4a Runtime and namespace directories read-only from inside: read-only mounts in the Linux view, a write denial in the macOS profile that outranks the project grant; shown by `policy --render`; tests overwrite and remove from a session and fail. (`add-sandbox-identity` extends this to all of `.devcroft/<name>/` except `home/`.)
- [ ] 4.5 Consistency test on every provider row: for each base name, `/bin/<name>`, `/usr/bin/<name>` and `<name>` execute the same file, or all fail. Nix and devbox rows: GNU from the closure. Flox minimal row: runtime.
- [ ] 4.6 Tests: `#!/usr/bin/env python3` in a flox sandbox without coreutils runs through the runtime's `env`; in a nix sandbox through the closure's; `/usr/bin/python3` fails; `/bin/awk` fails in flox without awk; the same from a fleet agent; `policy --render` lists each entry with target and provenance.

## 5. Inventory growth

- [ ] 5.1 Build the option corpus: commands and flags used by the eleven agent samples' typical tool calls and by the hooks of the qualified providers' samples.
  - Hooks half done (`design.md` D6): only coreutils-floor names, no `grep`/`sed`/`find`/`xargs`/`diff`. Agent half open: not in the repository, needs one logged run per agent with its credentials (owner).
- [ ] 5.2 Qualify `findutils`, `uu_grep`, `sed` (and decide on `diffutils`) against it; check each exposes an entry usable through the multicall wrapper; record results in `design.md`.
- [ ] 5.3 Owner decision per family; admitted families bump the generation.
- [ ] 5.4 Re-measure each `devenv-<agent>-sample` without its manual userland declarations; drop the ones the runtime covers, keep the rest with a reason.

## 6. Cost, attribution, documentation

- [ ] 6.1 Measure the integrated binary (standard and `release-small`), build time, and the shipped-dependency delta with the generator's own definition; record in `design.md`.
- [ ] 6.2 Regenerate `THIRD-PARTY-LICENSES.md`; make an MIT entry without a vendored text fail the generator.
- [ ] 6.3 `docs/known-gaps.md`: uutils ≠ GNU where the provider supplies nothing; awk and tar not in the runtime; the Linux `/usr/bin/env` gap closed.
  - uutils ≠ GNU and the missing families/shell: written (2026-11-23). The `/usr/bin/env` entry says what is left; it is marked closed with 4.2.
- [x] 6.4 `docs/decisions.md`: extend "devcroft's shell is devcroft's dependency" to the runtime, with the project-pin rule and the native exception.
  - "Decided: shell and userland are devcroft's dependency, pinned by the project" (§1), and the mise entry names the common layer. `docs/base-runtime.md` also gained the `runtime:`/`native:` kinds, the host-shell layer and native mode.
- [x] 6.5 `CLAUDE.md`: the `runtime:gen-N` and `native:<provider>` origins in the policy invariant list.

## 7. Verification

- [ ] 7.1 `cargo build`, `fmt`, `clippy`, `doc --no-deps` (zero rustdoc warnings), `test`, with the skip inventory recorded.
- [ ] 7.2 `openspec validate add-own-userland --type change`.
