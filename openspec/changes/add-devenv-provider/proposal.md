# Change: add-devenv-provider

Status: proposed. Scheduled at **0.5, with `sandbox-provisioning`** —
see `docs/roadmap.md` and `docs/decisions.md` §1 ("Not yet built:
devenv") for why that release and not an earlier one. This is the fourth
closure-tier provider, and the second one (after devbox) that exists to
confirm the `Provider` trait generalizes rather than to introduce a new
guarantee.

**Unlike `add-devbox-provider`, this proposal is written against a
running devenv** (2.2.2, aarch64-linux). Every claim below about which
entry point does what is measured, not read from documentation. That
matters here more than usual, because the measurement changed the
answer.

## Why

devenv is the largest population of declarative Nix environments devcroft
does not serve, and the only remaining one with a real lockfile
(`devenv.lock`), a declarative manifest (`devenv.nix`), and the same
`/nix/store` the three shipped providers already share. Criteria 3 and 5
are answered by that shared store model, so it is the cheapest qualified
provider left — the amortization `add-devbox-provider` demonstrated
applies again, because this is the fourth *nix* provider rather than the
first of a new kind.

`openspec/config.yaml` has listed devenv as "qualified but unscheduled"
for some time. **That qualification predates the criterion it would now
have to pass.** Criterion 4 was tightened by `fix-provisioning-hooks`
from "capturable activation" to "capturable activation **without
executing project code**" — the half that was always meant and never
written down, and which two shipped providers were violating unnoticed.
config.yaml still carries the old wording. So devenv is not
re-qualifying against the bar it was first measured against, and this
change exists partly to settle that.

**The measurement, which is the reason to want this provider rather than
merely to accept it.** `enterShell` is project code, so the question is
which entry point hands back an environment without running it. Measured
against devenv 2.2.2 with an `enterShell` that appends to a sentinel
file:

| entry point | runs `enterShell` | yields the full environment |
|---|---|---|
| `devenv shell -- <cmd>` | **yes** (twice per invocation) | yes |
| `devenv direnv-export` | **yes** | yes (74,959 bytes) |
| `devenv info` | no | no — a summary, not an environment |
| `devenv eval env` | no | no — only the *declared* `env` attribute |
| `devenv build shell` | **no** | **yes** — an 11,147-byte `declare -x` dump |
| `devenv tasks run devenv:enterShell` | yes, on demand | n/a |

Two findings follow, and together they are the case for this change:

- **devenv has a hook-free capture route.** `devenv build shell` builds
  the environment and emits it as `declare -x` statements — `CC`, `AR`,
  the declared packages, the project's own `env` entries — without
  running `enterShell`. That is the same shape as `nix print-dev-env`,
  which is the entry point the nix provider already uses.
- **devenv also has a native handle for running the hook later.**
  `enterShell` is not an inseparable phase of activation; it is a
  discrete, addressable derivation (`/nix/store/…-devenv-enterShell`, a
  407-byte executable script) exposed as the task `devenv:enterShell`.

That second finding is why 0.5 is the right release rather than a
constraint to work around. flox needed `flox::derive_hook_free_env` — a
derived, hook-free copy of the environment devcroft builds itself —
because no flox mode suppresses `[hook].on-activate`. devenv needs no
such derivation: the split already exists upstream, and
`Resolution::activation_script` plus `hooks::run_activation_script`
already exist to carry a captured hook into the sandbox. devenv would be
the first provider where the two-phase rule is satisfied by the
provider's own structure rather than by devcroft working around its
absence.

## What Changes

- **`env-provider` gains provider `devenv`**, closure tier. No new tier
  machinery, and `docs/decisions.md` §1's artifact-tier host-grant rule
  does not apply. `policy --render` gains nothing beyond a store closure
  attributed `provider:devenv`.
- **Resolution captures the environment host-side at `up` via the
  hook-free route**, diffed against the same fixed canonical baseline
  flox, nix and devbox share, so the diff does not depend on the
  operator's shell. The concrete command is design.md's decision; the
  measurement above constrains it to the `devenv build shell` family and
  rules out `direnv-export` and `devenv shell`, both of which run
  project code during provisioning.
- **`enterShell` is captured as data and run inside the sandbox**,
  through the existing `activation_script` path, the same way flox's
  `on-activate` is. `ran_activation_hook` stays `false` for devenv,
  because nothing project-defined runs host-side.
- **Store grants** come from the resolved closure's `/nix/store` paths by
  the same mechanism the other three closure providers use, annotated
  `provider:devenv`.
- **Preconditions, checked at `up`, layer `provider`, exit code 3:**
  `devenv` usable and `nix` usable (devenv is a frontend over it) —
  probed as a **capability**, never as a binary on `PATH`, since both
  `devenv --version` and `nix flake --help` succeed against an
  unreachable store, the failure mode this repo's testing rule exists
  for; `devenv.nix` present, whose absence is a missing environment with
  a `devenv init` hint rather than a missing feature; `devenv.lock`
  present **before** capture, checked up front the way devbox's
  `ensure_everything_locked` is, so a project that was never locked is
  told to lock it rather than told that capture resolved something; and
  `devenv.lock` unchanged **after** capture — the same "nothing resolves
  at `up`" rule, enforced the same way devbox's is, by byte comparison
  rather than by predicting which keys devenv needs.
- **Staleness**: fingerprint of `devenv.nix` + `devenv.yaml` +
  `devenv.lock`. Three files rather than two, because `devenv.yaml`
  carries the inputs and can change what resolves without
  `devenv.nix` changing at all.
- **Capture writes into the project tree, which no shipped provider
  does.** devenv keeps its evaluation artifacts under `.devenv/` in the
  project root — the same fact the `cli` delta relies on to rank a bare
  flake last. flox, nix and devbox all capture without touching the
  working tree, so this is a new property rather than a detail, and it
  is named here rather than discovered during implementation. What it
  costs — whether a refused `up` must remove those artifacts, what the
  sample's `.gitignore` carries, and what `sandbox-provisioning`'s
  profile has to grant — is settled in design.md decision 6, on
  measurements group 0 takes.
- **Services are `ServiceSupport::Unsupported` in this change.** See
  Impact; this is a scope decision with a named successor, not an
  oversight.
- **`doctor`** learns a devenv check, scoped to projects declaring
  `provider = "devenv"`, as it already is for the other three.
- **`init`** detects an existing `devenv.nix` and offers `devenv`.

## Capabilities

### New Capabilities

None. devenv is a fourth implementation of the existing `env-provider`
capability, the same reasoning `add-nix-provider` and
`add-devbox-provider` each gave.

### Modified Capabilities

- `env-provider`: adds a "devenv provider resolution" requirement
  (hook-free capture, captured-hook-runs-inside, store grants, lockfile
  and manifest preconditions, three-file staleness), and narrows "Only
  declarative providers" — `devenv` moves from a "not yet supported"
  rejection to an accepted value.
- `config`: `env.provider`'s accepted value set widens to include
  `devenv`.
- `cli`: `doctor` gains a devenv check scoped to projects declaring it;
  `init` detects `devenv.nix`.

## Impact

- **Affected specs**: `env-provider`, `config`, `cli`.
- **Affected code**: `src/provider/devenv.rs` (new), `validate.rs` (one
  name moves out of `NOT_YET_SUPPORTED`), `mod.rs`
  (`ProviderKind::Devenv`, dispatch, `static_name`,
  `manifest_fingerprint`), `doctor`/`init` in `src/bin/devcroft.rs`. No
  changes to lifecycle, exec, ssh, or policy compilation are expected —
  and as with nix and devbox, that absence is a result this change
  exists to demonstrate rather than an assumption it rests on.
- **The one genuine design tension, stated here so design.md cannot
  quietly settle it.** The hook-free artifact `devenv build shell`
  produces opens with its own warning: *"the existence of this path is
  not guaranteed. It is an internal implementation detail for
  pkgs.mkShell."* Consuming it is exactly the shape
  `add-flox-services` decision 1 rejected for flox's generated
  `service-config.yaml`, on the grounds that an undocumented internal
  artifact is not a contract. The difference worth weighing is that
  `nix print-dev-env` carries a comparable caveat and is already relied
  on, and that the alternative here is not a documented route but
  running project code during provisioning. design.md has to make that
  trade explicitly, and record what breaks if devenv changes the format.
- **Where devenv lands under `sandbox-provisioning`, stated rather than
  implied.** The two ship together, and that change's own spec requires
  residual host-side exposure be *stated rather than implied*, so this
  one cannot leave it to be inferred. devenv executes no project shell
  to resolve, so it is not the flox case (fail closed at layer
  `provider`); it falls in that spec's "a provider evaluates
  repository-controlled definitions" scenario, alongside `nix
  print-dev-env --json` and `devbox shellenv --pure` — evaluation runs
  host-side, bounded by the evaluator rather than by the provisioning
  sandbox. One step further than nix's, though, and the step is devenv's
  own frontend: it reads `devenv.yaml` and `devenv.nix` and generates a
  flake before Nix evaluates anything, so what runs host-side is
  devenv's interpreter *plus* the evaluator, not the evaluator alone.
  That is the residual exposure, recorded here so the two changes agree
  rather than each assuming the other settled it.
- **Deliberately out of scope: devenv services and `processes`.**
  devenv's `processes` are process-compose-backed, the same supervisor
  `src/services` already generates a config for, and
  `openspec/config.yaml` records that the *ownership* question —
  devenv's services overlapping devcroft's hooks — is already closed by
  `add-flox-services`: services and hooks are separate mechanisms with a
  stated precedence. What is **not** settled is the same question
  `add-devbox-provider` deferred: where the *declarations* come from.
  flox qualified because it has a documented `[services]` schema in its
  own manifest. Whether devenv's `processes` are readable as a contract
  or only as generated process-compose output has to be measured before
  it is claimed. Until then `provider = "devenv"` with a manifest
  declaring services fails the way `nix` and `devbox` do, which the
  `services` spec already requires be distinguishable from "supports
  services, none declared".
- **Unblocks**: `add-manifestless-mode` (0.6), which exists to be
  pointed at repositories nobody has read. A `devenv.nix` reported as
  unsupported is a poor version of "point it at anything", which is why
  this is wanted before that change rather than after. Detection there is
  that change's own surface: this one adds devenv to `init`'s documented
  order only, so 0.6 has to place it in manifestless detection itself
  rather than inherit it.

## Success Criteria

- A project with `devenv.nix` + `devenv.yaml` + `devenv.lock` and
  `env.provider = "devenv"` comes up; `devcroft exec` sees the devenv
  environment's toolchain; every tool runs under
  `network.default = "deny"`, because materialization happened host-side
  at `up`.
- **`enterShell` does not run during `up`'s provisioning phase**, proven
  by the sentinel method used to write this proposal rather than by
  reading devenv's output: a hook that appends to a file outside the
  project leaves that file untouched across `up`, and appends exactly
  once when the sandbox runs it.
- The captured env diff is byte-identical regardless of the invoking
  shell's own environment, verified the way
  `tests/flox_env_capture_is_deterministic.rs` verifies it.
- A full build inside the sandbox needs no host library grants: project
  root, `/tmp`, and the devenv closure's store root, with `/usr/bin/gcc`
  denied — the same measurement `own-policy-baseline` recorded for the
  other three.
- Editing any of `devenv.nix`, `devenv.yaml` or `devenv.lock` flips
  `status` to stale and `up` prints the `--recreate` notice.
- `policy --render` shows store grants with origin `provider:devenv`;
  provider resolution adds no write grants.
- **`devcroft shell` and an SSH login session both work** — a separate
  claim from `exec` seeing the toolchain, and the one most likely to be
  assumed. `src/shell.rs` has to resolve an absolute `sh` out of the
  devenv closure: from the captured `PATH` only where the hit lies inside
  a path the provider declared in `read_only_grants`, else from the
  closure's requisites. This is the invariant `own-policy-baseline` broke
  silently for all three shipped providers, it depends on what *this*
  provider's capture actually contains, and "it is Nix underneath" is not
  evidence for it.
- **A refused `up` leaves the project's own files as it found them**,
  `devenv.nix`, `devenv.yaml` and `devenv.lock` byte-identical, with
  anything capture writes confined to `.devenv/`.
- **No shared *type* changes shape.** Which shared *files* change is
  known and mechanical — `provider/mod.rs`'s dispatch, `validate.rs`'s
  name lists, `doctor`/`init` in `src/bin/devcroft.rs` — so the claim
  under test is the narrower and checkable one: if devenv forces a change
  to `Resolution`, to `policy::compile`, or to `lifecycle::up`'s provider
  handling, the "trait generalizes" claim is weaker than stated, and that
  is recorded rather than absorbed.

## Open Questions

- **Which member of the `devenv build` family to capture from**, and
  whether its output is stable enough to parse across devenv versions.
  Measured to exist and to be hook-free; not yet measured for stability.
- **Whether `devenv build shell`'s environment is complete.** It carries
  the declared packages, the project's `env`, and the compiler variables.
  Whether it is *identical* to what `devenv shell` would produce, minus
  the hook, has not been diffed — and a capture that silently omits part
  of the environment is worse than one that fails.
- **Whether `devenv.yaml`'s inputs can float.** `devenv.lock` pins them,
  but the "nothing resolves at `up`" rule has to be checked against a
  project whose inputs are unpinned, the way devbox's base nixpkgs entry
  turned out to resolve live.
- **What capture writes into the project tree**, and whether a failed or
  interrupted capture can leave `.devenv/` in a state the next `up`
  reads as valid.
- **Which command creates `devenv.lock`** for a project that has none.
  `devenv update` is the name-based candidate, and a name-based candidate
  is exactly what this proposal exists to avoid asserting.
- **Whether the devenv closure contains a shell `src/shell.rs` can
  resolve**, and whether it is reachable on the captured `PATH` inside a
  declared grant or only through the closure's requisites.
- **Why `devenv shell -- <cmd>` runs `enterShell` twice.** Measured
  consistently, unexplained, and worth understanding before relying on
  any adjacent behaviour.
