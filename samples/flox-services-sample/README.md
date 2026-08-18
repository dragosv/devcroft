# flox-services-sample

Demonstrates two things that are easy to conflate: **granting a loopback
port** (implemented, works today) and **devcroft supervising declared
services** (not implemented — `add-flox-services` task group 3). This
sample deliberately shows both, including the gap, rather than implying
services are done.

## What works today

`.flox/env/manifest.toml` declares a service the documented way, with its
port supplied through `vars` rather than hardcoded:

```toml
[services]
api.command = "python3 -m http.server $API_PORT --bind 127.0.0.1"
api.vars.API_PORT = "8710"
```

`devcroft.toml` grants that one loopback port while keeping egress
denied:

```toml
[network]
default = "deny"
ports = [8710]
```

Those two axes are independent, which is the point. Verified against this
sample on a live sandbox:

```
$ devcroft exec -- python3 -c "…bind(('127.0.0.1', 8710))…"
  8710 BOUND (granted)
  8711 denied correctly
```

Egress stays blocked (`policy --render` shows `network.block: true`), and
an ungranted port still fails with `EPERM` — it is an allowlist, not a
blanket unlock.

Before `network.ports` existed, the only way to let anything listen was
`network.default = "allow"`, which drops egress filtering entirely. That
was published as a limitation of the policy model itself; it was not.
nono's profile schema has always carried an `open_port` field and
devcroft simply never emitted it. [samples/nix-go-sample](../nix-go-sample/)
was migrated off that workaround the same way, and its `devcroft.toml`
comment records the before/after.

## How it runs, and what is still missing

devcroft starts the declared services itself. At `up` it reads the
documented declarations, generates a process-compose config it owns
(`.devcroft/services.yaml` — devcroft's artifact, not flox's internal
one), and the **keeper** runs process-compose as a supervised child.
The keeper owns their lifetime because `up` cannot: `up` exits, and
anything it started over the control socket would be escalated seconds
later.

```
$ devcroft up
$ devcroft exec -- curl -s localhost:8710    # the service answers
$ devcroft down                              # and is gone from the host
```

Teardown is the part worth trusting only after seeing it: services are
registered in the same registry interactive sessions use, so the
existing shutdown handler terminates their whole process group. Verified
by process absence, not by a stop command's exit code — during
development, killing process-compose alone left its child running and
holding the port.

Still missing (`add-flox-services` groups 5–6): `ps`, `logs`, and
`status` do not yet show per-service state. process-compose runs with a
unix socket for its API (`.devcroft/services.sock`) precisely so those
can query it later — it is not started with `--no-server`, even though
that would also work, because the socket keeps that door open.

Two things you may hit:

- **process-compose must be in the environment.** devcroft fails at
  layer `provider` if services are declared and the binary is not a
  closure member, rather than starting a sandbox whose services never
  come up. It is never located by scanning `/nix/store`.
- **Every port a service binds must be granted.** That includes ports
  you did not choose: process-compose binds its own API on TCP 8080 by
  default and treats failure as fatal, which killed it — and the
  services it had already started — before this used a unix socket.

## Why the daemon example is commented out

The manifest carries a commented Postgres example because it illustrates
a rejection devcroft performs at resolution:

```toml
# db.command = "pg_ctl start -D ./pgdata"
# db.is-daemon = true
# db.shutdown.command = "pg_ctl stop -D ./pgdata"
```

`is-daemon = true` without `shutdown.command` is refused outright. Such a
service cannot be stopped: its launcher exits by design, so killing that
process at teardown reaps nothing and the database survives `down`.
Better to fail while resolving than to discover it when a sandbox is torn
down and a database is still running.

## Running it

```sh
devcroft up                                  # starts the sandbox and its services
devcroft exec -- curl -s localhost:8710      # the service is already up
devcroft down                                # services stop with the sandbox
```

`.devcroft/` holds the generated config, process-compose's log, and its
API socket. It is devcroft's artifact directory, regenerated at each
`up` — worth adding to `.gitignore` in a real project.
