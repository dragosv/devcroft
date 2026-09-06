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

`spawn_keeper` already has the right channel. It calls `.envs(env)`, which can
only add or override, and then `env_remove` for each entry in
`Resolution::unset`. That mechanism exists because provider activation can
*unset* a key, and its own comment states the general problem: "a plain map has
no way to represent 'unset' at all."

## Goals / Non-Goals

**Goals:**
- The sandbox's environment is a property of the project, not of the shell.
- The simple credential story, delivered without a new dependency.
- `HOME` writable, so `[hooks]` can install things.

**Non-Goals:**
- Not scrubbing the provider's own output (proposal Non-Goals).
- Not a secrets manager; not brokering.

## Decisions

## D1 — Subtract the ambient, do not allowlist the wanted

**Decision.** Remove every variable whose **name and value both** match
devcroft's own ambient environment, minus an essential set (D2). Keep everything
else.

**Why not an allowlist.** It would have to enumerate what a provider's
activation produces — 74 names here, different per provider, per project and per
lockfile. Any list would be wrong on the next `flox install`.

**Why not a denylist of sensitive names.** It is a guess about what secrets are
called. `CLAUDE_CODE_MESSAGING_TOKEN` would not have been on anyone's list; nor
would the next tool's.

**Why the value comparison and not the name alone.** `PATH` is in both, and the
provider rewrote it — matching on name would delete the closure from the
sandbox's `PATH` and break everything. Comparing values keeps exactly what
activation changed.

**The residual false positive, stated:** a variable the provider sets to
*coincidentally* the same value the shell had is removed. D2's essential set
covers the cases where that matters; beyond it, a project declares the variable
(D3). This is a real if narrow cost, and it is preferred to the alternative,
where a missed name is a leaked credential rather than a missing convenience.

## D2 — The essential set is small, enumerated, and justified per entry

Variables a POSIX session cannot function without, kept even when they match the
ambient environment exactly: `TERM`, `LANG`, `LC_*`, `TZ`, `USER`, `LOGNAME`,
`SHELL`. `HOME` is not on the list because D4 replaces it outright, and `PATH`
is not because the provider always rewrites it and D1 therefore keeps it
already.

**Enumerated in one constant, not decided per call site** — the spec's third
scenario asks for exactly that, because a set that grows by convenience is how
this becomes a denylist again by accident.

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
