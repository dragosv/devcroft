# Change: add-not-applicable-status

## Why

**A capability whose threat does not exist on a platform has no honest
status today, and the matrix has one such entry right now.**

`abstract-unix-sockets` asks whether a sandboxed process can reach an
abstract (`@`-prefixed, pathless) unix socket outside its sandbox — dbus,
X11, PipeWire, systemd-journald on a Linux desktop. It carried
`unverified` on macOS, with the evidence *"Seatbelt has no scoping-ABI
equivalent examined; unmeasured on a macOS host."*

Measured on aarch64-darwin, two independent probes so the result does not
rest on one abstraction's behaviour:

| probe | result |
|---|---|
| Python, `bind("\0devcroft-probe")` | `ENOENT` |
| C, `sockaddr_un` with `sun_path[0] == 0` and an explicit `addrlen` — how Linux actually addresses the abstract namespace | `ENOENT` |

**Darwin has no abstract namespace at all.** It treats `sun_path` as an
ordinary pathname, and an empty one does not exist. So there is nothing
to reach and nothing to scope.

`unverified` is now simply false — this *has* been measured. And none of
the four remaining words fits:

- **`unsupported`** is documented as *"This platform cannot provide it. A
  constraint, not a choice"*, and reads as a loss. macOS users lose
  nothing here; there is no dbus-on-an-abstract-socket for them to be
  exposed to.
- **`enforced`** would be an enforcement claim with nothing enforcing.
- **`not-adopted`** is devcroft declining something the platform offers.
  The platform offers nothing.

The vocabulary's own requirement says it exists "to force three
distinctions prose keeps collapsing". This is a fourth, found by
measuring rather than by arguing: **a constraint versus an absent
threat.**

## What Changes

- **`not-applicable` joins the closed status vocabulary**, for a
  capability whose subject does not exist on that platform — as distinct
  from one the platform cannot deliver.
- **The requirement that fixes the vocabulary is restated** to five
  words and four distinctions.
- **`abstract-unix-sockets` on macOS moves to it**, carrying the two
  probes as its evidence.
- **Nothing else moves.** This is not an invitation to reclassify
  `unsupported` entries as painless; `per-agent-network-namespace` on
  macOS stays `unsupported`, because that one is a real loss — sandboxes
  share the host's port table as a direct result.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `backend-capabilities`: the status vocabulary gains a fifth value and
  the requirement gains the distinction it encodes.

## Impact

- **Affected specs**: `backend-capabilities`.
- **Affected code**: `src/backend_capabilities.rs` — the `Status` enum,
  its `Display`, and one entry. `doctor` renders whatever `Display`
  produces, so its output follows without a change there.
- **A status nobody can act on is the risk**, and it is the reason to
  keep the word narrow. `not-applicable` must mean "the thing this
  protects against does not exist here", never "we could not work out
  what this does". The latter already has a word, and it is `unverified`.
- **Reading the matrix should get easier, not harder.** Five words is
  more to learn than four; the trade is that a macOS reader currently
  has to decide for themselves whether `unsupported` on this row means
  they are exposed. They are not, and the table should say so.

## Success Criteria

- `abstract-unix-sockets` reports `not-applicable` on macOS and is
  unchanged on Linux.
- The vocabulary stays closed — five values, no free text.
- Every entry still cites evidence, `not-applicable` included: "this does
  not exist here" is a measurable claim and this one was measured twice.
- No entry that represents a genuine platform loss is moved to the new
  word. `per-agent-network-namespace` on macOS is the test case.

## Open Questions

- **Whether `abstract-unix-sockets` should appear on macOS at all**, or
  whether a capability with no macOS subject should simply be absent from
  that column. Absent is tidier and loses something: a reader comparing
  platforms would not learn that the Linux protection has no macOS
  counterpart *because there is nothing to protect*, which is a fact
  about the platform worth having. Stated as the reason for choosing a
  word over a blank, not as a settled matter.
- **Whether any other entry is mislabelled the same way.** This one was
  found because it was the last `unverified` on macOS. Nothing has swept
  the `unsupported` rows for entries that are really inapplicable, and
  that sweep is not in this change.
