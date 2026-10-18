# devenv-opencode-sample

OpenCode running **inside** a devcroft sandbox, from the project's closure
(`pkgs.opencode`, pinned by `devenv.lock`) rather than from your machine.

The same shape as [devenv-claude-sample](../devenv-claude-sample/), whose
README explains what every agent here relies on (a PID namespace of its own
for `/proc`, a shell by name, the CA bundle). The matrix of all eleven agents
is [docs/agents.md](../../docs/agents.md).

## Run it

```sh
devcroft up
devcroft exec devenvopencode -- opencode --version
```

With a key, which reaches the sandbox from your shell and is never written to
a file here:

```sh
OPENAI_API_KEY=... devcroft up --recreate --yes
devcroft exec devenvopencode -- opencode run "say ok"
```

`devenv` must be on your `PATH` for `up`.

## What it reaches

`network.default = "deny"`, with `api.openai.com` allowed. Measured 2026-10-18 with
a dummy key: the request reached the API and was refused there for the key.
Refused by the proxy along the way, without stopping it: `models.opencode.ai`, `registry.npmjs.org`.

## Specific to OpenCode

- One provider, OpenAI, the one measured. OpenCode uses whichever it has a key for: to use Anthropic instead, forward `ANTHROPIC_API_KEY` and allow `api.anthropic.com` in place of the OpenAI pair. Granting both is a host and a key the sandbox does not need.
- It is built on Bun, which read the proxy's old `http://<token>@127.0.0.1` URL as a host name; an isolated sandbox now gets a plain proxy URL (`docs/agents.md`).
- `models.opencode.ai` (its model catalogue) and `registry.npmjs.org` (plugins) are refused, and it still answered; allow them if you want what they do.

## Not covered

A dummy key only, so no real conversation or file edit was run, and no login
flow was tried.
