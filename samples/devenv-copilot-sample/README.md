# devenv-copilot-sample

GitHub Copilot CLI running **inside** a devcroft sandbox, from the project's closure
(`pkgs.github-copilot-cli`, pinned by `devenv.lock`) rather than from your machine. It is **unfree** in nixpkgs, so `devenv.yaml` sets `allowUnfree: true`.

The same shape as [devenv-claude-sample](../devenv-claude-sample/), whose
README explains what every agent here relies on (a PID namespace of its own
for `/proc`, a shell by name, the CA bundle). The matrix of all eleven agents
is [docs/agents.md](../../docs/agents.md).

## Run it

```sh
devcroft up
devcroft exec devenvcopilot -- copilot --version
```

With a key, which reaches the sandbox from your shell and is never written to
a file here:

```sh
COPILOT_GITHUB_TOKEN=... devcroft up --recreate --yes
devcroft exec devenvcopilot -- copilot -p "say ok"
```

`devenv` must be on your `PATH` for `up`.

## What it reaches

`network.default = "deny"`, with `api.github.com`, `api.githubcopilot.com` allowed. Measured 2026-10-18 with
a dummy key: the request reached the API and was refused there for the key.
Refused by the proxy along the way, without stopping it: nothing.

## Specific to GitHub Copilot CLI

- `COPILOT_GITHUB_TOKEN` is the one forwarded; the CLI also reads `GH_TOKEN`, which forwarding as well would only hand the sandbox a second credential.
- The token must be a fine-grained one (`github_pat_…`) with Copilot access. A classic `ghp_` token is rejected by the CLI itself before any request, on the host as here.
- `api.githubcopilot.com` is where completions go once authenticated; only the `api.github.com` half was reached with a dummy token.

## Not covered

A dummy key only, so no real conversation or file edit was run, and no login
flow was tried.
