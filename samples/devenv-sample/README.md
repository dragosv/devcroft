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

devenv has `processes`, backed by the same process-compose supervisor
devcroft already generates a config for — and they are **not** supported
here. Whether devenv's process declarations are readable as a contract or
only as generated output has not been measured, and the same question is
open for devbox. A manifest declaring `[services]` under
`provider = "devenv"` fails distinguishably rather than silently starting
nothing. `flox-services-sample` is the one to read for services that
work.
