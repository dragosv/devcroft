## 1. Qualify lock and offline-locked behavior

- [ ] 1.1 On `macos-arm64` and `linux-aarch64` with `mise >= 2026.11.9`, generate real `mise.lock` files for `aqua:`, `github:`, bare registry shorthands (`process-compose`, `ripgrep`) and `conda:` (`postgresql` or `nodejs`). Record exact lock schemas, sidecar files (if any), network calls during `mise install --locked`, and `MISE_*` settings needed so locked restore uses only locked URLs + SHA-256 without invoking external `cosign`/`slsa` binaries or executing downloaded payloads. Record results under `openspec/changes/add-mise-aqua-github-backends/evidence/` and `design.md`.

## 2. Implement backend validation and materialization

- [ ] 2.1 Update `src/provider/mise.rs` to accept string and qualified table `[tools]` declarations for `http:`, `aqua:`, `github:`, `conda:` and bare shorthands resolving to them in `mise.lock`, supporting both flat and nested `platforms.<target>` lock tables. Refuse `core:*`, `npm:`, `pypi:`, `cargo:`, `gem:`, `asdf:`, `vfox:`, `postinstall`, templates, non-HTTPS URLs, missing `sha256:` digests, and non-public `conda:` hosts before spawning `mise`.
- [ ] 2.2 Bump `ADAPTER`, include all consumed lock inputs in `fingerprint()` and object ID hashing, enforce byte-identical lock preservation after `mise install --locked`, and update `diagnose()` (`devcroft doctor`) to report the environment or `runtime:gen-2` (`brush`) shell rather than host `/bin/sh`.

## 3. Verify and document

- [ ] 3.1 Add unit and integration tests (`src/provider/mise.rs`, `tests/mise_provider_e2e.rs`) covering bare shorthand -> `aqua:` acceptance, bare `node` -> `core:node` refusal with `conda:` remedy, `postinstall` refusal, private `conda:` host refusal, shared read-only object reuse across sandboxes, and `doctor` reporting no host `/bin/sh`.
- [ ] 3.2 Update `samples/mise-sample` to use a portable multi-platform `mise.toml` + `mise.lock`, and update `docs/decisions.md` §1 and `openspec/config.yaml` with the newly qualified backends.
- [ ] 3.3 Qualify `conda:bash` as the mise route to a real shell, on both platforms, with a locked fixture: `/bin/sh` becomes that bash and the runtime’s brush `sh` is no longer used. Update the brush gap in `docs/known-gaps.md`, which names only pixi’s route today.
