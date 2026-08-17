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
still carries the old workaround in its comments for the same reason —
worth reading as the before to this sample's after.

## What does *not* work yet, and how you can tell

devcroft reads the declarations above — `command`, `vars`, `is-daemon`,
and `shutdown.command` are all parsed at `up`, host-side — but nothing
starts them. Confirmed on this sample rather than assumed:

```
$ devcroft up && devcroft exec -- python3 -c "…connect(('127.0.0.1', 8710))…"
  service not running: ConnectionRefusedError
```

flox itself can still start it, supervised by flox rather than by
devcroft:

```
$ flox activate --start-services -- sh -c 'flox services status'
NAME    STATUS     PID
api     Running    205630
```

That is the whole remaining gap: the declarations are shared, the
supervision is not. `add-flox-services` covers closing it — the keeper
generating a process-compose config it owns and running it as a
supervised session, so `down` reaps services deterministically instead of
leaving them to flox's own lifecycle.

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
devcroft up
devcroft exec -- python3 -m http.server 8710 --bind 127.0.0.1   # binds
devcroft exec -- curl -s localhost:8710                          # from another shell
devcroft down
```

Note the server has to be started by hand for now — which is exactly the
gap described above.
