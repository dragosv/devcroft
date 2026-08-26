# Tasks — Remove the gVisor Backend

## 0. Preserve before deleting

- [ ] Tag the current commit so the working implementation stays recoverable,
      and reference the tag from `docs/decisions.md`.
- [x] Verify every finding in `design.md` G1–G3 against the actual commit
      messages and test files before the code goes; correct the wording where
      memory and the record disagree. **The findings are the value being kept —
      they must be accurate.**
      **Done at integration. G1 and all three of G3 are accurate as written**
      (`runner.rs`'s module doc for the `mount()`/`EPERM` chain;
      `materialize_bundle_writes_config_json_and_pre_creates_every_mount_point`,
      `oci_spec.rs`'s absolute `root.path` and the gofer's symlink-escape guard,
      and `exec_args_carry_the_argv_directly_with_no_separator`). **G2 is
      overstated and is corrected in place** — port separation at this tier came
      from the netns devcroft requests in its own OCI spec, not from runsc's
      netstack, so deny-default sandboxes did not collide. See the note under
      G2; `add-port-allocation` had already caught the same error once

## 1. Remove

- [ ] Delete the backend implementation, OCI bundle synthesis, and its
      integration tests.
- [ ] Remove the pinned runtime from the development container.
- [ ] Remove tier selection plumbing; keep the session backend trait (G5).
- [ ] Remove tier-conditional branches across commands.

## 2. Fail well

- [ ] `up` rejects a manifest requesting the removed tier, naming it, the
      supported tier, and the VM path.
- [ ] `doctor` no longer probes for the removed runtime.
- [ ] No silent fallback to the process tier anywhere.

## 3. Document

- [ ] `docs/decisions.md`: the decision, the three reasons, and the recovery
      tag. State the reasons that hold (G1, G2, squeezed middle) and not the
      overstated one (G4).
- [ ] README: replace the tier table with a single-tier statement and a short
      "what we learned" entry. Keep it to a paragraph — the detail belongs in
      the change. A draft is in `readme-hardened-tier-removal.md` beside this
      file.
- [ ] `docs/threat-model.md`: the ceiling is now fixed; use case B is closed to
      this roadmap, and the VM is the named answer.
- [ ] Record the future-backend criteria (`design.md`) somewhere durable —
      they outlive this change.
- [ ] `CLAUDE.md`: it lists `add-gvisor-backend` among the fully implemented
      changes and describes the hardened tier as delivered. Added at
      integration, because `docs/threat-model.md` already asserts the tier is
      dropped and the two now contradict each other in prose.

## 4. Downstream

- [ ] `add-backend-capabilities`: rewrite C1 and C5 for a single backend. The
      matrix now tracks adoption of the sandbox library's capabilities rather
      than divergence between backends.
- [ ] Collapse the isolation axis: with one backend, the model is
      single-environment versus fleet.
- [ ] `add-linux-agent-fleet`: remove any wording that scopes fleet to a tier.
