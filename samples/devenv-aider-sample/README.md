# devenv-aider-sample

Aider running **inside** a devcroft sandbox, from the project's closure
(`pkgs.aider-chat`, pinned by `devenv.lock`) rather than from your machine.

The same shape as [devenv-claude-sample](../devenv-claude-sample/), whose
README explains what every agent here relies on (a PID namespace of its own
for `/proc`, a shell by name, the CA bundle). The matrix of all eleven agents
is [docs/agents.md](../../docs/agents.md).

## Run it

```sh
devcroft up
devcroft exec devenvaider -- aider --version
```

With a key, which reaches the sandbox from your shell and is never written to
a file here:

```sh
OPENAI_API_KEY=... devcroft up --recreate --yes
devcroft exec devenvaider -- aider --message "say ok" --model gpt-4o --no-git
```

`devenv` must be on your `PATH` for `up`.

## What it reaches

`network.default = "deny"`, with `api.openai.com` allowed. Measured 2026-10-18 with
a dummy key: the request reached the API and was refused there for the key.
Refused by the proxy along the way, without stopping it: `raw.githubusercontent.com`.

## Specific to Aider

- One provider, OpenAI (`--model gpt-4o`), the one measured. For an Anthropic model, forward `ANTHROPIC_API_KEY` and allow `api.anthropic.com` instead.
- `AIDER_CHECK_UPDATE=false` in `[env.vars]`: its update check goes to PyPI, which is not allowed.
- Aider wants a git repository and offers to create one. The sandbox cannot see this repository's `.git` (it is above the project root), so run it in a project of its own, or pass `--no-git` as the example does.
- Aider writes `.aider.chat.history.md` and `.aider.input.history` into the project root, which is writable; `.gitignore` covers them.
- `raw.githubusercontent.com` (litellm's model price list) is refused and aider falls back to its bundled copy.

## Not covered

A dummy key only, so no real conversation or file edit was run, and no login
flow was tried.
