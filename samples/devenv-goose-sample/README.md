# devenv-goose-sample

Goose running **inside** a devcroft sandbox, from the project's closure
(`pkgs.goose-cli`, pinned by `devenv.lock`) rather than from your machine.

The same shape as [devenv-claude-sample](../devenv-claude-sample/), whose
README explains what every agent here relies on (a PID namespace of its own
for `/proc`, a shell by name, the CA bundle). The matrix of all eleven agents
is [docs/agents.md](../../docs/agents.md).

## Run it

```sh
devcroft up
devcroft exec devenvgoose -- goose --version
```

With a key, which reaches the sandbox from your shell and is never written to
a file here:

```sh
OPENAI_API_KEY=... devcroft up --recreate --yes
devcroft exec devenvgoose -- goose run --no-session -t "say ok"
```

`devenv` must be on your `PATH` for `up`.

## What it reaches

`network.default = "deny"`, with `api.openai.com` allowed. Measured 2026-10-18 with
a dummy key: the request reached the API and was refused there for the key.
Refused by the proxy along the way, without stopping it: nothing.

## Specific to Goose

- `GOOSE_DISABLE_KEYRING` is set: Goose keeps secrets in the system keyring by default, and a sandbox has none, so it reads the key from the environment instead. `GOOSE_PROVIDER` and `GOOSE_MODEL` replace `goose configure`.
- Before devcroft granted a CA bundle, Goose failed with `Error builder error` and a keyring message: its HTTP client could not be built without system roots. The keyring text was a red herring.

## Not covered

A dummy key only, so no real conversation or file edit was run, and no login
flow was tried.
