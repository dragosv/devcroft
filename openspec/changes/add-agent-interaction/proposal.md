# Add Agent Interaction

**Depends on:** nothing hard. The attention half needs only devcroft's own
state and the keeper's existing control socket. The approval half adopts
`nono::supervisor`, which is already a dependency.

**Matters most for:** `add-linux-agent-fleet`, and it is deliberately being
specified *before* fleet builds its supervisor, because retrofitting a
per-agent state channel afterwards is more expensive than designing the agent
record with one.

## Why

devcroft can start an agent and confine it. It has no answer for the agent
**stopping and needing something** — which is the normal case, not the
exception, and is what makes a fleet either useful or a set of terminals to
babysit.

Two different things get called "the agent is stuck", and conflating them is
why neither is handled today:

**1. It hit the boundary.** The agent tried to read a path or reach a host the
policy does not grant. Today that is a bare `EACCES` or a refused connection.
The agent has to infer what happened from an errno, and agents infer badly:
they retry, they work around, they conclude the tool is broken, or — worst —
they succeed at something subtly different from what was asked. The operator
learns nothing unless they go looking.

**2. It needs a judgment.** The tests contradict the spec. Two approaches look
equally defensible. The next step deletes something. This is not a policy
question at all and devcroft must not pretend to answer it — but the agent has
no way to *say so* except by writing it to a terminal nobody is watching.

At N=1 both are tolerable, because you are looking at the agent. At N=5 —
which is the entire premise of fleet — "start them and come back later"
collapses if finding the blocked one means attaching to five sessions in turn.

**The mechanism for (1) already exists and devcroft does not use it.**
`nono::supervisor` provides exactly this: a child sends a `CapabilityRequest`
over a socket, the supervisor consults an `ApprovalBackend`, and a granted
path comes back as a file descriptor via `SCM_RIGHTS`. Its own documentation
notes the decision "may block (e.g., waiting for user input or a webhook
response)". There is even a `UrlOpenRequest`, which is the shape an OAuth login
needs — and subscription-based agent auth is precisely that. This is one of the
`not-adopted` entries `add-backend-capabilities` records.

## What Changes

- **NEW** `agent-attention`: a sandbox can be in a state that says *a human is
  needed*, with a message the agent supplied. Reported by `ps` and `status`, so
  finding the blocked agent in a fleet is a listing rather than a search.
- **NEW** `capability-approval`: a denied capability can become a *request*
  instead of a bare denial — bounded by a declared envelope, denied by default,
  and inspectable.
- **MODIFIED** `cli`: `ps`/`status` surface attention; a command the agent runs
  inside the sandbox to raise and clear it.
- **MODIFIED** `policy`: an enabled approval envelope appears in
  `policy --render`, because a mechanism that can widen policy at runtime is a
  rule and must be visible as one.

## What this deliberately does not do

- **It does not interpret the agent's question.** devcroft carries a message
  and a state. What "needs a decision" means is the agent's business, and any
  attempt to classify it would be devcroft guessing about a model's reasoning.
- **It does not make approval a way around the policy.** An approval channel
  that can grant anything is a worse sandbox than no approval channel, because
  it looks confined. The envelope is declared in the manifest, denials are the
  default, and a timeout is a denial (`nono`'s own contract says the supervisor
  should treat expiry as one).
- **It does not add a notification transport.** No email, no webhook, no
  desktop notification. `ps` reporting the state is the contract; anything that
  wants to poll it can.

## Capabilities

### New Capabilities

- `agent-attention`: the state, how an agent sets and clears it, and how it is
  reported.
- `capability-approval`: bounded runtime capability requests, their envelope,
  and their audit trail.

### Modified Capabilities

- `cli`: `ps`/`status` reporting, and the agent-facing command.
- `policy`: rendering the approval envelope.

## Impact

- Affected specs: new `agent-attention`, `capability-approval`; modified `cli`,
  `policy`.
- The two halves are **independently shippable**, and should ship in that
  order: attention needs no new dependency and is what fleet's agent record
  must account for; approval adopts `nono::supervisor` and can follow.
- `add-linux-agent-fleet`: its `agent-supervisor` state record gains attention
  as a first-class field rather than bolting it on later.
- `add-agent-workload`: unchanged. That change is about what an agent *is*
  (tooling, credentials, identity); this is about how it *communicates*.

## Non-Goals

- Interpreting, classifying, or routing the agent's question.
- Any transport beyond devcroft's own reporting surface.
- Approving anything outside a declared envelope, or defaulting to allow.
