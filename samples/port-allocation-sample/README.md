# port-allocation-sample

Two sandboxes of one project running the same service at the same time,
where they share the host's loopback. It is what `add-port-allocation`
exists for.

## Why they would collide

The flox manifest declares one service, reading its port from a variable
with a default of 8720:

```toml
[services]
web.command = "python3 -m http.server $WEB_PORT --bind 127.0.0.1"
web.vars.WEB_PORT = "8720"
```

`devcroft.toml` uses `network.default = "allow"`. On Linux, a `"deny"`
sandbox gets its own network namespace and its ports are private, so two of
them binding 8720 never meet. An `"allow"` sandbox shares the host's
loopback, as every sandbox on macOS does, and a second one binding 8720
would fail with `EADDRINUSE`.

## What devcroft does instead

```toml
[network.services.web]
var = "WEB_PORT"
```

For each sandbox, `up` chooses a free port for `WEB_PORT`, records it,
substitutes it for the provider's 8720 in that sandbox's generated service
config (this file is never touched), and sets it in sessions too:

```console
$ devcroft up --name pa-a
$ devcroft up --name pa-b
$ devcroft status pa-a | grep port
port WEB_PORT=13284 (service web)
$ devcroft status pa-b | grep port
port WEB_PORT=16290 (service web)
$ devcroft exec pa-a -- sh -c 'echo $WEB_PORT'
13284
```

`--name` is what gives two sandboxes of one project distinct identities;
`status`, `policy --render` and `exec` accept either name from this
directory.

- **Sticky.** `down` and `up` again reuse the same port. If something else
  took it meanwhile, `up` picks another and says so, naming both.
- **Stopped means recorded, not listening.** `status` on a stopped sandbox
  says the port is recorded and that nothing is listening on it.
- **Visible in the policy.** `devcroft policy --render pa-a` lists
  `web.WEB_PORT  13284 (allocated)`, with an origin distinct from a
  declared port.
- **Refused when it cannot work.** A service whose command hardcodes its
  port (`http.server 8720`) never reads the variable, so `up` refuses the
  request, naming the service, rather than granting a port nothing will
  listen on.

With `network.default = "deny"`, nothing is allocated and 8720 is used
unchanged in every sandbox, since each has its own namespace.

Clean up with `devcroft rm pa-a --yes` and `devcroft rm pa-b --yes`.
