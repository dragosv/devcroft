## 0. Re-qualification — no code until these are answered

Not scheduled: start when the owner records demand, or earlier to correct the existing "qualified" claim (groups 0 and 6 only). Record each result in `design.md`, tagged as measured, with the hermit version and target (linux-aarch64, osx-arm64), raw runs under `evidence/`.

- [ ] 0.1 Pin a hermit release. Record how a project stores its selected versions, whether a git source accepts a commit pin, and whether a pinned source is still re-synced or rewritten. Name every directory and variable hermit reads, including the state directory and its override.
- [ ] 0.2 Checksums: across the default manifest repository at one revision, count packages with no checksum for linux-arm64 and darwin-arm64; measure that a checksum mismatch and an unreachable `sha256-source` both fail closed.
- [ ] 0.3 Triggers: count packages per trigger and action at that revision. With sentinel archives, confirm which non-`run` actions execute nothing.
- [ ] 0.4 Confirm `hermit env` prints variables without running anything (no trigger fires, no hook installed) and record its grammar. Under `own-policy-baseline`: host-runtime files for Go, Node and Python packages, host `cc` unresolvable, a real build.
- [ ] 0.5 Verdict. Rejected if any of: no immutable source pin (0.1); checksums not enforced (0.2); `hermit env` runs triggers (0.4); fewer than two of Go, Node and Python admissible on both platforms after design H3 and H4 (0.3). Otherwise admitted. Go to group 6 on rejection, group 1 on admission.

## 1. Selection and inputs

- [ ] 1.1 `src/provider/validate.rs`: move `"hermit"` from `NOT_YET_SUPPORTED` to `SUPPORTED`; give the validate.rs unit test and the `add-mvp-core` "Planned provider rejected in MVP" scenario another not-yet-supported example; `ProviderKind::Hermit` (`Tier::Artifact`).
- [ ] 1.2 Source pinning and manifest reading per design H2–H4, before any subprocess.

## 2. Materialization and capture

- [ ] 2.1 Allowlisted environment, state inside `~/.local/share/devcroft-artifacts/hermit/<id>/` through `provider::artifacts`, completion record, sweep.
- [ ] 2.2 Capture `hermit env` as data; apply `hermit.hcl` `env` in devcroft; grants with origin `provider:hermit`; shell per the mise rule; fingerprint; `ServiceSupport::Unsupported`.

## 3. CLI

- [ ] 3.1 `init` detects `bin/hermit.hcl`; `doctor` reports version, pins, refused packages by trigger or checksum, store size.

## 4. Tests and sample

- [ ] 4.1 Refusal tests (unpinned, local source, command trigger, missing checksum); e2e cold restore, ambient state ignored, sentinel absent, runtime write refused, host `cc` unresolvable.
- [ ] 4.2 `samples/hermit-sample` with a README.

## 5. Documentation (admitted)

- [ ] 5.1 `docs/decisions.md` §1 "Built: hermit"; `README.md`, `openspec/config.yaml`, `docs/known-gaps.md`, `CLAUDE.md`.

## 6. Documentation (rejected)

- [ ] 6.1 `docs/decisions.md` §1: a "Rejected: hermit" entry naming the measured property and a revisit condition; remove hermit from the artifact-tier example list.
- [ ] 6.2 `src/provider/validate.rs`: replace the `NOT_YET_SUPPORTED` entry with a rejection carrying that reason; update its unit test and the `add-mvp-core` scenario.
- [ ] 6.3 `openspec/config.yaml` (drop it from "qualified but unscheduled") and the README's "hermit is a different answer" paragraph.
