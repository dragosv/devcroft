# Admit locked aqua, GitHub and Conda backends in the mise provider

## Why

The initial `mise` provider (`add-mise-provider`) admits only literal `http:` tools with platform-specific URLs written directly in `mise.toml`. That makes a single `mise.toml` non-portable across macOS and Linux, rejects mainstream registry declarations (`process-compose`, `ripgrep`, `jq`) that resolve to `aqua:` or `github:` in `mise.lock`, and blocks `add-mise-daemons` from declaring real servers (`postgresql`, `redis`) or language runtimes without triggering the `core:*` host-execution ban. In addition, `provider::mise::diagnose` still canonicalizes and reports the host's `/bin/sh` instead of the runtime shell introduced by `embed-brush-runtime`.

## What Changes

- Extend `provider::mise` to admit `aqua:`, `github:` and `conda:` backends — both when written with an explicit prefix in `mise.toml` and when written as a bare registry tool name whose `mise.lock` entry resolves to one of those backends — alongside `http:`.
- Keep `core:*` (`core:node`, `core:go`, `core:python`, `core:rust`), `npm:`, `pypi:`, `cargo:`, `gem:`, `asdf:` and `vfox:` refused before any `mise` subprocess starts.
- Require every selected tool (and any transitive `conda:` package metadata recorded by `mise.lock`) to carry an HTTPS URL without credentials or fragments and a verified `sha256:` checksum for the current platform, restricting `conda:` URLs to the qualified public hosts (`conda.anaconda.org`, `repo.prefix.dev`) so macOS keychain credentials are never consulted.
- Refuse unsafe per-tool options (`postinstall`, `install_env`, path-escaping `bin_path`/`exe`, templates) and control external signature/attestation helpers inside `isolated_env` so no host binary outside `CANONICAL_PATH` is required or executed implicitly.
- Update `provider::mise::diagnose` (`devcroft doctor`) to report the environment's resolved shell or devcroft's embedded runtime shell (`brush`), never the host's `/bin/sh`.

## Capabilities

### Modified Capabilities

- `config` accepts portable string and table tool forms for admitted `mise` backends while refusing unsafe tool options.
- `env-provider` validates and materializes locked `aqua:`, `github:` and public-host `conda:` entries without executing downloaded payloads on the host.
- `policy` attributes artifact and runtime-library grants for the expanded `mise` backends while keeping host toolchains and `/usr/bin` ungranted.
- `cli` aligns `doctor` shell reporting with `embed-brush-runtime` and reports actionable backend/lock refusals.

## Impact

Expected changes touch `src/provider/mise.rs`, `tests/mise_provider_e2e.rs`, `samples/mise-sample`, `docs/decisions.md` §1 and `openspec/config.yaml`. Completing this change unblocks portable multi-platform `mise.toml` definitions for `add-mise-daemons`.
