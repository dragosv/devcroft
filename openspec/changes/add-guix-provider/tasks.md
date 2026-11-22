## 0. Measurement gate — no code until these are answered

Not scheduled: start only when the owner records demand or the nix-concentration trigger in `docs/decisions.md` §1 fires. Record each result in `design.md`, tagged as measured, with the Guix revision and target (linux-x86_64, linux-aarch64), raw runs under `evidence/`.

- [ ] 0.1 Pin a Guix revision. Confirm `time-machine -C … -- shell -m … --pure --search-paths` prints definitions and starts no shell, and that `--root` works with `time-machine`. Record every file and variable guix reads from `HOME` and `/etc` during that command, and confirm an explicit `-C` ignores them.
- [ ] 0.2 Against a fixture whose packages and manifest would write a marker if any activation code ran, confirm nothing runs. Record the exact output grammar and escaping of `--search-paths`.
- [ ] 0.3 Compare `--search-paths` with `guix shell --pure -- env` for a Python plus C toolchain manifest; list any variable the first omits that a build needs.
- [ ] 0.4 Under `own-policy-baseline` with a mount view exposing only `/gnu/store`: a real C and Python build from the closure, the host `gcc`/`git` unresolvable, the daemon socket absent, a shell found in the closure.
- [ ] 0.5 Stop criteria. Stop and report to the owner if: capture requires running activation code (0.2); a pinned channels file does not reproduce the same store paths on a second host (0.1); a build needs a host library grant (0.4).

## 1. Selection and inputs

- [ ] 1.1 `src/provider/validate.rs`: add `"guix"` to `SUPPORTED`; `ProviderKind::Guix` (`Tier::Closure`) in `src/provider/mod.rs`. Platform refusal on macOS at layer `provider`.
- [ ] 1.2 `src/config/{mod,validate}.rs`: optional `[env.guix]` with `manifest` and `channels`.
- [ ] 1.3 Literal reader for `channels.scm` (design G1): every channel has a commit; computed forms refused.

## 2. Resolution, capture, roots

- [ ] 2.1 Run `time-machine` with an allowlisted environment; parse `--search-paths` per design G2.
- [ ] 2.2 Register the GC root at `<state>/<name>/guix-profile`; remove it at `rm`; `status` reports a missing root as broken.
- [ ] 2.3 Fingerprint over `manifest.scm`, `channels.scm` and the adapter version. `ServiceSupport::Unsupported`.

## 3. Shared seams

- [ ] 3.1 Make the store root provider-supplied in `src/provider/capture.rs` and `src/shell.rs` (design G5); nix, flox, devbox and devenv keep `/nix/store` with byte-identical compiled profiles (assert it).
- [ ] 3.2 `fleet::mount::construct_view`: expose `/gnu/store`, hide `/var/guix` (design G6). Add a `host_can_build_guix_closures()` probe.

## 4. CLI

- [ ] 4.1 `init` detects `manifest.scm` plus `channels.scm`; `doctor` reports guix, daemon reachability, the pin, and that the first `up` of a new pin may build Guix.

## 5. Tests, sample, documentation

- [ ] 5.1 Refusal tests (missing pin, computed channels, escaping paths, macOS); e2e on Linux: cold `up`, data-only capture, GC root survives `guix gc`, daemon socket unreachable, host `gcc` unresolvable.
- [ ] 5.2 `samples/guix-sample` (C and Python from the closure) with a README.
- [ ] 5.3 `docs/decisions.md` §1 "Built: guix" (or the rejection), the nix-concentration paragraph, `README.md`, `openspec/config.yaml`, `CLAUDE.md`.
