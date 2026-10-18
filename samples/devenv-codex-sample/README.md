# devenv-codex-sample

Codex running **inside** a devcroft sandbox, from the project's closure
(`pkgs.codex`, pinned by `devenv.lock`) rather than from your machine.

The same shape as [devenv-claude-sample](../devenv-claude-sample/), whose
README explains what every agent here relies on (a PID namespace of its own
for `/proc`, a shell by name, the CA bundle). The matrix of all eleven agents
is [docs/agents.md](../../docs/agents.md).

## Run it

```sh
devcroft up
devcroft exec devenvcodex -- codex --version
```

With a key, which reaches the sandbox from your shell and is never written to
a file here:

```sh
CODEX_API_KEY=... devcroft up --recreate --yes
devcroft exec devenvcodex -- codex exec --skip-git-repo-check "say ok"
```

`devenv` must be on your `PATH` for `up`.

## What it reaches

`network.default = "deny"`, with `api.openai.com` allowed. Measured 2026-10-18 with
a dummy key: the request reached the API and was refused there for the key.
Refused by the proxy along the way, without stopping it: `chatgpt.com`, `api.github.com`.

## Specific to Codex

- `codex exec` authenticates with `CODEX_API_KEY`. Measured: with `OPENAI_API_KEY` alone it sent no key at all (`401 Missing bearer or basic authentication`), so `CODEX_API_KEY` is the one forwarded.
- `--skip-git-repo-check` is needed only outside a repository; the sandbox does not see this repository's `.git`, which is above the project root.

## Not covered

A dummy key only, so no real conversation or file edit was run, and no login
flow was tried.
