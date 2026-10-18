# devenv-qwen-sample

Qwen Code running **inside** a devcroft sandbox, from the project's closure
(`pkgs.qwen-code`, pinned by `devenv.lock`) rather than from your machine.

The same shape as [devenv-claude-sample](../devenv-claude-sample/), whose
README explains what every agent here relies on (a PID namespace of its own
for `/proc`, a shell by name, the CA bundle). The matrix of all eleven agents
is [docs/agents.md](../../docs/agents.md).

## Run it

```sh
devcroft up
devcroft exec devenvqwen -- qwen --version
```

With a key, which reaches the sandbox from your shell and is never written to
a file here:

```sh
OPENAI_API_KEY=... devcroft up --recreate --yes
devcroft exec devenvqwen -- qwen -p "say ok" --auth-type openai
```

`devenv` must be on your `PATH` for `up`.

## What it reaches

`network.default = "deny"`, with `dashscope.aliyuncs.com` allowed. Measured 2026-10-18 with
a dummy key: the request reached the API and was refused there for the key.
Refused by the proxy along the way, without stopping it: `api.openai.com`, `gb4w8c3ygj-default-sea.rum.aliyuncs.com`.

## Specific to Qwen Code

- `--auth-type openai` means an OpenAI-compatible API, and its default endpoint is Alibaba's DashScope, not OpenAI: that is the host allowed. Point it elsewhere with `OPENAI_BASE_URL` and allow that host instead.
- When its API host is refused it prints `Value of "this" must be of DOMException`, not a network error. If you see that, check `network.allow`.

## Not covered

A dummy key only, so no real conversation or file edit was run, and no login
flow was tried.
