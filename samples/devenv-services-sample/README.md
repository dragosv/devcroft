# devenv-services-sample

The devenv counterpart to
[samples/flox-services-sample](../flox-services-sample/), and deliberately
the same shape: one real service on one granted loopback port, so the two
providers can be compared field for field.

flox and devenv are the only two providers whose services devcroft
supervises. nix has no service concept; devbox's are refused for a
measured reason (`docs/decisions.md`). If you want the devenv provider
itself — the hook, the capture, the closure — read
[samples/devenv-sample](../devenv-sample/) first. This one is only about
services.

## What it declares

```nix
processes.api = {
  exec = "python3 -m http.server $API_PORT --bind 127.0.0.1";
  env.API_PORT = "8730";
};
```

The port comes through `env` rather than being hardcoded in the command,
which is the same shape flox's sample uses for `vars`. devcroft reads
`exec`, `env`, `cwd`, the restart policy, the shutdown signal and grace
period, and ordering.

`devcroft.toml` grants that one loopback port while keeping egress
denied:

```toml
[network]
default = "deny"
ports = [8730]
```

## What was measured, on this sample

```
$ devcroft up
devcroft: sandbox 'devenvsvc' is started.

$ devcroft status
service api: running pid=54678
service probe: running pid=54677

$ devcroft exec -- python3 -c "…urlopen('http://127.0.0.1:8730')…"
200

$ devcroft down
$ ps -eo args | grep -c "http.server 8730"
0
```

The service answers from inside the sandbox under
`network.default = "deny"`, because the port grant and egress are
independent axes. Teardown is verified by process absence, not by a stop
command's exit status — that distinction earned its place during
`add-flox-services`, where killing the supervisor alone left a child
holding the port.

`devcroft ps` lists services indented under their sandbox; `devcroft
status` reports them with pids. A service's own output goes to
`.devcroft/<sandbox>/services.log`, not to `devcroft logs`, which shows
the keeper's.

## Two things this sample had to work around, both worth knowing

**The sandbox name is short on purpose.** The supervisor's unix socket
lives at `<project>/.devcroft/<name>/services.sock`, and the OS caps a
unix socket path at 103 bytes. With `devenv-services-sample` as the name
this sample's own path came to **112**, and `up` refused, naming the
limit and the fix. So the name is `devenvsvc`. A sample that cannot come
up where it lives would not be much of a sample.

**On macOS the port grant is degraded, and devcroft says so.** Measured
here:

```
$ devcroft exec -- python3 -c "…bind 8730, then 8731…"
8730 denied: 48        # EADDRINUSE — the service itself holds it
8731 BOUND             # not granted, and bound anyway
```

`devcroft status` reports the reason without being asked:

> `network.ports` (per-port listen scoping) is degraded on this host:
> macOS Seatbelt cannot filter bind/listen by port, so granting one port
> grants all of them (fallback: the declared ports work, but a process in
> this sandbox can also listen on ports the manifest did not grant)

On Linux the grant is per-port and an ungranted one is denied. This is
the "degraded capabilities are surfaced, never silent" rule doing its
job, and it is why this README does not repeat `flox-services-sample`'s
"8711 denied correctly" — that was measured on Linux, and copying it here
would have been a claim about a platform this sample was not run on.

Note also that 8730 came back `EADDRINUSE` rather than granted: the
service was already listening on it. That failure is the proof the
service is up.

## The spelling that looks right

`probe` declares `after = [ "devenv:processes:api" ]`, not
`after = [ "api" ]`. Both are accepted by devenv's own evaluation, and
only the first does anything.

`before`/`after` are edges in devenv's **task** graph, whose nodes are
things like `devenv:enterShell`, `devenv:files` and
`devenv:processes:<name>`. A bare name matches no node, so devenv creates
no ordering from it at all — measured, and visible in `devenv tasks
list`, where the bare form leaves `probe` a leaf and the qualified form
draws the edge.

devcroft therefore **refuses** the bare form rather than treating it as a
dependency:

```
devenv process `probe` declares `after`, which devcroft cannot carry:
`api` is a bare name, which devenv itself creates no ordering from —
write `devenv:processes:api` if you meant the process by that name
```

Honouring it would mean devcroft ordering a service that devenv does not,
so the same project would behave differently under the two tools — with
devcroft the *more* featureful of the pair. For a tool whose job is to
run the project's own environment faithfully, that is the wrong direction
to be wrong in.

## What is refused, and why refusing beats dropping

`ready`, `watch`, `proxy`, `ports`, `listen`, `linux.capabilities`,
`start.enable = false`, and devenv's `process-compose` passthrough block
all fail `up` at layer `provider`, naming the process and the field.

A project that declares a readiness probe and gets a service without one
has been lied to: it would report healthy on a condition nobody checked.
A refusal you can act on beats a silence you cannot see.

The `process-compose` block is the one that costs something real.
Expressing a dependency through it is common in devenv projects, and it
would work — devcroft's own supervisor *is* process-compose. It is
refused to keep the supervisor behind the seam
`decouple-service-supervisor` built, and the fix is to restate the
dependency in devenv's own vocabulary, which this sample does.

## Where a service you never wrote comes from

`devenv eval processes` returns every process devenv's evaluation
produces, not only the ones typed under `processes`. Enabling one of
devenv's own integrations — `services.redis.enable = true` — contributes
one too, with its command pointing into the Nix store. devcroft
supervises it like any other: filtering would need devcroft to
distinguish handwritten from contributed, which devenv does not mark, and
would drop exactly the service a user enabling an integration wants
running.

So a `redis` in `devcroft status` that you never wrote is expected, and
this paragraph is where that is written down.

## process-compose is the project's dependency

`packages` includes `pkgs.process-compose`. devcroft never takes the
supervisor from the host's PATH, and fails at layer `provider` if
services are declared and the binary is not a closure member — rather
than starting a sandbox whose services never come up.

That is the project's dependency to declare because the project declared
the services, and it is devcroft's requirement leaking into the user's
manifest. `decouple-service-supervisor` is the change that exists to
remove it.
