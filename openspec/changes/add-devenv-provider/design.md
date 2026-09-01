## Context

See proposal.md — Why, for motivation and the full entry-point
measurement. What matters here is the shape that measurement leaves
behind.

devcroft's `Provider` trait has been exercised by three implementations
with three activation mechanisms. `Resolution` did not change shape when
nix landed, nor when devbox did. The pieces devenv needs already exist:
`capture` for fixed-baseline env diffing, `store_grants` for closure
attribution, and — this is the part flox forced into existence —
`Resolution::activation_script` plus `hooks::run_activation_script`, the
path that carries a provider's own hook into the sandbox instead of
running it host-side.

Three measured facts about devenv 2.2.2 constrain everything below:

1. `devenv build shell` emits the complete environment as `declare -x`
   statements and does **not** run `enterShell`.
2. `devenv direnv-export` — devenv's own environment-export command, used
   by its direnv integration — **does** run `enterShell`.
3. `enterShell` is a discrete derivation (`…-devenv-enterShell`, a
   407-byte executable script), addressable as the task
   `devenv:enterShell`.

## Goals / Non-Goals

**Goals:**

- Capture a devenv environment host-side with no project code executed.
- Run `enterShell` inside the sandbox, through the existing hook path.
- Add no new concepts to `Resolution`, policy compilation, or lifecycle.

**Non-Goals:**

- devenv services / `processes` (see proposal.md — Impact).
- Any change to how the other three providers capture.
- Supporting devenv's container, task, or test subsystems.

## Decisions

### Decision 1: capture via `devenv build shell`, not `direnv-export`

`direnv-export` is the obvious choice by name — it is devenv's own
environment-export command, and it produces a complete 75KB environment.
It is rejected because it runs `enterShell`, measured, which makes it a
criterion-4 failure and a two-phase violation regardless of how
convenient it is.

`devenv build shell` returns a store path whose content is the same
environment expressed as `declare -x` lines, produced without running the
hook. Parsing `declare -x` is the same class of work `nix.rs` already
does against `print-dev-env`.

**Alternative considered: `devenv eval`.** Hook-free, but it returns only
the *declared* `env` attribute — no `PATH`, no toolchain. Insufficient,
and worth recording so it is not revisited as though it were.

**Alternative considered: build the environment, then run
`direnv-export` and subtract the hook's effects.** Rejected outright:
subtracting a side effect after running it is not the same as not running
it, and the side effects are arbitrary project code.

### Decision 2: consume an artifact that calls itself internal, and say so

The file `devenv build shell` produces opens with:

> WARNING: the existence of this path is not guaranteed. It is an
> internal implementation detail for pkgs.mkShell.

`add-flox-services` decision 1 rejected consuming flox's generated
`service-config.yaml` on exactly this ground — an undocumented internal
artifact is not a contract. That objection is taken seriously here rather
than waved through, and the answer is that the two cases differ in what
the alternative is:

- For flox services, the alternative was devcroft generating its own
  process-compose config from a **documented** `[services]` schema. A
  documented route existed.
- For devenv capture, every documented route runs project code. The
  alternative is not a better contract, it is abandoning criterion 4.

So this is taken, with three obligations that the tasks enforce: the
parser fails loudly on unrecognized content rather than silently
capturing a partial environment; a test pins the format against a real
devenv so an upstream change breaks CI rather than a user's sandbox; and
`docs/decisions.md` records the dependency, so if devenv changes the
format the response is a decision, not a surprise.

### Decision 3: `enterShell` is captured as data, run inside

`Resolution::activation_script` already exists for exactly this, built
for flox. devenv is the easier case: flox needed
`flox::derive_hook_free_env` — devcroft building a derived, hook-free
copy of the environment — because no flox mode suppresses the hook.
devenv's hook is already separate, so devcroft reads it rather than
engineering around its absence.

**How to obtain the script text** is deliberately left to task-group
measurement rather than decided here: the `devenv:enterShell` derivation
is a readable file, and `devenv eval` may expose the source, but which is
stable across versions has not been measured. Either satisfies the spec,
which is written as a property of the result.

**Consequence, and it is a behaviour difference worth stating.** An
`enterShell` reaching for host tooling is denied inside the sandbox. This
is `own-policy-baseline` working as designed and matches what flox hooks
already do — but a devenv user whose `enterShell` shells out to a host
binary will see it fail where `devenv shell` succeeds.

### Decision 4: `ran_activation_hook` is false for devenv

The flag means "project code ran host-side, so the reason the
provisioning phase is trusted does not hold". For devenv it never does.
Reporting true would train users to ignore a warning that, for this
provider, is always wrong.

This makes devenv the first provider where the flag is false *and* a
project hook exists — nix and devbox report false because they have no
captured hook at all. The distinction is real and the tests assert it.

### Decision 5: staleness fingerprints three files

`devenv.nix` + `devenv.yaml` + `devenv.lock`. `devenv.yaml` carries the
inputs; it can change what resolves with `devenv.nix` untouched. Omitting
it would report a changed environment as fresh — the failure mode
staleness detection exists to prevent.

## Risks / Trade-offs

- **`devenv build shell`'s format changes upstream** → A format test
  against a real devenv, run in CI where devenv is available and skipped
  on the *capability* (not the binary) elsewhere, per this repo's
  standing rule. The parser fails loudly rather than capturing a partial
  environment.
- **The captured environment is subtly incomplete** — it carries the
  packages, `env`, and compiler variables, but has not been diffed
  against what `devenv shell` produces minus the hook → Task group 0
  does that diff before any code depends on the answer. A capture that
  silently omits part of the environment is worse than one that fails.
- **`devenv shell` runs `enterShell` twice per invocation**, measured and
  unexplained → Not on the chosen path, so it does not block this change;
  recorded because an unexplained doubling near the hook boundary is the
  kind of thing that turns out to matter later.
- **Fourth nix-based provider deepens a concentration risk** already
  accepted in `docs/decisions.md` §1: upstream churn in flakes, the
  daemon, or store semantics now hits four providers at once → Accepted,
  not mitigated, and named here so it is not rediscovered as a surprise.
- **devenv is not installed in this repo's devcontainer by default** →
  It is reachable via `nix run nixpkgs#devenv`, which is how this design
  was measured; tests guard on the capability.

## Migration Plan

None. Additive: a new `env.provider` value. No existing manifest
changes meaning, and `policy --render` output for any manifest that does
not name `devenv` is byte-identical, which the config delta asserts.

## Open Questions

- Which artifact to read `enterShell`'s text from (the derivation file or
  `devenv eval`). Both satisfy the spec; the choice is a stability
  question the first task group answers.
- Whether `devenv.yaml` inputs can float in a way `devenv.lock` does not
  pin, the way devbox's base nixpkgs entry did. Affects one precondition
  test, not the approach.
