# devbox-services-sample

The devbox counterpart to
[samples/flox-services-sample](../flox-services-sample/) and
[samples/devenv-services-sample](../devenv-services-sample/): one real
service on one granted loopback port, under a deny-all egress policy.

It differs from both on the axis that matters most. **Nothing in this
project declares a service.** `devbox.json` lists two packages and
nothing else, and devcroft still supervises a postgres.

## Where the service comes from

```json
{
  "packages": ["postgresql@latest", "process-compose@latest"]
}
```

`devbox.json` has no service schema. The `postgresql` service is the
**package's devbox plugin**, which `devbox install` writes into a
generated file under the project's cache directory:

```yaml
# .devbox/virtenv/postgresql/process-compose.yaml
processes:
  postgresql:
    command: "sh -c 'exec postgres -k \"$PGHOST\" -p \"${PGPORT:-5432}\"'"
    is_daemon: false
    shutdown:
      command: "pg_ctl stop -m fast"
    availability:
      restart: "always"
    readiness_probe:
      exec:
        command: "pg_isready -p \"${PGPORT:-5432}\""
```

That file is the plugin author's, not yours, and it is the only place the
declaration exists. So a service in `devcroft status` that you never
wrote is expected for devbox, and **this section is where that is written
down**. Each package that ships a plugin with a `process-compose.yaml`
contributes its services this way (`redis`, `nginx`, `mysql`, `valkey`,
`caddy` and others).

devcroft reads it with three rules worth knowing:

- **The file is read, never executed.** devcroft does not go through
  `devbox services ls`, which runs `shell.init_hook`. A full `up` runs
  the project's `init_hook` zero times host-side
  (`a_services_project_runs_its_init_hook_only_inside_the_sandbox`).
- **The packages `devbox.json` declares decide, not the directory.**
  `.devbox/virtenv/` is stale cache: removing `postgresql` from
  `devbox.json` leaves its plugin directory and this file behind. devcroft
  reads a plugin's file only while its package is declared, so removing
  the package removes the service.
- **A field devcroft cannot carry is refused by name, never dropped.**
  `up` fails at layer `provider`, naming the plugin, the process and the
  field.

Building devbox services against the objection that a plugin's definition
is not a project declaration was a deliberate call. `docs/decisions.md`
("Built, against the argument: devbox services") records both sides.

## What the project still has to supply

The plugin starts postgres but creates no database: `devbox info
postgresql` says to run `initdb`. So `devcroft.toml` does it once, inside
the sandbox:

```toml
[hooks]
post_create = '[ -f "$PGDATA/PG_VERSION" ] || initdb'
```

The guard tests `PG_VERSION`, not the directory, because `devbox install`
creates `$PGDATA` **empty**. A `[ -d "$PGDATA" ]` guard skips `initdb` on
the very first `up` and postgres never starts. If `initdb` fails, the next
`up` runs `post_create` again.

`process-compose` is in `packages` for the same reason as in the other two
services samples: devcroft never takes the supervisor from the host's
`PATH`, and fails at layer `provider` if services exist and it is not a
closure member.

## What was measured, on this sample (Linux)

```
$ devcroft up
devcroft: note: this sandbox has its own network namespace, so its declared
port(s) 5432 are reachable from inside it and through `devcroft ssh -L
<local>:127.0.0.1:<port> devboxsvc`, but not directly on the host's own loopback
devcroft: sandbox 'devboxsvc' is started.

$ devcroft status          # about 10 s later
service postgresql: running pid=64, ready

$ devcroft exec -- sh -c 'psql -h "$PGHOST" -d postgres -Atc "select 40 + 2"'
42

$ devcroft exec -- sh -c 'id -un; psql -h "$PGHOST" -d postgres -Atc "select current_user"'
vscode
vscode

$ devcroft down
$ pgrep -x postgres; pgrep -x process-compose
                           # nothing
```

`, ready` appears only once the plugin's `pg_isready` probe passes.
Before that, the line reads `, not ready`.

## What it took

A postgres was the hardest service any sample has run, because `initdb`
and postgres expect an ordinary Unix user on an ordinary system. In a
sandbox whose view holds only what is granted, four things were missing,
each measured failing before it was fixed:

- **Sessions were uid 0**, and `initdb` refuses to run as root. They now
  run as your own uid.
- **No `/etc/passwd`**, so `initdb` and `psql` could not look up the uid.
  process-compose had been logging `Failed get user` too.
- **No `/bin/sh`**, which `popen()` starts by absolute path. `initdb`
  runs `postgres -V` that way.
- **No `/etc/hosts`**, so postgres could not listen on `localhost`.

devcroft now writes the last three into the view itself, from its own
knowledge rather than the host's. The host's `passwd` would point tools at
your real home, and the host's `/bin/sh` is a host binary the policy keeps
out. `policy --render` shows each one with origin `identity`:

```
$ devcroft policy --render | grep identity
  /etc/passwd                              identity
  /etc/group                               identity
  /etc/hosts                               identity
  /bin/sh                                  identity
  /usr/bin/env                             identity
```

If the manifest grants one of these paths itself, the sandbox sees the
host's file instead.

## On macOS

The service does not come up there yet. postgres needs System V shared
memory (`shmget`), which Seatbelt refuses. Linux is unaffected, because
Landlock does not mediate SysV IPC. The upstream ask is
`docs/nono-sysv-ipc-issue.md`. devenv's postgres hits the same wall on
macOS, from the same cause.

## The sandbox name

`devboxsvc`, short for the same reason as `devenvsvc`: the supervisor's
socket is `<project>/.devcroft/<name>/services.sock`, and the OS caps a
unix socket path at 103 bytes.
