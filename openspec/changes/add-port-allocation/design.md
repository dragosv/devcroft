# Design: add-port-allocation

## Context

See `proposal.md` — Why. The short version: at the `process` tier no
namespace separation exists between sandboxes, so distinct ports have to
be *chosen* rather than isolated. At the `hardened` tier that is only
true when egress is granted; a deny-default hardened sandbox already
gets its own network namespace from `oci_spec::build`, and needs no
allocation at all. **Scope follows the resolved network mode, not the
tier**, and the proposal's earlier blanket claim to the contrary was
wrong.

Two existing facts make this tractable:

- `network.ports` already compiles to a real loopback grant — today
  `nono::CapabilitySet::allow_localhost_port`, via
  `policy::CapabilityPlan` (`use-nono-library`; it was nono-cli's
  `open_port` profile key when this was first written). So granting an
  arbitrary loopback port is solved, and this change decides *which*
  port, not how to permit it.
- devcroft generates the process-compose configuration itself, including
  each service's `environment` block. It therefore controls the value a
  variable carries — without touching the provider's manifest or the
  service's command string.

## Goals / Non-Goals

**Goals:**

- Several sandboxes from one committed manifest run the same service
  without collision.
- The chosen port is discoverable and stable for the sandbox's life.
- Nothing is granted that `policy --render` cannot show.

**Non-Goals:**

- Network namespace separation *as something this change would add*. The
  hardened tier already requests one for deny-default sandboxes
  (`oci_spec::build`); the `process` tier cannot, and this change does
  not try to give it one.
- Rewriting service commands to inject ports. devcroft does not own
  them, and a rewriter that parses arbitrary shell would be both fragile
  and a surprise.
- Allocating anything other than loopback TCP ports.

## Decisions

### 1. Allocate by variable name, not by rewriting commands

The manifest names an environment variable; devcroft picks a port and
that variable carries it. This works because it matches how ports are
already declared in practice: flox's own documented example passes a
service's port through `vars` (`command = "… -p \"$PGPORT\""` with
`vars.PGPORT`), and `vars` is exactly what devcroft materializes into
the generated config's `environment`.

Alternatives considered:

- **Rewrite the port inside the service's command string.** Rejected on
  the property that fails: devcroft would have to parse arbitrary shell
  to find the number, and would be modifying project code it does not
  own. A service that then failed would fail in a command the user never
  wrote.
- **Offset every declared port by a per-sandbox constant.** Rejected:
  it silently changes the meaning of a number the user did write, and
  the user has no way to predict the result — every debugging session
  starts by discovering that 5432 is not 5432.

The consequence is a real limitation, and it is specced rather than
hidden: a service whose port is baked into its command cannot be
allocated, and asking for both fails at `up` naming the service. That is
better than granting a port nothing listens on.

**The request has to name the service, not just the variable.** The
first draft of this design put the request in `[network]` as a flat list
of variable names, and that shape cannot express the failure the
paragraph above promises — with no service attached to a request, "fail
naming the service" has nothing to name. It also makes the detection
rule unimplementable: in a project with `db`, `worker` and `migrate`,
the services that legitimately never reference `DB_PORT` are the
majority, so "fail if some declared service doesn't reference the
variable" fails every real project, while "fail if none does" quietly
passes a service that has the variable in `vars` but hardcodes the port
in its command. Neither is the stated requirement.

So the request is keyed by service *and* variable — the service whose
config devcroft generates, and the variable within it to substitute.
That makes the check local and exact: for that one service, does its
command reference that variable? A request naming a service the
provider never declared is itself an error, and an allocation wanted for
sessions rather than for a service is a separate, service-less form
rather than a special case of this one.

### 2. Sticky for the sandbox's life, recorded in `meta.json`

Re-choosing on every `up` would be simpler and is wrong: a user or agent
that wrote down a connection string would find it silently invalid after
an ordinary restart, with nothing to indicate why. So the chosen port is
recorded alongside the other per-sandbox facts `meta.json` already
holds, and reused while that state exists.

`rm` removing state means the next creation allocates fresh — correct,
since nothing can be assumed about a port that has been unclaimed for an
arbitrary period.

If a recorded port can no longer be granted, allocation falls back to
choosing a new one rather than failing: the recorded value is a
preference, not a contract with the rest of the host.

Two things about that fallback have to be stated, because leaving either
implicit reproduces a mistake this project already made once:

- **"Can no longer be granted" really means "can no longer be bound".**
  The policy grants whatever it is asked to, so nothing at the policy
  layer can refuse a recorded port. The only way to discover it is
  unavailable is to try to bind it — which is decision 4's race, run a
  second time. The fallback is therefore not a separate mechanism; it is
  what happens when the re-bind loses.
- **The change must be announced at `up`, not discovered later.** The
  whole point of stickiness is that a user or agent wrote the port
  down. Silently swapping it invalidates exactly the artifact
  stickiness exists to protect, and leaves the user debugging a
  connection string that was correct yesterday. This is the same
  mechanism-without-visibility error as decision 7 in
  `add-flox-services` — supervision shipped before observability — so
  the spec carries a scenario requiring the change be reported.

**Interaction with decision 4, which weakens this decision more than it
first appears.** Binding `:0` draws from the ephemeral range, so a
recorded port is one the kernel may hand to an unrelated outbound
connection at any time while the sandbox is down. That makes the
fallback *more* likely the longer a sandbox lives and the busier the
host is — i.e. the stability guarantee is weakest precisely on the
many-sandbox hosts that motivate the change. Whether to draw from a
devcroft-owned range instead is the proposal's open question, and it is
load-bearing for this decision rather than cosmetic.

### 3. Discovery is part of the feature, not a follow-up

An allocated port nobody can find is exactly as useful as a port that
collides. This is the same mistake this project already made once with
services — supervision shipped before observability, and the argument
for not auto-restarting turned out to depend on a visibility that did
not exist (`add-flox-services` design decision 7). So `status` reporting
the port is a requirement here, not a later polish task.

### 4. The allocate-then-bind race is accepted and documented, for now

Choosing means binding `:0`, reading the number, closing, and letting
the service bind it later. Between the close and the service's bind,
another process on the host can take the port.

Considered: holding the socket and passing the fd into the sandbox — the
same inheritance trick `up` already uses for the control socket. It
closes the race properly, but it means devcroft opening a listener per
service and handing it to a process it does not control, through a
config format (process-compose) that has no notion of inherited fds.
Disproportionate for the exposure.

Chosen: allocate, and let the failure be visible. A service that loses
the race fails to bind, and — because `add-flox-services` made service
failure visible — shows up as failed with its log rather than
disappearing. That is a worse outcome than winning, but not a silent
one, which is the property that matters.

**Revisit if** this proves common in practice rather than theoretical;
the fix is real, just expensive.

## Risks / Trade-offs

- **A port from the ephemeral range may be re-used by the kernel for an
  unrelated outbound connection** → Mitigation: none in the first cut,
  and flagged as an open question. A devcroft-owned range would be more
  predictable, at the cost of assuming that range is free.

- **Two sandboxes racing to allocate simultaneously could pick the same
  port** → Mitigation: each binds `:0` independently, so the kernel
  hands out distinct ports; the exposure is only between close and
  bind, the same window as decision 4.

- **`status` reporting a recorded port for a stopped sandbox may read as
  "something is listening"** → Mitigation: label it as recorded rather
  than live, or omit it when down. Open question in the proposal.

- **This change alone does not make fan-out work** → Mitigation: state
  it plainly. `add-agent-workload` fixes worktrees sharing a sandbox
  name; without that, two worktrees never get as far as needing two
  ports.

## Migration Plan

Additive and inert: a manifest requesting no allocation compiles to a
byte-identical policy and generates a byte-identical service config,
which is the regression test. `meta.json` gains a field that reads as
empty for sandboxes recorded before it existed — the same posture the
isolation-tier field used.

Rollback is removal of the request key; recorded allocations become
unread rather than invalid.
