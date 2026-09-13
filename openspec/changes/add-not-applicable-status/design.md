# Design — add-not-applicable-status

## Context

The status vocabulary is closed by spec and has four values. A
measurement found a fifth situation it cannot express, so the question is
narrow: add a word, or reuse one.

## Decision 1: a fifth word, rather than stretching `unsupported`

`unsupported` is documented as *"This platform cannot provide it. A
constraint, not a choice"*. Stretching it to cover "the subject does not
exist here" would make one word carry two opposite messages to a reader
deciding whether to develop on a Mac:

| row | `unsupported` means | what the reader should do |
|---|---|---|
| `per-agent-network-namespace` | sandboxes share the host's port table | weigh it; it is a real loss |
| `abstract-unix-sockets` | Darwin has no abstract namespace | nothing |

A table whose cells mean different things depending on which row you are
on is a table people stop trusting, and this one exists precisely because
prose kept collapsing distinctions.

**Cost, accepted:** five words is more to learn than four, and every
added value is a chance to pick the wrong one. Decision 3 is the guard.

## Decision 2: `not-applicable` needs evidence like every other status

The obvious shortcut is to let this one value be asserted — "everyone
knows abstract sockets are Linux-only". That is the inference the
vocabulary's own second distinction exists to refuse, and it would have
been wrong-shaped here even though the conclusion is right: the *first*
probe returned `ENOENT` for a reason that had nothing to do with Darwin
lacking the namespace. Python passes `sun_path` as a C string, so
`"\0name"` becomes an empty path. Only the C probe, building
`sockaddr_un` with an explicit `addrlen` the way Linux actually addresses
the abstract namespace, established the platform fact rather than an
artefact of the binding.

So: absence is a claim, claims cite evidence, and one probe was not
enough to establish this one.

## Decision 3: the word is narrow on purpose, and the scope is deliberately small

`not-applicable` means the capability's **subject** is absent. It does
not mean:

- the capability exists and is unenforced → `unsupported`
- the capability exists and nobody measured it → `unverified`
- the platform offers it and devcroft declined → `not-adopted`

Each of those already has a word, and the failure mode this change most
plausibly causes is a future entry taking the comfortable new word
instead of the accurate old one.

**Only one entry moves.** No sweep of the existing `unsupported` rows is
part of this — task 3.1 re-reads them and is explicitly forbidden from
changing any on the strength of a re-read. If one of them is really an
absent threat, that is a measurement in its own change, made the way this
one was.

## Rejected

- **Omitting the row on macOS entirely.** Tidier, and it loses a fact
  worth having: that the Linux protection has no macOS counterpart
  *because there is nothing to protect against*. A reader comparing
  platforms learns something from the word that a blank does not tell
  them. See the proposal's first open question — this is the reason for
  the choice, not a claim that the alternative is unreasonable.
- **`enforced` with explanatory evidence.** It reads as a protection in
  the summary line, and the summary line is what gets read. An
  enforcement claim with nothing enforcing is the exact substitution
  `Status::Unverified`'s own doc comment says this project has shipped
  before and been wrong about.
