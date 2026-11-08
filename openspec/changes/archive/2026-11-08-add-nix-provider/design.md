# Design: add-nix-provider

## Context

`src/provider/` today has exactly one implementation of the `Provider`
trait: `FloxProvider`. Its contract is small and already proven —
`resolve(project_root) -> Resolution { env, unset, read_only_grants }` —
and everything downstream (keeper env injection, policy compilation of
`provider:*` grants, staleness in `status`) consumes `Resolution` without
knowing which provider produced it. `provider::validate` already reserves
the names `nix`, `flake`, and `flakes` as "not yet supported (closure
tier)", and `lifecycle::up` hard-wires `FloxProvider` at its single call
site.

Flox is itself nix underneath: its activation diff already yields
`/nix/store` paths, and `store_grants` already extracts the store root
from them. A nix flakes provider is therefore mostly the same machinery
pointed at `nix` directly, minus flox's manifest layer.

## Goals / Non-Goals

**Goals:**

- A second `Provider` implementation, proving the trait generalizes.
- Serve `flake.nix` + `flake.lock` projects at closure tier with the
  exact same guarantees flox projects get (fixed-env activation capture,
  network-deny-safe sessions, deterministic policy).
- Preconditions all verifiable at `up` in milliseconds (decisions.md §1
  criterion 6).

**Non-Goals:**

- Guarantee-tier machinery as user-visible state (`add-mise-provider`
  owns that; both flox and nix are closure tier so nothing needs
  distinguishing yet).
- Classic non-flake nix (`shell.nix`, `default.nix`) — no lockfile,
  fails criterion 2.
- Non-default dev shells (`devShells.<system>.ci`) — additive later via
  a manifest key.
- Installing or managing nix itself; devcroft checks, it does not
  bootstrap.

## Decisions

### 1. Capture via `nix develop --command env -0`, not `print-dev-env`

Mirror flox's capture exactly: run the activation once with the canonical
fixed baseline (`HOME` real, `PATH = CANONICAL_PATH`, nothing else), dump
the resulting environment NUL-separated, diff against the baseline.
`FloxProvider` does `flox activate -- env -0`; `NixProvider` does
`nix develop <root> --command env -0`.

Alternative considered: `nix print-dev-env` emits the shell environment
without spawning a shell, which is cheaper — but its output is a bash
script meant to be `eval`'d (function definitions, `declare -x` with
bash quoting), so consuming it means either sourcing it in a bash child
(back to spawning a shell, now with parsing on top) or writing a parser
for bash serialization. `env -0` under `--command` produces the same
byte format flox capture already handles, so the diff/unset/grant
pipeline is shared code, not parallel code. Revisit only if `nix
develop`'s startup cost proves material; it runs once per `up`.

`--command env -0` also sidesteps pty concerns: no interactive shell is
ever spawned.

### 2. Shared capture machinery moves up, providers stay thin

`canonical_base_env`, `changed_env`, `unset_env`, `store_grants`, and the
NUL-separated env parsing are provider-independent and currently private
to `flox.rs`. They move to `provider::capture` (module-private to
`provider/`), and both providers become: check preconditions, run their
one activation command, hand the output to shared capture. The
determinism guarantee ("Activation is independent of the invoking
shell") is then enforced in one place for every provider — including
mise later.

Alternative: copy the helpers into `nix.rs`. Rejected — the review
finding that produced `canonical_base_env` (invoking-shell leakage)
would have to be re-learned per provider.

### 3. Flake evaluation is pinned by requiring `flake.lock` and `--no-update-lock-file`

An unlocked or partially-locked flake resolves inputs at evaluation time
against the registry — the same manifest resolving differently on
different days. So:

- `flake.lock` missing → `up` fails, layer `provider`, exit 3, hint
  `nix flake lock`.
- The capture command runs with `--no-update-lock-file` so nix errors
  out rather than silently writing a new lock for inputs the lock does
  not cover (a lock write is also a project-root write during
  provisioning, which is fine, but a *silent repin* is not).
- `--impure` is never passed. Purity beyond that is not proven (nix
  itself only enforces full hermeticity during builds, not evaluation);
  documented as a known limitation rather than claimed.

### 4. Store grants come from the diff, with a closure-shaped widening

Flox's `store_grants` extracts `/nix/store` (the root) from activated
env values. For nix the same extraction works — dev shell env vars are
dense with store paths — and granting the store *root* read-only is what
flox already does, so nix inherits exactly that behavior and origin
annotation shape (`provider:nix`). Granting individual closure paths
instead (via `nix path-info --recursive` on the shell derivation) would
be tighter but is a policy-size and staleness liability (thousands of
rules), and would diverge from what flox already ships. Keep root-level;
note the tightening as future work for both providers together.

### 5. Precondition checks, in order, all at `up`

1. `flake.nix` exists in project root → else `NoEnvironment`-class error,
   hint `nix flake init`. (The flox hint stays `flox init`; the error
   message becomes provider-aware.)
2. `nix` resolves on the *ambient* PATH (`paths::resolve_on_path`, same
   pre-replacement lookup that fixed the `nono` ENOENT bug) → else
   `MissingBinary`, hint `devcroft doctor`.
3. Flakes enabled: `nix flake metadata <root> --no-update-lock-file`
   exits 0. This single probe covers "experimental-features missing",
   "daemon unreachable", and "flake.lock stale vs flake.nix inputs" in
   one cheap command, each with a distinguishable error to surface.
4. `flake.lock` exists → else hint `nix flake lock` (checked before 3 so
   the hint is precise rather than nix's own error).

`doctor` runs 2 and a flakes-enablement probe (`nix flake --help` exit
status) without needing a project.

### 6. Provider dispatch replaces the hard-wired `FloxProvider`

`lifecycle::up` currently constructs `FloxProvider` directly. It gains a
`provider_for(name) -> Box<dyn Provider>` (or small enum — enum
preferred: two variants, no object-safety questions, exhaustive match)
keyed off the validated `env.provider`. `validate.rs` moves
`nix`/`flake`/`flakes` out of `NOT_YET_SUPPORTED`; `flake`/`flakes`
normalize to `nix` at config parse so exactly one canonical name reaches
dispatch, `status`, and policy origins. Staleness dispatches the same
way: `manifest_fingerprint` becomes provider-keyed (`flake.nix` +
`flake.lock` for nix).

### 7. `init` detection is additive, not preferential

`init` currently assumes flox. With both present (`.flox/` and
`flake.nix`), prefer flox — the more specific, devcroft-native choice —
and say so in one line. With only `flake.nix`, write
`env.provider = "nix"`. With neither, the existing `flox init` guidance
stands (nix flakes require hand-writing a flake; flox init scaffolds).

## Risks / Trade-offs

- [`nix develop` startup cost — flake evaluation can take seconds on
  first run, minutes on cold store] → It runs host-side at `up` where
  provisioning is allowed to take time and has network; `up` already
  prints progress for flox materialization. Not a session-time cost.
- [Env capture through `--command` inherits nix's bash wrapping; some
  dev shells' `shellHook`s print to stdout and would corrupt `env -0`
  output] → Capture stdout of `env -0` only via a marker or, simpler,
  redirect: run `env -0 > <tmpfile>` inside the command and read the
  file, making shellHook chatter harmless. Decide at implementation;
  test with a shellHook that prints.
- [Purity is not proven (decision 3): a flake can read the host via
  builtins even without `--impure`] → Documented limitation, consistent
  with tier framing ("closure tier as delivered by nix evaluation", not
  a stronger claim than nix itself makes). Never claimed otherwise.
- [Store-root read grant is wide (all of `/nix/store`)] → Identical to
  shipped flox behavior; contents are world-readable by nix's own model;
  tightening tracked as shared future work, not a regression introduced
  here.
- [macOS: nix daemon socket must be reachable *host-side* only] →
  Provisioning runs before restriction so the daemon is reachable at
  `up`; sessions never need it. If a session-time tool shells out to nix
  it fails under the default policy — same category as any other
  network/daemon tool, surfaced by the error contract, not special-cased.

## Migration Plan

Purely additive: no existing manifest changes meaning, `flox` remains the
default provider, and rejection messages for still-unsupported providers
are untouched. Rollback is removing the dispatch arm; no state format
changes (`profile.json` gains only rules with a new origin string).

## Open Questions

Carried from proposal.md (capture mechanism finalized above as decision
1; the rest stand): non-default dev shell selection, and whether
`shell.nix` rejection should get its own message distinct from "no
flake.nix here".
