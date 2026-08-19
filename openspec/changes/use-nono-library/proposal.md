# Change: use-nono-library

Status: **proposal only**, post-MVP. No tasks.md — this is a sketch
carrying real delta specs, the same posture `add-mise-provider` holds.
Depends on: `own-policy-baseline`, which is not optional here but
load-bearing, for the reason in Why.

## Why

devcroft applies the process tier by resolving a `nono` binary on
`PATH` and exec'ing the keeper under `nono wrap -p <profile> --`
(`src/lifecycle/up.rs`). Three costs follow from the process boundary
rather than from anything devcroft wants:

**The dependency is unpinned.** The backend is whatever version happens
to be installed. `doctor` compensates with a hand-maintained range
(`>=0.71.0, <0.72.0`), currently three releases behind the published
0.74.0. A library dependency is pinned by `Cargo.lock`, which is where
every other devcroft dependency is already pinned.

**The interface was derived by experiment.** The source records that the
profile must be passed via `-p` and never `-c`, because the latter is
"an unrelated, stricter capability manifest schema" — a distinction
found by running the binary, not by reading a contract. A typed API
makes the same distinction a compile error.

**There is an exec hop that the architecture would rather not have.**
devcroft's listener-before-restriction invariant requires creating the
unix sockets first, then having the restricted process apply the profile
*to itself*. Today that self-application happens inside a foreign
process that then execs devcroft's keeper, so the listener fds must
survive a hop through a binary devcroft does not control. The library
exposes `Sandbox::apply_auto(&caps)`, which applies restrictions to the
current process irreversibly — precisely the shape the invariant
describes, with the hop removed.

Upstream supports this reading: `nono` (250,935 downloads) and
`nono-cli` (2,696) are published in lockstep from one repository, and
the library carries the enforcement layer as its documented purpose.
Consuming the library is the intended path, not a workaround.

## Why it depends on own-policy-baseline

The split between the two crates is not where it might appear.
Inspected directly:

- `crates/nono/src/` — `capability.rs`, `manifest.rs`, `sandbox/`,
  `net_filter.rs`, `resource/`, `query.rs`. The enforcement layer.
- `crates/nono-cli/src/` — `profile/`, `profile_runtime.rs`,
  `policy.rs`, `protected_paths.rs`, `command_policy.rs`,
  `network_policy.rs`, plus 53 direct dependencies. The policy content,
  including every built-in profile and the group catalog `default`
  resolves to.

So the library provides the mechanism and none of the policy. That
matters more than it first appears, and more than an earlier version of
this section stated.

Measured with `nono profile show <file> --json`: the CLI injects its
full 18-group set into *every* profile, including one that declares no
groups and extends nothing. Eight of those groups are mandatory and
refuse exclusion — `deny_credentials`, `deny_keychains_*`,
`deny_browser_data_*`, `deny_shell_history`, `deny_shell_configs`,
`deny_macos_private`. They are why `~/.ssh` is denied inside a devcroft
sandbox today, and devcroft does not emit a single one of them.

A devcroft that links `nono` therefore does not merely lose a
convenience: it loses the credential, keychain, browser-data and
shell-config denials outright, unless it carries them itself. That is
the real content of the dependency on `own-policy-baseline` — not
"emit a different profile field first", but "know what you are currently
getting for free before you stop getting it".

Stated the other way round: `own-policy-baseline` is what turns the
question "what does devcroft's policy actually consist of" from
unanswered into answered, and this change cannot be evaluated honestly
until it is.

## What Changes

- **The process tier links `nono` instead of exec'ing `nono`.** The
  keeper applies the compiled capability set to itself after inheriting
  its listener fds, replacing the `nono wrap` prefix.
- **The compiled policy becomes a typed value, not a JSON file.**
  `policy::compile` produces something convertible to the library's
  capability set directly. `policy --render` continues to render, and
  gains the property that it no longer requires a backend binary to be
  installed at all.
- **`doctor`'s backend check changes shape.** There is no external
  binary to find or version-match; what remains is a platform-support
  question, which the library answers directly (`SupportInfo`).
- **Degraded-capability detection moves to the source of truth.** The
  "surfaced, never silent" invariant currently reasons about platforms
  from devcroft's own knowledge; the library reports what the running
  kernel actually supports.

## Capabilities

### Modified Capabilities

- `policy`: the compiled policy is handed to the backend as a typed
  value; rendering no longer depends on an installed backend binary.
- `lifecycle`: the keeper self-restricts after inheriting its listeners,
  with no intermediate process between `up` and the keeper.

## Impact

- Affected specs: modified `policy`, `lifecycle`.
- Affected code: `src/policy/mod.rs` (profile emission becomes capability
  construction), `src/lifecycle/up.rs` (`spawn_keeper` loses the `nono`
  prefix; the keeper gains a self-restrict step), `src/bin/devcroft.rs`
  (`doctor_backend`), `Cargo.toml`.
- Not affected: the hardened tier. gVisor is a different backend
  entirely and this change does not reach it — it stays as it is.
- Installation: `nono` stops being a runtime prerequisite for the
  process tier. That is a user-visible simplification and the strongest
  practical argument for the change.

## The unresolved objection

This is the reason the change is a proposal rather than a plan.

Linking `nono` adds **141 crates** to devcroft's dependency tree
(measured: `nono` resolves 189, devcroft resolves 158, 48 shared).
Among them:

- the full Sigstore verification stack (9 crates) plus `x509-cert`,
  `cms`, `cmpv2`, `crmf`, `der`
- `reqwest`, `hyper`, `hyper-rustls`, `tower`, `rustls`,
  `rustls-platform-verifier`, `aws-lc-rs`/`aws-lc-sys`
- the ICU stack, via `idna`

None of it is optional. Verified against crates.io's dependency
metadata for `nono` 0.74.0: of 23 normal dependencies exactly one —
`keyring` — is declared `optional = true`. `sigstore-verify`,
`sigstore-trust-root`, `x509-cert` and `der` are unconditional, so
`default-features = false` removes `keyring` and nothing else.

Two concerns follow, of unequal weight:

- **A networked trust client links into the keeper.** The keeper is the
  process that must have no network access after restriction. Nothing
  would call the TUF or Rekor paths, but linking a capability into the
  one process whose defining property is not having it deserves an
  explicit answer rather than a shrug.
- **Build weight.** Less severe than assumed: the probe crate compiled
  in 15.7s wall on this machine with no `cmake` installed —
  `aws-lc-sys` used its `cc` fallback. Dependency count is a real cost
  for audit and supply-chain surface, but "hard to build" is not a
  supportable objection.

The clean resolution is upstream, and it is worth stating as the
concrete request: if `nono` gated its trust module behind a feature, or
published enforcement separately from verification, this change would
have no objection left. That is a smaller and better-defined
contribution than reimplementing anything locally.

## Success Criteria

Deliberately stated even though this is a sketch, because they are what
would make the change acceptable:

- The process tier behaves identically before and after — the existing
  `process_tier_landlock_boundaries` suite passes unchanged, since it
  asserts enforcement rather than mechanism.
- The listener-before-restriction ordering is preserved and tested: the
  control socket remains reachable from outside after the keeper
  restricts itself.
- `nono` is absent from `PATH` and the process tier still works.
- `policy --render` produces output with no backend installed.
- Degraded capabilities are reported from the library's platform support
  rather than inferred.

## Open Questions

- **Whether the trust dependency is acceptable at all**, or whether this
  change should be blocked on an upstream feature gate. Not settled, and
  it is the question that decides the change.
- **Whether both consumption paths should coexist** during migration —
  library on Linux, binary on macOS, say — or whether a split backend is
  worse than either end state.
- **What `doctor` should report** when there is no backend binary to
  probe. A check that always passes is not obviously better than no
  check, and the platform-support question it would replace it with is a
  different question than users currently read it as answering.
