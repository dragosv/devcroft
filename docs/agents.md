# Coding agents inside a sandbox

Eleven mainstream coding-agent CLIs, each run inside one devcroft sandbox and
measured on 2026-10-18 (Linux, devenv provider, the agents from nixpkgs via
`devenv.lock`, `network.default = "deny"`). Every one of them starts, reaches
its own vendor's API through the egress proxy, and is refused there for the
dummy key it was given. That last step is the test: a refusal from the real
API means the runtime, the network path and TLS all worked from inside.

A working project to copy is
[samples/devenv-claude-sample](../samples/devenv-claude-sample/); every
agent below goes in the same place in `devenv.nix`, with its API host in
`network.allow` and its key in `[env] forward`.

## The matrix

| Agent | nixpkgs | Key | Allow | Refused, harmlessly | Needs |
|---|---|---|---|---|---|
| Claude Code 2.1.293 | `claude-code` (unfree) | `ANTHROPIC_API_KEY` | `api.anthropic.com` | `http-intake.logs.us5.datadoghq.com` | — |
| Codex 0.161.0 | `codex` | `CODEX_API_KEY` | `api.openai.com` | `chatgpt.com`, `codeload.github.com` | `exec --skip-git-repo-check` outside a repo |
| Gemini CLI 0.47.0 | `gemini-cli` | `GEMINI_API_KEY` | `generativelanguage.googleapis.com` | `play.googleapis.com` | `GEMINI_CLI_TRUST_WORKSPACE=true` |
| OpenCode 1.18.35 | `opencode` | the provider's own (here `OPENAI_API_KEY`) | that provider's host | `models.opencode.ai`, `registry.npmjs.org` | — |
| GitHub Copilot CLI | `github-copilot-cli` (unfree) | `COPILOT_GITHUB_TOKEN` or `GH_TOKEN` | `api.github.com`, `api.githubcopilot.com` | — | a fine-grained token (`github_pat_…`) |
| Aider 0.86.1 | `aider-chat` | `OPENAI_API_KEY` | `api.openai.com` | `raw.githubusercontent.com` | `--no-check-update`; a git repo or `--no-git` |
| Goose 1.49.0 | `goose-cli` | `OPENAI_API_KEY` | `api.openai.com` | — | `GOOSE_DISABLE_KEYRING=true`, `GOOSE_PROVIDER`, `GOOSE_MODEL` |
| Amp | `amp-cli` (unfree) | `AMP_API_KEY` | `ampcode.com` | — | — |
| Cursor agent 2026.10.01 | `cursor-cli` (unfree) | `CURSOR_API_KEY` | `api2.cursor.sh` | — | — |
| Qwen Code 0.23.4 | `qwen-code` | `OPENAI_API_KEY` | `dashscope.aliyuncs.com` | `…rum.aliyuncs.com` | `--auth-type openai` |
| Crush 0.96.1 | `crush` | `ANTHROPIC_API_KEY` | `api.anthropic.com` | `catwalk.charm.land`, `data.charm.land`, `hyper.charm.land` | — |

"Refused, harmlessly" is what the proxy turned away while the agent still
reached its API: telemetry, update checks, model catalogues. Allow any of
them deliberately if you want what they do; none was needed for a request.

Two of those refusals print misleading errors, worth knowing before reading
them as a sandbox failure:

- **Gemini CLI** reports `An unexpected critical error occurred:[object
  Object]` when its telemetry host is refused; the API's own answer is in
  `--debug` output.
- **Qwen Code** reports `Value of "this" must be of DOMException` when its
  API host is refused. `--auth-type openai` defaults to Alibaba's DashScope,
  not OpenAI, which is the host to allow.

The "Needs" column is the agent's own configuration, the same on the host:
Goose wants a system keyring that a sandbox does not have, Gemini wants a
trusted directory, Copilot rejects a classic `ghp_` token locally.

## What devcroft had to fix

The first run had two of eleven working. Three things in devcroft, not in the
agents, stopped the rest, and each is fixed:

- **No CA bundle.** devenv's environment names none, `/etc/ssl` is not in the
  view, and everything that reads the system store failed TLS: Go (Crush),
  rustls (Codex), reqwest (Goose, which could not build a client at all).
  Node, Bun and Python's certifi bring their own, which is why this hid.
  devcroft now grants the host's system bundle (`trust-store` in `policy
  --render`) and sets `SSL_CERT_FILE` when the environment names nothing.
- **The proxy's token in the URL.** The proxy URL carried a per-session token
  as userinfo, on the reasoning that every HTTP client sends it. Five did
  not: Gemini, Qwen and Cursor sent no credentials (407), and OpenCode and
  Amp, built on Bun, read `<token>@127.0.0.1` as a host name. An isolated
  sandbox now gets a plain `http://127.0.0.1:<port>`; its traffic reaches the
  proxy only through the unix socket, whose bridge presents the token, so the
  check is unchanged and the socket's 0700 directory is its boundary.
- **`/proc`.** Claude Code is a Bun binary that aborts in milliseconds without
  `/proc/self/maps` and friends. A sandbox with a PID namespace of its own is
  granted `/proc` (`pid-namespace`); see
  [known-gaps.md](known-gaps.md) for a host that cannot give one.

Earlier, from the same measurement: devcroft's resolved shell is now `SHELL`
and the last entry on `PATH`, since agents and npm look for `sh` by name.

## Not measured

- **A real key.** Every run used a dummy one, so a full conversation, tool
  use and file edits inside the sandbox were not exercised.
- **Logins.** Subscription and OAuth flows (`claude /login`, `codex login`,
  `amp login`, Copilot's device flow) were not tried.
- **macOS.** Not run. Its boundary is Seatbelt with no view, so none of the
  three fixes above is what decides there, and nothing here predicts it.
