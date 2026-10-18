# devenv-cursor-sample

Cursor agent running **inside** a devcroft sandbox, from the project's closure
(`pkgs.cursor-cli`, pinned by `devenv.lock`) rather than from your machine. It is **unfree** in nixpkgs, so `devenv.yaml` sets `allowUnfree: true`.

The same shape as [devenv-claude-sample](../devenv-claude-sample/), whose
README explains what every agent here relies on (a PID namespace of its own
for `/proc`, a shell by name, the CA bundle). The matrix of all eleven agents
is [docs/agents.md](../../docs/agents.md).

## Run it

```sh
devcroft up
devcroft exec devenvcursor -- cursor-agent --version
```

With a key, which reaches the sandbox from your shell and is never written to
a file here:

```sh
CURSOR_API_KEY=... devcroft up --recreate --yes
devcroft exec devenvcursor -- cursor-agent -p "say ok"
```

`devenv` must be on your `PATH` for `up`.

## What it reaches

`network.default = "deny"`, with `api2.cursor.sh` allowed. Measured 2026-10-18 with
a dummy key: the request reached the API and was refused there for the key.
Refused by the proxy along the way, without stopping it: nothing.

## Specific to Cursor agent

- It prints `Cannot open directory /etc/ssl/certs to load OpenSSL certificates` and works anyway: it reads the bundle devcroft points `SSL_CERT_FILE` at.

## Not covered

A dummy key only, so no real conversation or file edit was run, and no login
flow was tried.
