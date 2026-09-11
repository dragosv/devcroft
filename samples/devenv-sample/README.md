# devenv-sample

The fourth closure-tier provider, and the only one whose **project hook
runs inside the sandbox because the provider itself separates it** — not
because devcroft worked around its absence.

There is no application code here, deliberately, the same choice
[samples/flox-services-sample](../flox-services-sample/) makes. The point
of this sample is a property of the provider, and a CLI would only be
scenery. If you want a devenv project that builds something, the three
`citytime` samples are that for flox, nix and devbox, and nothing about
them would differ here.

## What this demonstrates

`enterShell` is project code. Every provider devcroft supports has some
version of it, and the rule is the same for all four: **it never runs on
the host during provisioning.** What differs is how much work that costs.

- **flox** has no activation mode that suppresses `[hook].on-activate`,
  so devcroft materializes from a derived, hook-free copy of the
  environment it builds itself (`flox::derive_hook_free_env`).
- **nix** and **devbox** hand back the environment without running the
  project's script at all, so there is nothing to defer — their hooks
  simply never run.
- **devenv** is the case neither of those describes. It has *both*: a
  hook-free way to get the environment (`devenv build shell`) and a
  separate, addressable handle on the hook itself
  (`devenv eval enterShell`). devcroft reads the second as **data** at
  `up` and runs it **inside** the sandbox, after restriction.

So the hook in this sample's `devenv.nix` does something you can check:

```sh
devcroft up
ls .devcroft-sample/enter-shell-ran      # exists — the hook ran
```

It ran under this project's own compiled policy, with the network denied
by default and only the project root writable — not with your shell's
access. Nothing wrote that file during `up` itself; run `up` with the
directory removed and watch it stay missing until the sandbox starts.

**The consequence worth knowing before it surprises you:** an
`enterShell` that reaches for host tooling is *denied* here, where
`devenv shell` would have let it work. That is
`own-policy-baseline` behaving as designed — a hook's commands must come
from the closure — but it is a real behaviour difference, and the same
one flox hooks already have.

## What devcroft writes here

`devenv build shell` writes into `.devenv/`, devenv's own cache and
GC-root store. That is the only place capture touches in the project tree
— measured, and asserted by a test — and devcroft never removes it,
because it is devenv's, shared with your own `devenv` invocations.
`.gitignore` covers it.

`devenv.nix`, `devenv.yaml` and `devenv.lock` are byte-identical after
`up`, including an `up` that fails. All three are committed, and all
three are fingerprinted for staleness: `devenv.yaml` carries the inputs,
so it can change what resolves with `devenv.nix` untouched.

An unlocked project is refused up front with `devenv update`, rather than
being allowed to resolve during provisioning. That is not tidiness:
`devenv build shell` on a project with no lockfile *writes* one, so
without the refusal the first `up` would pick input revisions at `up`.

## Services

**devenv is the second provider whose services devcroft supervises**, and
the first added since flox. `processes.ticker` and `processes.follower`
above are read at `up` through `devenv eval processes` — normalized data,
zero project code executed — and then run *inside* the sandbox under this
project's own compiled policy, supervised by its keeper, reaped at
`devcroft down`.

```sh
devcroft up
devcroft ps          # both processes, with their state
devcroft down        # neither survives
```

That is the whole reason devenv qualified and **devbox did not**. Measured
against devbox 0.17.5: `devbox.json` has no service schema, plugin
services arrive as generated per-plugin files under a cache directory,
and — decisively — `devbox services ls` executes `shell.init_hook`. Merely
*enumerating* devbox's services runs project code on the host, on the one
path support would need. See `docs/decisions.md`.

### Ordering, and the spelling that looks right but is not

`follower` declares `after = [ "devenv:processes:ticker" ]`, not
`after = [ "ticker" ]`. Both are accepted by devenv's own evaluation, and
only the first does anything: `before`/`after` are edges in devenv's
**task** graph, whose nodes are things like `devenv:enterShell` and
`devenv:processes:<name>`. A bare name matches no node, so devenv creates
no ordering from it — measured, visible in `devenv tasks list`.

devcroft therefore **refuses** the bare form rather than treating it as a
dependency. Honouring it would mean devcroft ordering a service that
devenv does not, so the same project would behave differently under the
two tools, with devcroft the more featureful of the pair. The refusal
names the process, the field, and the spelling that works.

### What else is refused

`ready`, `watch`, `proxy`, `ports`, `listen`, `linux.capabilities`,
`start.enable = false`, and devenv's `process-compose` passthrough block
all fail `up` at layer `provider`, naming the process and the field —
rather than being dropped. A project that declares a readiness probe and
gets a service without one has been lied to: the service would report
healthy on a condition nobody checked. A refusal you can act on is better
than a silence you cannot see.

The `process-compose` block is the one that costs something real:
expressing a dependency through it is common in devenv projects, and it
would work, since devcroft's own supervisor *is* process-compose.
It is refused to keep the supervisor behind the seam
`decouple-service-supervisor` built — the fix is to restate the
dependency as `after = [ "devenv:processes:<name>" ]`, which this sample
does.

### Where a process you never wrote comes from

`devenv eval processes` returns every process devenv's evaluation
produces, not only the ones typed under `processes`. Enabling one of
devenv's own integrations — `services.redis.enable = true` — contributes
one too, with its command pointing into the Nix store. devcroft
supervises it like any other, because filtering would need devcroft to
distinguish handwritten from contributed (devenv does not mark them) and
would drop exactly the service a user enabling an integration wants
running.

So a `redis` in `devcroft ps` that you never wrote is expected, and this
paragraph is where that is written down.
