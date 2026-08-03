# Design: add-mvp-core

## Context

Landlock and Seatbelt restrictions apply to a process tree and are inherited
by children; there is no API to join an existing sandbox from outside
(no `docker exec` equivalent). Therefore any long-lived, multi-session
sandbox must be built around a resident process that was started inside the
boundary and that spawns sessions on request.

## Decision 1: Keeper as spawn server

The keeper is a small resident process. The supervisor (the `devcroft` CLI
in `up`) performs, in order:

1. Create the unix listener socket(s) in the state dir.
2. Resolve the environment (provider activation script, PATH, env vars).
3. Spawn the keeper, passing the listener fds via fd inheritance.
4. The keeper applies the compiled sandbox profile to itself (via nono's
   library path or by re-exec under `nono run` with fd passing).
5. From this point the keeper — and every session it forks — is inside the
   boundary and cannot widen it. The sockets remain reachable from outside
   because they were created before restriction.

Rejected alternative: applying the sandbox per-session (wrap each `exec` in
`nono run`). Simpler, but sessions would not share state, activation cost
would be paid per command, and the SSH endpoint could not live inside the
boundary.

## Decision 2: Environment resolution happens once, at `up`

`flox activate` is executed once by the supervisor to capture the resolved
environment (env diff approach: run activation, diff `env -0` before/after).
The captured environment is injected into the keeper's process environment
before sandbox application. Sessions inherit it for free.

Consequence: changing `manifest.toml` of the flox env requires
`devcroft up --recreate`. This is acceptable and mirrors devcontainer
rebuild semantics.

Rejected alternative: activating per-session inside the sandbox. Requires
read access to flox's own state dirs and pays activation latency per
session; the profile must then allow flox internals forever.

## Decision 3: SSH via embedded server + ProxyCommand, not system sshd

The keeper embeds an SSH server (russh) listening on a unix socket. Client
access goes through `ProxyCommand devcroft proxy %n`, which connects the
socket to stdio. A single wildcard block in `~/.ssh/config` covers all
sandboxes:

    Host *.devcroft
      ProxyCommand devcroft proxy %n
      IdentityFile ~/.local/share/devcroft/id_ed25519
      StrictHostKeyChecking no
      UserKnownHostsFile /dev/null

Host keys are ephemeral per sandbox; authentication is the devcroft client
keypair generated on first run. The socket's filesystem permissions (0700
state dir) are the real authentication boundary; the SSH layer exists for
protocol compatibility with editors, not for network security. The endpoint
MUST NOT listen on TCP in MVP.

Rejected alternative: system sshd with ForceCommand wrappers. Requires root
configuration, pollutes global sshd config, and cannot place the server
itself inside the boundary.

## Decision 4: Policy compilation is explicit and inspectable

The manifest compiles deterministically into a nono profile JSON. The
compiled artifact is written to the state dir and printable via
`devcroft policy --render`. `devcroft why` delegates to `nono why` with the
compiled profile. Nothing is passed to the backend that cannot be shown to
the user.

## Decision 5: Fleet semantics in MVP are "N independent sandboxes"

MVP provides no PID/mount/network namespace separation between sandboxes;
two sandboxes can see each other's processes (Landlock does not hide them).
This is documented as a known limitation. The state-dir layout, socket
naming, and keeper design are chosen so namespace hardening can be added
later without breaking the CLI contract.

## Risks

- nono CLI/profile schema is pre-1.0 and moving; pin a tested version range,
  fail `doctor` outside it.
- macOS Seatbelt cannot enforce domain-level network allowlists without a
  cooperative proxy; MVP surfaces this as a degraded capability at `up`.
- fd passing across `nono run` re-exec must be verified on both platforms
  early; it is the load-bearing trick of the whole design.
