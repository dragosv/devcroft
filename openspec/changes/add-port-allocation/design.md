# Design: add-port-allocation

## Context

See `proposal.md` — Why. The short version: no namespace separation
exists between sandboxes and none is coming (rootless gVisor rejects
`--network=sandbox`), so distinct ports have to be *chosen*, not
isolated.

Two existing facts make this tractable:

- `network.ports` already compiles to nono's `open_port`, so granting an
  arbitrary loopback port is solved. This change decides *which* port,
  not how to permit it.
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

- Network namespace separation. Rejected upstream and not revisited
  here.
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
