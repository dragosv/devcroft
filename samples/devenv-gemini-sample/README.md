# devenv-gemini-sample

Gemini CLI running **inside** a devcroft sandbox, from the project's closure
(`pkgs.gemini-cli`, pinned by `devenv.lock`) rather than from your machine.

The same shape as [devenv-claude-sample](../devenv-claude-sample/), whose
README explains what every agent here relies on (a PID namespace of its own
for `/proc`, a shell by name, the CA bundle). The matrix of all eleven agents
is [docs/agents.md](../../docs/agents.md).

## Run it

```sh
devcroft up
devcroft exec devenvgemini -- gemini --version
```

With a key, which reaches the sandbox from your shell and is never written to
a file here:

```sh
GEMINI_API_KEY=... devcroft up --recreate --yes
devcroft exec devenvgemini -- gemini -p "say ok"
```

`devenv` must be on your `PATH` for `up`.

## What it reaches

`network.default = "deny"`, with `generativelanguage.googleapis.com` allowed. Measured 2026-10-18 with
a dummy key: the request reached the API and was refused there for the key.
Refused by the proxy along the way, without stopping it: `play.googleapis.com`.

## Specific to Gemini CLI

- `GEMINI_CLI_TRUST_WORKSPACE` is set in `[env.vars]`: without it Gemini CLI refuses to run headless in a directory it has not been told to trust, on the host as much as here.
- Its telemetry host is refused, and when it is Gemini CLI prints `An unexpected critical error occurred:[object Object]` after the API's real answer. `gemini --debug` shows that answer (`API key not valid` for a bad key).

## Not covered

A dummy key only, so no real conversation or file edit was run, and no login
flow was tried.
