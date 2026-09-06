# Design — Own the Sandbox Environment

## Context

Measured before designing, on a real flox session (macOS 15.7.4):

| | count |
|---|---|
| variables reaching the sandbox | 180 |
| **identical name *and* value to the invoking shell's** | **101** |
| names shared but value changed by the provider | 5 |
| contributed only by the provider | 74 |

So the shell contributes a clear majority, and it does so *verbatim* — which is
what makes the fix precise rather than heuristic.

**And the environment devcroft *wants* was already being computed.** Every
provider runs activation under `capture::canonical_base_env()` — the real `HOME`
and a canonical `PATH`, nothing else — with `.env_clear()`. That guarantee is
explicitly about reproducibility: the same manifest must resolve the same way
whoever runs `up`.

`spawn_keeper` then built its `Command` without clearing, so the operator's
shell was layered back on top of that result. The 101 variables are not a
missing filter; they are a discarded guarantee.

## Goals / Non-Goals

**Goals:**
- The sandbox's environment is a property of the project, not of the shell.
- The simple credential story, delivered without a new dependency.
- `HOME` writable, so `[hooks]` can install things.

**Non-Goals:**
- Not scrubbing the provider's own output (proposal Non-Goals).
- Not a secrets manager; not brokering.

## Decisions

## D1 — Do not inherit at all. The clean environment was already being computed
and then discarded

**Corrected during implementation, and the correction is the whole change.** The
first version of this decision proposed subtracting the ambient environment by
comparing names and values. That was solving a problem devcroft had already
solved one layer down and then thrown away.

**Every provider already runs activation under a fixed environment.**
`capture::canonical_base_env()` returns exactly two variables — the real `HOME`
and a canonical `PATH` — and `flox.rs`, `nix.rs` and `devbox.rs` each invoke
activation with `.env_clear().envs(base)`. Its doc comment says why, and it is
this change's argument almost verbatim: a `PATH` full of one operator's tools
"leaking into either side means the exact same manifest can resolve a different
activation diff depending on who ran `up` and from which shell".

So `resolution.env` **is** the authoritative environment: derived from the
manifest and lockfile, not from a shell. The 101 leaked variables never came
through it. They came from `spawn_keeper` building a `Command` and letting it
inherit, which put the operator's shell back on top of a result that had been
carefully computed without it.

**Decision.** `cmd.env_clear()` before `.envs(env)`. The keeper starts from
nothing and receives exactly what the provider resolved plus what devcroft sets
explicitly.

**Why this is better than subtraction, not merely simpler.** Subtraction is a
*comparison* and therefore has false positives and false negatives: a provider
variable coincidentally equal to an ambient one is wrongly removed, and a
variable the shell exported that activation also happens to set is wrongly kept.
`env_clear` has neither, because it is not deciding anything — it is declining
to add a source that was never wanted.

**What it costs.** Nothing the keeper reads is left to inheritance: everything
devcroft-internal is set explicitly on the same `Command`, and `HOME` arrives
through `resolution.env` because `canonical_base_env` puts it there. The risk is
not in the mechanism but in what *sessions* turn out to depend on, which is why
task 0.3 enumerates the breakage before any of it is fixed.

## D2 — There is no essential set

**Superseded by D1.** An essential set existed to protect variables that
subtraction would wrongly delete. `env_clear` deletes nothing that was wanted:
what a session needs comes from the provider's activation, which is the thing
the project actually declared.

If a session turns out to need something no provider supplies — a terminal type,
a locale — that is a gap in what devcroft passes down, to be added explicitly
and named, not a list of shell variables to preserve by accident. Task 0.3 is
what turns that from a guess into a list.

## D3 — `[env] forward` is the declared exception, and the simple credential path

```toml
[env]
forward = ["ANTHROPIC_API_KEY", "GITHUB_TOKEN"]
```

**Decision.** Names only, values from the host, listed in a committed file.

**Rationale.** This is the whole of the "simple variant": a credential reaches
the sandbox because the project said so, in a place a reviewer reads, instead of
because someone's shell happened to have it. That is a smaller claim than
brokering and a much larger improvement than today.

**Missing variables warn rather than fail** (spec scenario). A forwarded
variable is a convenience, and failing `up` would make a project unbuildable on
a machine that simply does not set it — the opposite of the reproducibility this
project exists for. Note the deliberate contrast with `adopt-nono-proxy`, where
a *brokered* route's missing credential **does** fail `up`: there the route is
the mechanism the sandbox depends on, and starting without it guarantees a
confusing failure later.

## D4 — `HOME` moves into the artifact directory

**Decision.** `HOME=<project>/.devcroft/<name>/home`, created at `up`.

**Rationale.** Today `HOME` names a directory the baseline denies, so every tool
that writes to it fails — which is a bug independent of this change and the
reason an agent's official installer cannot run inside a sandbox at all. The
artifact directory is already the one place the sandbox both reads and writes,
already ignored by `init`, and already removed by `rm`, so lifetime and
cleanliness come for free.

**What this unlocks, and where it belongs.** `[hooks] post_create` running an
agent's *official installer* — the vendor's own script, inside the boundary,
under the project's own policy — becomes possible. That is a far cheaper answer
to `add-agent-workload`'s tooling problem than devcroft shipping runtimes, and
it is recorded as a task there rather than built here.

**Consequence to state:** `~/.gitconfig`, `~/.npmrc` and similar stop being
visible. They were already unreadable — the baseline denied them — so nothing
that worked stops working; what changes is that the failure becomes "no config"
rather than "permission denied", which is the better error of the two.

## D5 — `forward_agent`: implement, because closing the environment makes it cheap

**Decision.** Implement rather than remove. Once D1 lands, `SSH_AUTH_SOCK` stops
arriving by accident, so the key becomes a single place that adds the variable
back and grants the socket path.

**Rationale.** The scenario already exists in `add-mvp-core`'s `ssh` spec and
would otherwise have to be deleted, which is a documentation change dressed as a
decision. And the honest reading of today's state is that the requirement was
never implemented — not that it was implemented and regressed.

**Not measured, and it gates the "on" half:** whether the host agent socket is
*reachable* from inside once granted. macOS classifies unix-socket `connect` as
a network operation, so `network.default = "deny"` may refuse it regardless.
Task 4 measures it before anything claims agent forwarding works.

## Risks / Trade-offs

- **[Risk] This breaks working projects, by design.** A project relying on an
  undeclared variable stops seeing it. → **Mitigation**: the failure is
  diagnosable (spec requirement 3), the remedy is one manifest line, and the
  change is worth an entry in the README rather than a silent minor release.
- **[Risk] The essential set is wrong somewhere**, and something ordinary
  breaks. → **Mitigation**: it is one constant with a test per entry, and the
  cost of a miss is a missing convenience with a named remedy, not a leak.
- **[Trade-off] `HOME` moving is a visible behaviour change** beyond the
  security fix, folded into the same release. Kept together because a closed
  environment with an unwritable `HOME` is a worse place to stop than either
  end.

## Open Questions

1. **Does `status` or `doctor` report the removed set, or a dedicated command?**
   The spec requires it be *reportable*; where it lives is not obvious, and
   putting it in `status` risks burying it in output nobody reads at the moment
   they need it.
2. **Should `forward` support renaming** (`forward = ["HOST_KEY:SANDBOX_KEY"]`)?
   Not needed for anything today, and it is the kind of surface that is easier
   to add than to remove.
