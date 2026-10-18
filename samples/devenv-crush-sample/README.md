# devenv-crush-sample

Crush running **inside** a devcroft sandbox, from the project's closure
(`pkgs.crush`, pinned by `devenv.lock`) rather than from your machine. Its license (FSL) counts as **unfree** in nixpkgs, so `devenv.yaml` sets `allowUnfree: true`.

The same shape as [devenv-claude-sample](../devenv-claude-sample/), whose
README explains what every agent here relies on (a PID namespace of its own
for `/proc`, a shell by name, the CA bundle). The matrix of all eleven agents
is [docs/agents.md](../../docs/agents.md).

## Run it

```sh
devcroft up
devcroft exec devenvcrush -- crush --version
```

With a key, which reaches the sandbox from your shell and is never written to
a file here:

```sh
ANTHROPIC_API_KEY=... devcroft up --recreate --yes
devcroft exec devenvcrush -- crush run "say ok"
```

`devenv` must be on your `PATH` for `up`.

## What it reaches

`network.default = "deny"`, with `api.anthropic.com` allowed. Measured 2026-10-18 with
a dummy key: the request reached the API and was refused there for the key.
Refused by the proxy along the way, without stopping it: `api.github.com`, `catwalk.charm.land`, `data.charm.land`, `hyper.charm.land`.

## Specific to Crush

- One provider, Anthropic, the one measured. For OpenAI, forward `OPENAI_API_KEY` and allow `api.openai.com` instead.
- Written in Go, so it trusts only the system CA store. Before devcroft granted the host's bundle it failed with `x509: certificate signed by unknown authority`.
- The `charm.land` hosts (provider catalogue, telemetry) are refused, and it still answered.

## Not covered

A dummy key only, so no real conversation or file edit was run, and no login
flow was tried.
