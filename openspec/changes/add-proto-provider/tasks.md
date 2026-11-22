## 0. Measurement gate — no code until these are answered

Not scheduled: start only when the owner records demand. Record each result in `design.md` under "Evidence", tagged as measured, with the proto version and target, and the raw runs under `evidence/`.

- [ ] 0.1 Pin the candidate proto release. Confirm `install --immutable-lockfile`, `lock --platform` and `activate --json` exist on it. Name every directory and variable proto reads (`PROTO_HOME`, cache, temp, trust store, `~/.prototools`, parent-directory walk) and the switch, if any, that stops the walk.
- [ ] 0.2 Plugins: determine whether built-in plugin WASM is fixed by the proto release (bundled, or fetched at a pinned version with a checksum) and whether any locator form pins a version and a checksum. Record every network destination contacted during `install --immutable-lockfile`.
- [ ] 0.3 With sentinel archives (executables that write a marker when run), install Node, Bun, Deno, Go and Python under immutable mode on linux-aarch64 and osx-arm64. Record which execute anything downloaded, which call a host command from the plugin, which build from source, and whether `activate --json` runs anything or writes anything.
- [ ] 0.4 Measure the lock: which tools get a checksum from `proto lock` without installing; whether immutable install leaves `.protolock` byte-identical; whether installed trees are prefix-hardcoded under `PROTO_HOME`. On Linux and macOS under `own-policy-baseline`: the host-runtime files each admitted tool needs, `cc`/`git` unresolvable inside, and one real build from the environment alone.
- [ ] 0.5 Stop criteria. Stop and report to the owner, and go to group 6 instead of group 1, if any of these hold: built-in plugins cannot be pinned (0.2); fewer than two of Node, Go and Python are admissible on both platforms (0.3); immutable mode contacts a resolver or rewrites the lock (0.4); `activate --json` runs project code (0.3).

## 1. Selection and inputs

- [ ] 1.1 `src/provider/validate.rs`: move `"proto"` from `VERSION_MANAGERS` to `SUPPORTED`; add `ProviderKind::Proto` (`Tier::Artifact`) in `src/provider/mod.rs` with arms in `from_name`, `static_name`, `tier` and `resolution_reach`.
- [ ] 1.2 `src/config/{mod,validate}.rs`: optional `[env.proto]` with `config`; unknown keys, escaping paths and the table under another provider fail at layer `config`.
- [ ] 1.3 Classify `.prototools` per design P3: consumed, refused by key, not carried. Refuse unpinned plugin locators per P2.

## 2. Locked materialization

- [ ] 2.1 Parse `.protolock` against the qualified format; refuse missing records, missing checksums and unadmitted tools before any subprocess.
- [ ] 2.2 Build the derived configuration and the allowlisted environment; test that `~/.prototools`, `~/.proto` and `PROTO_*` change nothing.
- [ ] 2.3 Store at `~/.local/share/devcroft-artifacts/proto/<id>/` through `provider::artifacts`: per-object lock, removal of incomplete objects, `install --immutable-lockfile`, inputs verified unchanged, atomic completion record. Sweep at `rm` and `up --recreate`.

## 3. Capture, grants and staleness

- [ ] 3.1 Capture `activate --json`; apply `[env]` literals in devcroft; refuse `file`, `[shell]`, `[shell.aliases]`.
- [ ] 3.2 `read_only_grants`: tool directories and the header-walk host runtime, origin `provider:proto`. Shell per design P6.
- [ ] 3.3 Fingerprint over `.prototools`, `.protolock`, the plugin pins and the adapter version. Return `ServiceSupport::Unsupported`.

## 4. CLI

- [ ] 4.1 `init` detects `.prototools` by reading TOML only; `doctor` reports the proto version against the qualified range, missing records, refused tools and store size; minimum version enforced, one warning above the last qualified.

## 5. Tests, sample, documentation

- [ ] 5.1 Config and refusal tests; cold locked restore; ambient state ignored; sentinel absent; record for other platforms only; runtime write to the object refused; host `cc` unresolvable.
- [ ] 5.2 `samples/proto-sample` with a README showing the artifact-tier line, `policy --render` and what is refused.
- [ ] 5.3 `docs/decisions.md` §1: replace "Rejected: asdf, proto" with "Built: proto" and a separate asdf rejection; `README.md`, `docs/known-gaps.md`, `openspec/config.yaml`, `CLAUDE.md`.

## 6. If the gate stops

- [ ] 6.1 Rewrite "Rejected: asdf, proto" in `docs/decisions.md` with the measured property that fails and a new revisit condition. Update `VERSION_MANAGER_REASON` or give proto its own reason in `src/provider/validate.rs` if the old wording ("cannot guarantee a restorable lockfile") is no longer accurate. Close the change with groups 1–5 struck as not applicable.
