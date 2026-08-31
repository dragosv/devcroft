# flox-services-sample

Demonstrates two things that are easy to conflate, both of which work
today: **granting a loopback port** and **devcroft supervising declared
services**. They are independent axes — a port grant is a policy rule, a
service is a process the keeper owns — and this sample exercises each
without the other doing the work.

It is also the regression case for a third thing, by accident of how it
was written: its flox manifest declares **no shell**, which is what every
real flox project looks like. See "The shell nobody declares" below.

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

`status` and `ps` report per-service state by querying that API over
its unix socket (`.devcroft/services.sock`) — which is why
process-compose is not started with `--no-server`, even though that would
also avoid the default TCP bind:

```
$ devcroft status
service api: running pid=57009
```

Immediately after `up` that line reads `not started` for about a second,
until process-compose has bound its socket. It is a race in the report,
not in the service. What `devcroft logs` shows is the *keeper's* log; a
service's own output goes to `.devcroft/<sandbox>/services.log`.

Two things you may hit:

- **process-compose must be in the environment.** devcroft fails at
  layer `provider` if services are declared and the binary is not a
  closure member, rather than starting a sandbox whose services never
  come up. It is never located by scanning `/nix/store`. This is the
  project's dependency to declare, because the project declared the
  services — unlike the shell below, which is devcroft's.
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

## The shell nobody declares

This sample's `[install]` section has `python3` and `process-compose` and
no shell, which is not an oversight — it is what a real flox manifest
looks like. Nothing about flox needs a shell declared, because under a
plain `flox activate` the host's `PATH` is still there and `sh` resolves
to `/usr/bin/sh`.

devcroft's policy denies host binaries (`own-policy-baseline`), and
devcroft needs a shell for three things the project never asked for: SSH
login sessions, `devcroft shell`, and the command process-compose runs
each service through. For a while all three resolved a bare `sh` through
`PATH` *inside* the sandbox, which reached the host's copy and was
refused — so on this sample every service died at launch with
`fork/exec /usr/bin/sh: permission denied`, buried in the supervisor's
own log, while `up` reported a healthy sandbox.

devcroft now resolves an absolute shell out of the closure itself
(`src/shell.rs`): the environment's own `PATH` if it supplies one *in the
store*, otherwise a `bin/sh` from the closure's requisites — present here
even though nothing declares it, because bash is already a transitive
dependency. The chosen path is granted explicitly, so it survives
`add-mount-isolation` tightening the blanket `/nix/store` grant, and it
shows up like any other rule:

```
$ devcroft policy --render | grep bash
  /nix/store/f6ls...-bash-interactive-5.3p9   provider:flox

$ devcroft why --path /nix/store/f6ls...-bash-interactive-5.3p9/bin/sh --op read
ALLOWED
allowed by rule provider:flox
```

Requiring the project to declare `bash` was the other option, and was
rejected: it would fail the first `up` of every existing flox project for
a dependency devcroft introduced.
