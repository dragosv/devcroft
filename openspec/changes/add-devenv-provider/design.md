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

## Measured (group 0)

devenv 2.2.2, aarch64-darwin, against a project declaring one package,
one `env` entry and an `enterShell` appending to a sentinel outside the
project root. Sentinel counts are hook executions.

| entry point | exit | hook runs | yields |
|---|---|---|---|
| `devenv build shell` | 0 | **0** | `{"shell": "/nix/store/…-devenv-shell"}` → a 16,062-byte `declare -x` dump |
| `devenv info` | 0 | 0 | 1,356 bytes of summary |
| `devenv eval env` | 0 | 0 | 724 bytes — the declared `env` only |
| `devenv eval enterShell` | 0 | **0** | 3,896 bytes of JSON: the hook's full text |
| `devenv direnv-export` | 0 | **1** | 80,256 bytes |
| `devenv shell -- env -0` | 0 | **2** | the real environment |
| `devenv update` | 0 | 0 | writes `devenv.lock` |

Four results the proposal did not have:

1. **`devenv build shell` writes `devenv.lock` when the project has
   none** — it resolves and locks rather than refusing. The up-front
   lockfile precondition is therefore load-bearing, not symmetry with
   devbox.
2. **With the lock present, capture touches exactly one path in the
   project tree**: `.devenv/nix-eval-cache.db`. Decision 6's confinement
   claim is measured, and the precondition in (1) is what keeps it true.
3. **`devenv eval enterShell` returns the hook's text without running
   it**, which settles design.md's open question on where to read it
   from: no store-path archaeology, and `devenv eval` is a documented
   command where the derivation path is not.
4. **The completeness diff is not empty in either direction** — the
   assumption this design rested on. Decision 8.

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

**How to obtain the script text: `devenv eval enterShell`.** Measured in
group 0 — it returns the hook's full text as JSON and does not run it
(0 sentinel appends, 3,896 bytes). The alternative, reading the
`…-devenv-enterShell` derivation out of the closure, works too and is
rejected: it means locating a store path by name pattern, where
`devenv eval` is a documented command taking a documented attribute.

Worth knowing before reading the result: what comes back is **not only
the project's `enterShell`**. devenv wraps it in a generated preamble —
temp-directory fixups, `MANPATH`, a profile symlink, a direnv-version
warning, and the `unset` of builder variables decision 8 covers — with
the project's own text at the end. devcroft runs the whole thing, which
is correct (that preamble is part of what makes a devenv shell a devenv
shell) and is why the captured script is larger than the project's
`enterShell` block.

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

### Decision 6: `.devenv/` is devenv's to own, and devcroft writes nothing else

devenv keeps its evaluation artifacts under `.devenv/` in the project
root, so unlike flox, nix and devbox, capturing a devenv environment
writes into the working tree. Three ways to take that, and only one
survives:

- **Refuse to write at all** (capture into a scratch copy of the
  project). Rejected: it changes what devenv evaluates — relative paths,
  `devenv.yaml` inputs pointing at the repo — so it would be capturing a
  different environment, the same objection that makes
  `flox::derive_hook_free_env` legitimate and a copy-the-tree trick not.
- **Write, then remove `.devenv/` afterwards.** Rejected: it is devenv's
  own cache and GC-root directory, shared with the user's own `devenv`
  invocations. Deleting it makes every `up` re-realize the closure and
  can drop GC roots the user was relying on — devcroft would be
  destroying state it does not own in order to look tidy.
- **Write, confined and declared.** Taken. Capture may create or update
  paths under `.devenv/` and nothing else in the project tree; the
  project's own declaration files are byte-identical across a capture,
  refused or not. That is stated as a spec property with a scenario, so
  "devenv wrote somewhere else" is a test failure rather than a
  discovery.

Two consequences that belong to other changes and are recorded so they
are not rediscovered: `sandbox-provisioning`'s provisioning profile must
grant write to `.devenv/` for this provider, and the sample in task 5.1
ignores it in git the way a devenv project normally does.

### Decision 7: the shell resolution is measured for devenv, not inherited

`src/shell.rs` resolves an absolute `sh` from the sandbox's own closure,
accepting a `PATH` hit only when it lies inside a path the provider
declared in `read_only_grants`. Every closure-tier provider satisfies it
today through `capture::store_grants`, but *whether the captured
environment contains a store-backed shell at all* is a property of what
the capture route returns, not of Nix in general — and the capture route
here is `devenv build shell`'s `declare -x` dump, which is exactly the
place a missing entry would not announce itself.

So it is measured in group 0 alongside the completeness diff, and
asserted end to end. A failure here does not look like a capture error:
it looks like `up` succeeding and `devcroft shell`, SSH login, and every
service command failing later.

### Decision 8: the hook-free capture needs a filter, and the hook restores the rest

`devenv build shell` does not emit the environment a developer gets. It
emits the environment of the *derivation that builds* it, and the two
differ in both directions. Measured against `devenv shell -- env -0`
under the canonical baseline — 103 keys versus 69:

**Present only in the hook-free capture (38 keys).** The Nix builder's
own variables: `out`, `outputs`, `stdenv`, `builder`, `buildInputs`,
`nativeBuildInputs`, every `deps*`, `phases`, `buildPhase`, `patches`,
`doCheck`, `doInstallCheck`, `preferLocalBuild`, `strictDeps`,
`__structuredAttrs`, `shell`, `shellHook`, `HOST_PATH`, `NIX_BUILD_CORES`,
`NIX_BUILD_TOP`, `NIX_ENFORCE_PURITY`, `NIX_LOG_FD`, `GZIP_NO_TIMESTAMPS`,
`TZ`, `TERM`, and `TMP`/`TMPDIR`/`TEMP`/`TEMPDIR` all pointing at
`/nix/var/nix/builds/nix-…`. Three of these are not noise but active
harm if injected into a sandbox:

- `HOME=/homeless-shelter` — the builder's sentinel home. Every session
  would get a `HOME` that does not exist.
- `SSL_CERT_FILE` and `NIX_SSL_CERT_FILE` set to `/no-cert-file.crt` —
  TLS broken for everything in the sandbox, in a way that looks like a
  devcroft network bug.
- `TMPDIR` and friends pointing into a build directory that is gone by
  the time the sandbox runs.

**Present only after the hook (6 keys).** `IN_NIX_SHELL`, `MANPATH`,
`DEVENV_CMDLINE`, plus `OLDPWD`, `SHLVL`-adjacent shell bookkeeping and
`__CF_USER_TEXT_ENCODING`. Of these, `MANPATH` and `IN_NIX_SHELL` are
exported **by `enterShell` itself** — so running the captured hook inside
the sandbox restores them. That is the result that keeps this design
whole: the hole in the hook-free route is exactly the part the hook
fills, which is the part devcroft was already going to run.

**Where the deny list comes from, and why not devenv's own.** devenv's
`enterShell` opens by unsetting 26 of those variables by name and
rewriting `TMP`/`TMPDIR`/`TEMP`/`TEMPDIR` — devenv knows the problem and
solves it *inside the hook*. devcroft cannot inherit that: the hook runs
in its own shell inside the sandbox, so its `unset` never reaches the
environment the keeper injects into every session. And the list is not
sufficient anyway — it omits `HOME`, `SSL_CERT_FILE`, `NIX_ENFORCE_PURITY`
and the temp directories, which the surrounding shell code handles
separately.

So the filter is devcroft's, stated as a rule rather than a list where
possible — drop what the Nix builder sets, keep what the environment
declares — and pinned by a test that, on a host that can run both routes,
asserts the filtered hook-free capture plus the hook's own exports equals
the truth, modulo an enumerated set. A key appearing on neither side of
that comparison breaks CI, which is the only way a list like this stays
correct across devenv versions.

**Why this does not reopen the gate.** Task 0.9 stops the change if the
hook-free route yields an *incomplete* environment and no complete
hook-free route exists. The route is not incomplete — it is a superset
with a different `HOME` — and nothing that is missing is missing after
the hook devcroft already runs. Criterion 4 holds.

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
- **The captured environment has no shell `src/shell.rs` can resolve**,
  which surfaces as working `exec` and broken login sessions rather than
  as a capture failure → Measured in group 0, asserted in group 5
  (decision 7).
- **Capture writes into the project tree** — a property no shipped
  provider has → Bounded to `.devenv/` by spec and test (decision 6).
- **devenv is not installed in this repo's devcontainer by default** →
  It is reachable via `nix run nixpkgs#devenv`, which is how this design
  was measured; tests guard on the capability.

## Migration Plan

None. Additive: a new `env.provider` value. No existing manifest
changes meaning, and `policy --render` output for any manifest that does
not name `devenv` is byte-identical, which the config delta asserts.

## Open Questions

- ~~Which artifact to read `enterShell`'s text from.~~ Answered in group
  0: `devenv eval enterShell`, which does not run it. See decision 3.
- Whether `devenv.yaml` inputs can float in a way `devenv.lock` does not
  pin, the way devbox's base nixpkgs entry did. Affects one precondition
  test, not the approach.
- ~~Which devenv command creates a missing `devenv.lock`.~~ Answered in
  group 0: `devenv update` writes it, runs no hook, and is what `init`
  advises. (`devenv build shell` also writes it, which is why the
  up-front precondition exists rather than being symmetry with devbox.)
