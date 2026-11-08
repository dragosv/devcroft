# Design: fix-provisioning-hooks

## Context

See `proposal.md` — Why. Everything below is measured against the
tooling in this repo's devcontainer (flox 1.14.0, nix 2.31.5, devbox
0.18.0), not derived from documentation. That distinction earned its
place: three separate claims in this area were inverted by measurement
while investigating it.

The shape of the problem is the same in all three providers, which is
what makes it worth stating as a rule rather than as two bug fixes:

| provider | entry point that runs the hook | entry point that does not |
|---|---|---|
| flox | `flox activate -- <cmd>` | *none found* |
| nix | `nix develop --command <cmd>` | `nix print-dev-env --json` |
| devbox | `devbox run -- <cmd>` | `devbox shellenv` |

Every provider offers a "run something inside the activated shell" door
and every one of those doors runs the hook. Two of the three also offer
a "hand me the environment" door that does not. devcroft walked through
the wrong door in both providers it has shipped — in nix's case
deliberately, having considered and rejected the right one.

## Goals / Non-Goals

**Goals:**

- Stop executing project code during nix resolution, with no change to
  the environment actually captured.
- Make the flox case visible, since it cannot be made to stop.
- Correct the invariant text that currently asserts the opposite of
  what the code does.

**Non-Goals:**

- Failing `up` on a flox project that uses `on-activate`. It is
  idiomatic flox, the user's own `flox activate` runs it too, and
  rejecting it would break a large share of real projects to defend
  against a threat the user already accepts elsewhere.
- Sandboxing provider resolution. Materialization genuinely needs the
  network and the store; confining it is a much larger change and does
  not belong bundled with a capture-mechanism swap.
- Capturing `bashFunctions`. The current mechanism cannot see them, so
  ignoring them preserves today's behavior exactly; adopting them is a
  separate additive decision.
- Fixing devbox. It is not implemented yet, and
  `add-devbox-provider` already picks the correct mechanism.

## Decisions

### 1. nix switches to `print-dev-env --json`, not to `print-dev-env`

The obvious fix is wrong, and this is the third claim in this
investigation that measurement inverted. `nix print-dev-env` in its
default (shell-script) form does **not** run the hook when generating
the script — but the script it emits ends with:

```
eval "${shellHook:-}"
```

so evaluating it runs the hook after all. A switch to
`print-dev-env` + `eval` + `env -0` would have looked like a fix,
passed a naive review, and changed nothing. Confirmed by sentinel: the
hook ran.

`--json` is the actual fix. It emits
`{"bashFunctions": {...}, "variables": {...}}`, where each variable
carries a `type` (`exported`, `var`, or `array`) and a `value`. The
`shellHook` appears as an ordinary entry —
`{"type": "exported", "value": "…"}` — inert data devcroft never
evaluates. Measured: the hook did not run.

It is also better on grounds unrelated to this change. There is no
shell in the pipeline at all, so the quoting and multi-line-value
fragility that made `nix.rs` reject `print-dev-env` in the first place
does not apply to the JSON form. And devcroft already links
`serde_json`, so the parser is free.

**Equivalence, measured rather than assumed:** taking the variables
whose `type` is `exported` yields 74 keys against the current
mechanism's 74. The only differences are shell bookkeeping the current
mechanism picks up as noise — `PWD`, `SHLVL`, `_`, `OLDPWD` — plus
`TERM`/`TZ`/`NIX_CONFIG`, which the fixed baseline diff already
handles. Dropping `SHLVL` and `_` from a captured environment is a
small improvement, not a regression.

Alternative rejected: post-processing the shell form to strip its
trailing `eval` line. It works, but it makes correctness depend on
matching one line of nix's output format, and it fails open — a
reworded line means the hook silently runs again. The JSON form has no
such failure mode.

### 2. flox is reported, not refused

No `flox activate` mode suppresses `on-activate`: measured across the
default, `--mode run`, `--mode dev`, and `--no-start-services`. flox's
help says the `<cmd>` form "does not run any profile scripts", which is
accurate and covers `[profile]`, a different manifest section from
`[hook]`.

So the choice is refuse, or report. Refusing is wrong here:

- `on-activate` is how flox environments do setup — creating a venv,
  generating a config. Rejecting it would reject a large share of real
  flox projects.
- The user running `flox activate` by hand gets exactly the same
  execution. devcroft would be refusing to do what the user's own
  workflow already does.
- devcroft's default provider is flox. A rule that rejects the common
  case of the default provider is a rule that gets disabled.

Reporting matches an existing contract rather than inventing one:
"degraded capabilities are surfaced, never silent — exactly one warning
naming the aspect, the reason, and the fallback."

The warning is deliberately **not** suppressible. A flag to silence a
warning about something still true is how this went unnoticed for two
providers and one full release cycle.

### 3. Detection reads the manifest, and is allowed to be imprecise

Deciding whether to warn means knowing whether `[hook].on-activate` is
present in `.flox/env/manifest.toml`. devcroft already parses TOML and
already reads that file for fingerprinting, so this is a key lookup, not
new machinery.

Imprecision is acceptable in one direction only. A false negative —
staying silent when a hook exists — defeats the change, so detection
errs toward warning: an unparsable manifest, or one whose shape is
unexpected, warns rather than assuming safety. A false positive is
merely noise.

Deliberately not attempted: deciding whether a given hook is
*dangerous*. That is unanswerable — it is arbitrary shell — and
attempting it would produce a filter users learn to distrust.

## Risks / Trade-offs

- **The JSON capture may differ from the shell capture in a way the
  74-key comparison did not surface** (a project whose dev shell sets
  something exotic) → Mitigation: the equivalence assertion is a test
  against a real flake, and the "no hook, unchanged environment"
  scenario in the spec is a regression test for exactly this.

- **`print-dev-env --json` is a newer interface than `nix develop` and
  could be less stable across nix versions** → Mitigation: it is a
  documented flag with a structured contract, which is easier to detect
  breakage in than a shell-script format; a parse failure is loud, where
  a shell-format change fails silently. Recorded as a real trade-off
  rather than dismissed.

- **Warning fatigue: every flox project using `on-activate` now prints a
  warning on every `up`** → Mitigation: none available that does not
  reintroduce the invisibility. This is the cost of the honest position,
  and the open question about suppressibility is where it gets revisited
  if it proves unbearable in practice.

- **Users may read the warning as "devcroft is unsafe" and conclude more
  than is true** → Mitigation: the wording says what ran and where, not
  a severity judgement. The README entry gives the context the one-line
  warning cannot.

## Migration Plan

No state migration. A sandbox created before this change and brought up
after it re-resolves through the new mechanism; if the captured
environment differs, the existing staleness machinery already reports
it and `--recreate` already resolves it.

Rollback is reverting the capture mechanism, which reintroduces the
defect — so the test asserting the hook does not run is the thing that
must not be deleted along with any future refactor.
