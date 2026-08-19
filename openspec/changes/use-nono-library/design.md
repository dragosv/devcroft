# Design: use-nono-library

## Context

See `proposal.md`. This file records what was measured, so a later
reader can tell which parts are findings and which are judgement — and
re-derive the findings when the upstream numbers move.

Measured against `nono` 0.74.0 and this repo at commit `f507f07`:

```sh
curl https://crates.io/api/v1/crates/nono/0.74.0/dependencies   # optional flags
cargo tree --edges normal                                       # both trees
curl https://api.github.com/repos/nolabs-ai/nono/contents/crates/nono/src
curl https://api.github.com/repos/nolabs-ai/nono/contents/crates/nono-cli/src
```

| | value |
|---|---|
| `nono` resolved tree | 189 crates |
| devcroft resolved tree | 158 crates |
| shared | 48 |
| net new for devcroft | 141 |
| `nono` normal deps declared `optional = true` | 1 (`keyring`) |
| probe build, cold, no `cmake` present | 15.7s wall |

## Goals / Non-Goals

**Goals:**

- Pin the backend in `Cargo.lock` rather than in a hand-maintained
  version range.
- Remove the intermediate process between `up` and the keeper.
- Stop requiring an external binary at runtime for the process tier.

**Non-Goals:**

- The hardened tier. gVisor is a separate backend reached through
  `SessionBackend`; nothing here touches it.
- Reimplementing any part of nono. If something needed is missing from
  the library, the answer is an upstream request, not a local copy —
  see Decision 3.
- Owning policy content. That is `own-policy-baseline`, and it is a
  prerequisite rather than a part of this change.

## Decision 1: self-restriction replaces the wrap hop

**What:** the keeper, after inheriting its listener fds, calls the
library's apply-to-self entry point. `spawn_keeper` drops the `nono
wrap -p <profile> --` prefix and spawns the keeper binary directly.

**Why:** it is what the architecture already describes. The stated
invariant is that the sockets are created first and the keeper applies
the profile *to itself*, and the only reason a foreign process is in the
middle today is that self-application was only available through a
binary. The library's `Sandbox::apply_auto(&caps)` applies to the
current process and is irreversible — the same semantics, minus the
exec.

**What this removes:** the fd numbers currently travel as argv through
`nono wrap` into the keeper, and the SSH key material travels as
environment variables across the same boundary. Neither has to cross a
foreign process afterwards. That does not make the current arrangement
wrong — it works and is tested — but it does mean one fewer place where
an upstream change could break it.

**What must be proven, not assumed:** that restriction still happens
after the listeners exist and before anything project-supplied runs. The
ordering is the whole safety argument, and moving where restriction
happens is exactly the change most likely to get it wrong. It is the
first success criterion for that reason.

## Decision 2: the policy stays a rendered artifact

**What:** `policy::compile` continues to produce devcroft's annotated
representation with origins. Conversion to the library's capability set
is a projection from it, exactly as `to_nono_profile` is today.

**Why:** the origin model is devcroft's, not the backend's, and
`policy --render` plus `why` are built on it. Handing the library a
capability set changes the projection target and nothing else. A design
that built the library's types directly and rendered *from those* would
lose the origins, which is the one thing rendering exists to show.

**Side benefit worth stating:** rendering stops depending on a backend
binary being installed. Today `policy --render` describes a policy
devcroft can compile but cannot fully account for; after
`own-policy-baseline` it accounts for all of it, and after this change
it can do so on a machine with no backend at all.

## Decision 3: missing library capability is an upstream request

**What:** if the library lacks something devcroft needs, the response is
an upstream issue or PR, not a local reimplementation.

**Why:** the failure mode this guards against is specific and likely.
The library deliberately excludes the profile and group machinery; a
devcroft that starts filling gaps locally ends up maintaining a partial
copy of `nono-cli` with none of its testing. `own-policy-baseline`
already draws the line in the right place — devcroft owns the ~96 paths
its own sandboxes need, and does not own a general system-paths catalog.
This decision keeps that line from eroding.

**The known request:** gate the trust module behind a Cargo feature, or
publish enforcement separately from verification. That single change
retires this proposal's only unresolved objection, which makes it the
highest-value thing to ask for.

## Decision 4: not settled — whether the trust dependency is acceptable

**Deliberately left open.** The argument each way:

*For accepting it:* nothing calls the Sigstore paths. Unused code in a
linked library is not a capability the process exercises, and devcroft
already links substantial crypto through russh. The keeper's network
restriction is enforced by the sandbox, not by what is absent from the
binary — a linked HTTP client behind an active network denial cannot
reach anything.

*Against:* the argument above proves the risk is contained, not that it
is zero, and it applies to the process tier whose own framing is
"accident protection, not a security boundary". Adding 141 crates —
including a TUF client and a second TLS stack — to the process whose
defining property is having no network is the kind of thing that should
be decided explicitly rather than justified after the fact. Supply-chain
surface is also a real cost independent of runtime behavior.

The honest position is that this is a judgement about acceptable
dependency surface, not a technical unknown, and it belongs to whoever
owns the project's dependency policy. Recording both sides is what this
change can usefully contribute.

## Risks

- **Restriction ordering regression.** The highest-consequence risk, and
  the reason this is not a mechanical refactor. Mitigated only by tests
  that assert the socket is reachable from outside after restriction —
  which exist, and must be run against the new arrangement rather than
  assumed to still apply.
- **Feature unification.** Linking `nono` pulls `rustls` with its
  default features into the shared dependency graph. devcroft cannot
  turn off default features of a transitive dependency, so the crypto
  provider is decided by nono, not devcroft. Worth checking before
  committing, since it can affect the whole binary and not just the
  backend path.
- **Losing an escape hatch.** An external binary can be swapped,
  version-matched, or patched by a user without rebuilding devcroft. A
  linked library cannot. For a tool whose users are exactly the kind of
  people who patch their sandboxing layer, that is a real if minor loss.
