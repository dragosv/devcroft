# Add Backend Capabilities

**Blocks:** `add-linux-agent-fleet`, which declares the capabilities it requires
rather than assuming a backend provides them.

**Unblocks:** `remove-gvisor-backend`'s one remaining task, which has been open
since that change shipped precisely because this one did not exist.

## Why

devcroft makes claims about what it enforces — in the README, in
`docs/threat-model.md`, in `docs/decisions.md`, in `policy::degraded`'s warnings
and in `doctor`'s output. Today those claims live as **prose, in five places,
maintained by hand**. They drift, and this project has now watched them drift
repeatedly rather than predicting that they might:

- The README said domain filtering "compiles to a plain network block". True
  when written; false once `add-egress-proxy` shipped. Three documents
  disagreed about whether it worked until someone read the code.
- `policy::degraded` states macOS domain filtering is cooperative. The pinned
  library's own doc comment for the same code path reads as *enforced*. Nobody
  knows which is right, because no one has a macOS host — and the claim has
  been shipping either way.
- `docs/decisions.md` claimed raw sockets bypass the allowlist on every
  platform. A live test refuted it for Linux, and the entry carried the wrong
  claim until someone happened to test it.
- CLAUDE.md, the README, and a change spec each independently described the
  isolation tier situation, and all three needed editing when one tier was
  removed.

The pattern is the same every time: a claim is true when written, the code
changes, and nothing forces the prose to change with it. `docs/threat-model.md`
already anticipates the fix and points at a change that does not exist —
"What any given backend can and cannot do ... is declared data, not prose: see
`add-backend-capabilities`. Prefer that matrix over any caveat written here or
in the README, and treat a discrepancy as a bug in the prose."

## What this change is now, and what it was

**It was:** a matrix comparing what the `process` tier could do against what the
`hardened` tier could do. That question died with the tier
(`remove-gvisor-backend`), and rebuilding the change as written would produce a
matrix with one column.

**It is:** a single, machine-readable declaration of what devcroft enforces —
which capability, on which platform, at what strength, and verified how. With
one backend the interesting axis is no longer *backend versus backend* but
**offered versus adopted**: what the sandbox library provides, and what devcroft
actually uses.

**The axis has already paid for itself, before the change is built.** An
audit of what `nono`/`nono-proxy` offer against what devcroft uses turned up
three capabilities answering requirements devcroft had already written as
open — credential brokering for `add-agent-workload`, approval hooks for
`add-agent-interaction`, and audit integrity for that change's durable
record — plus one property devcroft's own proxy was missing and had recorded
as satisfied: a per-session token, without which a loopback proxy is an open
relay. None of that was discoverable from the prose, which is the argument
for the matrix in one paragraph.

That axis turns out to be wide. devcroft sets exactly one of the library's
capability knobs (`signal_mode`) and inherits the defaults for the rest —
`ProcessInfoMode`, `IpcMode`, resource limits, snapshot/undo, the keystore —
without naming them anywhere. Those defaults are currently sensible, which is
the problem: nothing records that devcroft depends on them, so a library upgrade
could change a security-relevant default and no test, document or review step
would notice.

## What Changes

- **NEW** `backend-capabilities`: a declared capability set, each entry carrying
  what it is, its status on each platform, and how that status was established.
- **Status is a small closed vocabulary**, not free text: `enforced`,
  `enforced-with-named-degradation`, `unsupported`, `not-adopted`,
  `unverified`. The last two are the ones prose keeps blurring — "we don't do
  this" and "we think this works but nobody measured it" are different facts
  with different consequences.
- **Every entry names its evidence.** A capability claimed as `enforced` cites
  the test or live measurement that established it. An entry whose evidence is
  "it seemed to work" is `unverified` by definition.
- **`doctor` reports the matrix against the current host**, so the difference
  between "devcroft supports this" and "this host can do it" stops being
  something a user has to infer.
- **The prose defers to it.** README, `docs/threat-model.md` and
  `docs/decisions.md` stop restating capability claims and point at the matrix,
  which is what `threat-model.md` already says they should do.

## Capabilities

### New Capabilities

- `backend-capabilities`: the declaration itself, its vocabulary, its evidence
  requirement, and how it is surfaced.

### Modified Capabilities

- `cli`: `doctor` gains the host-versus-declared comparison.

## Impact

- Affected specs: new `backend-capabilities`; modified `cli`.
- Unblocks `remove-gvisor-backend`'s task 4.1 and `add-linux-agent-fleet`'s
  dependency on a matrix to declare requirements against.
- Prose in README/`threat-model.md`/`decisions.md` becomes shorter and
  authoritative-by-reference rather than by repetition.

## Non-Goals

- **Not a compatibility promise.** The matrix records what is true, including
  where the answer is "nobody has checked". It is not a support commitment.
- **Not a second policy engine.** It describes what devcroft can enforce; the
  compiled policy still decides what a given sandbox does enforce. A capability
  being `enforced` says nothing about whether a manifest requested it.
- **Not a substitute for `policy --render`.** That shows one sandbox's actual
  rules; this shows the ceiling those rules are drawn from.
