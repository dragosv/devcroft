# Tasks — Remove the gVisor Backend

## 0. Preserve before deleting

- [x] Tag the current commit so the working implementation stays recoverable,
      and reference the tag from `docs/decisions.md`. Tag:
      **`gvisor-backend-last`**, on the commit where the tier was last
      implemented and verified live. Its message lists what was deleted and
      points at this change for why. Referenced from `docs/decisions.md`
      (task 3.1)
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

- [x] Deleted `src/gvisor/` (mod, oci_spec, runner, runsc_command,
      session_backend — 1089 lines) and the three integration tests
      (`gvisor_hardened_e2e`, `hardened_tier_ssh_parity`,
      `hardened_services_wiring` — 670 lines), plus `pub mod gvisor` from
      `lib.rs` and the two gVisor-only `StatePaths` fields
      (`gvisor_bundle`, `gvisor_runsc_state`)
- [x] Removed the pinned `runsc` install from `.devcontainer/Dockerfile`,
      and the cross-references to it in the nono and devbox install blocks
      that would otherwise dangle. **`seccomp=unconfined` is deliberately
      kept**, with its justification rewritten: it was added for this tier,
      nothing in the tree needs it today, and `add-linux-agent-fleet` will
      need exactly it for namespace creation. Keeping a weakened container
      open for future work is a real trade, so the comment now says that
      rather than justifying the flag by a tier that no longer exists
- [x] Removed tier-selection plumbing: `up`'s `Isolation` dispatch,
      `up_hardened` (both the Linux implementation and the non-Linux
      stub), `spawn_hardened_keeper`, `hardened_keeper_main`, the hardened
      teardown helper, and `resolve_backend` — which collapses to
      `RESOLVED_BACKEND`, a constant, since there is nothing left to
      resolve and nothing that can fail. **The `SessionBackend` trait is
      kept** (G5); `ssh::server`'s comment about it no longer names a
      second implementation that does not exist
- [x] Removed tier-conditional branches: the `__hardened_keeper` dispatch
      arm in `main`, and `doctor`'s `hardened-tier` and `gvisor-backend`
      checks. `devcroft doctor` no longer mentions gVisor at all

## 2. Fail well

- [x] `up` rejects `isolation = "hardened"` at layer `config` (exit 2)
      with a message naming the removed tier, the supported one, and the
      VM path. Implemented as its own `ConfigError::RemovedIsolationTier`
      rather than letting serde produce "unknown variant", because the
      spec's requirement is about what the user reads: `RawSandbox` now
      takes the key as a raw string so the message is devcroft's to write.
      A tier that never existed gets a *different* error
      (`InvalidIsolationTier`) — telling a typo it was "removed" would be
      a small lie. Verified live for all three cases
- [x] `doctor` no longer probes for the removed runtime — verified by
      running it
- [x] No silent fallback: the rejection is an error, not a downgrade, and
      omitting the key produces the supported tier with **no output at
      all** (the spec rules out a deprecation notice for the common case —
      a manifest that never named the removed tier should not learn it
      existed). Both asserted by unit tests

## 3. Document

- [x] `docs/decisions.md`: the two-tier entry is replaced by a one-tier one
      naming the VM as the path to more, and a new "Removed: the gVisor
      hardened tier" entry carries the decision, the three reasons in the
      order they carry weight, and the `gvisor-backend-last` tag. G4 is
      recorded explicitly as **not** a reason. Three further entries that
      treated the tier as live were corrected: the netstack rejection is
      marked superseded (kept for its measurements), the resource-limits
      "revisit at the hardened tier" now points at `add-linux-agent-fleet`
      where cgroups actually live, and the port-collision entry's tier
      table collapses to "unconditional at N > 1" with the nuance kept as
      history
- [x] README: Limitations now opens with the single tier and the VM answer,
      followed by a "We built a hardened tier and removed it" section. The
      draft was applied **with the G2 correction it needed** — it claimed
      concurrent environments "collided exactly as they would with no tier
      at all", which was only true once egress was granted. The Status
      section's hardened narrative is marked superseded rather than deleted:
      it is a chronological log, and the measurements in it are the durable
      part
- [x] `docs/threat-model.md`: "with the hardened tier dropped" now names the
      change, states the ceiling as fixed at the process tier, and points at
      the recorded future-backend criteria for what would change it
- [x] The future-backend criteria are referenced from `docs/decisions.md`'s
      removal entry ("Revisit if"), from `openspec/config.yaml`'s deferred
      backends note, and from `docs/threat-model.md` — three places that
      outlive the change directory, each pointing at the list rather than
      restating it
- [x] `CLAUDE.md`: `add-gvisor-backend` is out of the implemented list, the
      `src/gvisor` module is off the module list, the tier-qualification
      framing rule is replaced by a one-tier one naming the VM, and the SSH
      invariant no longer branches on tier (with a note that the underlying
      invariant was the same at both, which is why removing one did not
      change it). `openspec/config.yaml`'s project context updated the same
      way. The contradiction with `docs/threat-model.md` is closed

## 4. Downstream

- [ ] `add-backend-capabilities`: rewrite C1 and C5 for a single backend. The
      matrix now tracks adoption of the sandbox library's capabilities rather
      than divergence between backends.
      **Blocked, not skipped:** that change does not exist in this repo. It is
      cited as a dependency by `add-linux-agent-fleet` and as authoritative by
      `docs/threat-model.md` ("prefer that matrix over any caveat written here
      or in the README"), so the rewrite has nothing to rewrite yet. Carry this
      forward to whenever it is written
- [x] Collapsed the isolation axis everywhere it was stated: `CLAUDE.md`'s
      framing rules, `openspec/config.yaml`'s Non-Goals and its "Isolation
      tiers" bullet, `docs/decisions.md`'s tier entry, and the README's
      Limitations. The remaining axis is single-environment versus fleet
- [x] `add-linux-agent-fleet`: its Impact bullet asked for the two-tier model
      to be re-examined and its design's Open Question 5 asked whether that
      model survives. Both are now answered rather than deleted — the axis
      collapsed on its own, for reasons unrelated to fleet, so the only axis
      left is the one that change is about
