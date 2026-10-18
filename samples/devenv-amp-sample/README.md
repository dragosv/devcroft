# devenv-amp-sample

Amp running **inside** a devcroft sandbox, from the project's closure
(`pkgs.amp-cli`, pinned by `devenv.lock`) rather than from your machine. It is **unfree** in nixpkgs, so `devenv.yaml` sets `allowUnfree: true`.

The same shape as [devenv-claude-sample](../devenv-claude-sample/), whose
README explains what every agent here relies on (a PID namespace of its own
for `/proc`, a shell by name, the CA bundle). The matrix of all eleven agents
is [docs/agents.md](../../docs/agents.md).

## Run it

```sh
devcroft up
devcroft exec devenvamp -- amp --version
```

With a key, which reaches the sandbox from your shell and is never written to
a file here:

```sh
AMP_API_KEY=... devcroft up --recreate --yes
devcroft exec devenvamp -- amp -x "say ok"
```

`devenv` must be on your `PATH` for `up`.

## What it reaches

`network.default = "deny"`, with `ampcode.com` allowed. Measured 2026-10-18 with
a dummy key: the request reached the API and was refused there for the key.
Refused by the proxy along the way, without stopping it: nothing.

## Specific to Amp

- Built on Bun: it failed with `Couldn't connect to the Amp server` until an isolated sandbox stopped receiving the proxy token in the URL (`docs/agents.md`).

## Not covered

A dummy key only, so no real conversation or file edit was run, and no login
flow was tried.
