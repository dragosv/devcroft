# Coding agents inside a sandbox

Eleven mainstream coding-agent CLIs, each run inside one devcroft sandbox and
measured on 2026-10-18 (Linux, devenv provider, the agents from nixpkgs via
`devenv.lock`, `network.default = "deny"`). Every one of them starts, reaches
its own vendor's API through the egress proxy, and is refused there for the
dummy key it was given. That last step is the test: a refusal from the real
API means the runtime, the network path and TLS all worked from inside.

Each agent has a sample of its own, linked in the table: the agent in
`devenv.nix`, its key in `[env] forward`, its API host alone in
`network.allow`, and its own configuration in `[env.vars]`. Each was brought
up as committed and refused by its vendor's API for a dummy key. The
multi-provider agents (OpenCode, Aider, Crush) are set up for the one provider
they were measured with; their READMEs say what to swap for another.

## The matrix

| Agent | Sample | nixpkgs | Key | Allow | Refused, harmlessly | Needs |
|---|---|---|---|---|---|---|
| Claude Code 2.1.293 | [devenv-claude-sample](../samples/devenv-claude-sample/) | `claude-code` (unfree) | `ANTHROPIC_API_KEY` | `api.anthropic.com` | `http-intake.logs.us5.datadoghq.com` | — |
| Codex 0.161.0 | [devenv-codex-sample](../samples/devenv-codex-sample/) | `codex` | `CODEX_API_KEY` | `api.openai.com` | `chatgpt.com`, `api.github.com` | `exec --skip-git-repo-check` outside a repo |
| Antigravity CLI (`agy`) 1.4.1 | [devenv-antigravity-sample](../samples/devenv-antigravity-sample/) | `antigravity-cli` (unfree) | `GEMINI_API_KEY` | `generativelanguage.googleapis.com` | `antigravity-unleash.goog`, `play.googleapis.com` | `modelProvider = "gemini"` in its settings; `network.loopback = "any"` (Linux only) |
| OpenCode 1.18.35 | [devenv-opencode-sample](../samples/devenv-opencode-sample/) | `opencode` | the provider's own (here `OPENAI_API_KEY`) | that provider's host | `models.opencode.ai`, `registry.npmjs.org` | — |
| GitHub Copilot CLI | [devenv-copilot-sample](../samples/devenv-copilot-sample/) | `github-copilot-cli` (unfree) | `COPILOT_GITHUB_TOKEN` | `api.github.com`, `api.githubcopilot.com` | — | a fine-grained token (`github_pat_…`) |
| Aider 0.86.1 | [devenv-aider-sample](../samples/devenv-aider-sample/) | `aider-chat` | `OPENAI_API_KEY` | `api.openai.com` | `raw.githubusercontent.com` | `--no-check-update`; a git repo or `--no-git` |
| Goose 1.49.0 | [devenv-goose-sample](../samples/devenv-goose-sample/) | `goose-cli` | `OPENAI_API_KEY` | `api.openai.com` | — | `GOOSE_DISABLE_KEYRING=true`, `GOOSE_PROVIDER`, `GOOSE_MODEL` |
| Amp | [devenv-amp-sample](../samples/devenv-amp-sample/) | `amp-cli` (unfree) | `AMP_API_KEY` | `ampcode.com` | — | — |
| Cursor agent 2026.10.01 | [devenv-cursor-sample](../samples/devenv-cursor-sample/) | `cursor-cli` (unfree) | `CURSOR_API_KEY` | `api2.cursor.sh` | — | — |
| Qwen Code 0.23.4 | [devenv-qwen-sample](../samples/devenv-qwen-sample/) | `qwen-code` | `OPENAI_API_KEY` | `dashscope.aliyuncs.com` | `api.openai.com`, `…rum.aliyuncs.com` | `--auth-type openai` |
| Crush 0.96.1 | [devenv-crush-sample](../samples/devenv-crush-sample/) | `crush` (unfree: FSL) | `ANTHROPIC_API_KEY` | `api.anthropic.com` | `api.github.com`, `catwalk.charm.land`, `data.charm.land`, `hyper.charm.land` | — |

"Refused, harmlessly" is what the proxy turned away while the agent still
reached its API: telemetry, update checks, model catalogues. Allow any of
them deliberately if you want what they do; none was needed for a request.

One of those refusals prints a misleading error, worth knowing before reading
it as a sandbox failure:

- **Qwen Code** reports `Value of "this" must be of DOMException` when its
  API host is refused. `--auth-type openai` defaults to Alibaba's DashScope,
  not OpenAI, which is the host to allow.

The "Needs" column is the agent's own configuration, the same on the host:
Goose wants a system keyring that a sandbox does not have, Antigravity uses
its key only once a setting asks for API-key mode, and Copilot rejects a
classic `ghp_` token locally.

**Antigravity is the one agent that needs a devcroft key beyond the usual.**
It starts an embedded language server on ports the kernel picks and connects
to them, so it needs `network.loopback = "any"`: any port on the sandbox's
*own* loopback, inside its route-less namespace (`allow-loopback-ephemeral-ports`).
macOS refuses that key, since its sandboxes share the host's loopback, so
`agy` does not run in a devcroft sandbox there. A flag for the language
server's ports would make the key unnecessary; none exists in agy 1.4.1 or
1.5.6. It replaced Gemini CLI in this matrix on 2026-11-14.

## Whose endpoint can be moved

This matters for brokering a credential at the proxy (`add-agent-workload`):
the sandbox would hold a placeholder, and the proxy would put in the real
key. That needs the agent to send its API traffic somewhere devcroft
chooses, in plain HTTP, so the proxy can see the header it must rewrite.

**All eleven can be moved.** Measured 2026-11-15 (Antigravity 2026-11-14),
same versions as the matrix, each agent outside any sandbox with a dummy key.
Its proxy variables pointed at a closed port, so a request the setting did
not move failed instead of leaving. The endpoint was a local listener that
logged each request and answered `401`.

| Agent | Moved by | First request it sent | Key travels in |
|---|---|---|---|
| Claude Code | `ANTHROPIC_BASE_URL` | `POST /v1/messages?beta=true` | `x-api-key` |
| Codex | config `openai_base_url` (`-c openai_base_url=…`); **not** `OPENAI_BASE_URL` | `GET /v1/responses` (WebSocket upgrade), then `POST /v1/responses` | `Authorization: Bearer` |
| Antigravity | `GOOGLE_GEMINI_BASE_URL` | `POST /v1beta/models/…:streamGenerateContent` | `X-Goog-Api-Key` |
| OpenCode | `OPENAI_BASE_URL` (for its OpenAI provider) | `POST /v1/responses` | `Authorization: Bearer` |
| Copilot CLI | `COPILOT_API_URL` | `POST /mcp/readonly`, `GET /models` | `Authorization: Bearer` |
| Aider | `OPENAI_API_BASE` | `POST /v1/chat/completions` | `Authorization: Bearer` |
| Goose | `OPENAI_HOST` | `GET /v1/models`, `POST /v1/chat/completions` | `Authorization: Bearer` |
| Amp | `AMP_URL` | `POST /api/internal?getUserInfo` | `Authorization: Bearer` |
| Cursor | `CURSOR_API_ENDPOINT` or `--endpoint` | `POST /auth/exchange_user_api_key` | `Authorization: Bearer` |
| Qwen Code | `OPENAI_BASE_URL` | `POST /v1/chat/completions` | `Authorization: Bearer` |
| Crush | `ANTHROPIC_API_ENDPOINT`; **not** `ANTHROPIC_BASE_URL` | `POST /v1/messages` | `X-Api-Key` |

What the table does not show, and a broker has to handle:

- **Cursor exchanges its key.** The first call trades the key for a session
  token, and later calls carry that token. A broker that swaps the key only
  at the exchange leaves a working credential inside the sandbox, so for
  Cursor brokering protects the long-lived key, not access itself. Cursor
  also ignores `NO_PROXY`. With a proxy set, it sent even a loopback
  endpoint as a `CONNECT` tunnel, so the proxy sees a tunnel and not the
  header.
- **Codex tries a WebSocket first** and falls back to `POST` when that
  fails. A broker either handles the upgrade or relies on the fallback,
  which costs a few retries per session.
- **Amp's endpoint is its own service**, not a model API. Moving it works,
  but the upstream a broker forwards to is `ampcode.com`, and the key is an
  Amp account key.
- **Copilot sends its MCP calls to the same host**, with the same token. A
  broker for its model traffic covers those too.
- Two agents read a variable other than the one their SDK's convention
  suggests (Codex, Crush). The obvious variable was silently ignored and the
  request went to the vendor. Anyone configuring a broker should check the
  listener, not the agent's exit status.

**And every one will speak plain HTTP to the proxy**, which is what a broker
needs to read the header. With the endpoint set to an `http://` name and its
proxy variables pointing at a logging listener, nine of the agents sent an
absolute-URI request (`POST http://<host>/v1/…`, key header in clear). Qwen
Code and Cursor opened `CONNECT <host>:80` instead, and inside the tunnel
sent plain HTTP/1.1, key header included. Codex sends its WebSocket upgrade
the same way. The design that follows from this is in `add-agent-workload`,
design.md Decision 6.

Not measured: whether each agent sends *all* its traffic to the moved
endpoint. The proxy refused some other hosts along the way, such as
telemetry and update checks, as in the matrix above. Those still go out
through the allowlist, and none carries the model key.

## Brokering the key

A brokered key never enters the sandbox. You declare it in your own
configuration (`~/.config/devcroft/config.toml`), not the project's. At `up`
devcroft reads the real value from your shell and hands it to the egress
proxy alone. The sandbox gets two variables: a placeholder under the key's own
name, and the proxy's endpoint for that host under the agent's endpoint
variable. The proxy swaps the placeholder for the real value in the declared
header and sends the request on to `https://<host>`.

```toml
[[credentials.broker]]
host = "api.anthropic.com"        # must also be in the project's network.allow
header = "x-api-key"
env = "ANTHROPIC_API_KEY"         # real value read here at up; placeholder inside
endpoint_env = "ANTHROPIC_BASE_URL"
placeholder = "sk-ant-api03-{}"   # optional: a shape for clients that check one
```

The endpoint is `http://127.0.0.1:<port>/broker<n>`, a path on the proxy's
own port. That has a consequence worth knowing before it surprises you:
**the host is then reachable only through that endpoint.** The proxy refuses
a direct `CONNECT` to it, so anything that ignores the endpoint variable is
refused rather than sent the key. `up` says so on every run, and
`policy --render` lists each brokered host with origin `credential:broker`.

`up` refuses, at layer `config`, rather than fall back to handing over the
value. It refuses a host missing from `network.allow`, a value missing from
your shell, `network.default = "allow"` (where traffic need not pass the
proxy), and a variable that the manifest or `[credentials] env` also sets.

| Agent | `host` | `header` | `env` | `endpoint_env` | Through a route |
|---|---|---|---|---|---|
| Claude Code | `api.anthropic.com` | `x-api-key` | `ANTHROPIC_API_KEY` | `ANTHROPIC_BASE_URL` | **in devcroft, end to end**, isolated and not |
| Codex | `api.openai.com` | `Authorization` | `CODEX_API_KEY` | any name, read by a flag (below) | `nono-proxy` alone |
| Antigravity | `generativelanguage.googleapis.com` | `x-goog-api-key` | `GEMINI_API_KEY` | `GOOGLE_GEMINI_BASE_URL` | `nono-proxy` alone |
| OpenCode | the provider's host | `Authorization` | `OPENAI_API_KEY` | `OPENAI_BASE_URL` | `nono-proxy` alone |
| Qwen Code | `dashscope.aliyuncs.com` | `Authorization` | `OPENAI_API_KEY` | `OPENAI_BASE_URL` | `nono-proxy` alone |
| Copilot CLI | not settled (below) | `Authorization` | `COPILOT_GITHUB_TOKEN` | `COPILOT_API_URL` | no |
| Aider | `api.openai.com` | `Authorization` | `OPENAI_API_KEY` | `OPENAI_API_BASE` | no |
| Goose | `api.openai.com` | `Authorization` | `OPENAI_API_KEY` | `OPENAI_HOST` | no |
| Amp | `ampcode.com` | `Authorization` | `AMP_API_KEY` | `AMP_URL` | no |
| Crush | `api.anthropic.com` | `x-api-key` | `ANTHROPIC_API_KEY` | `ANTHROPIC_API_ENDPOINT` | no |
| Cursor | **not brokerable** | | | | refused (below) |

"Through a route" says how far each row has been tested:

- **In devcroft, end to end**: Claude Code ran inside a real sandbox with a
  dummy key, against a local TLS server standing in for the API
  (`tests/broker_claude.rs`). The server got the real key, and the
  sandbox's environment and the proxy log never held it.
- **`nono-proxy` alone**: the same proxy library, in a harness outside
  devcroft (2026-11-15). The real value reached the upstream.
- **No**: the agent was measured sending the same request form to a plain
  listener ([above](#whose-endpoint-can-be-moved)), never through a route.

For `Authorization`, the proxy injects `Bearer <value>`, and every other
header gets the bare value.

Per agent:

- **Codex** takes its endpoint from config, not a variable, so pick any free
  name for `endpoint_env` and pass it as a flag:
  `codex -c openai_base_url="$CODEX_BASE_URL" exec …`. Its WebSocket attempt
  is refused and it falls back to `POST`, which a route serves.
- **Copilot** rejects a classic `ghp_` token locally, so give it
  `placeholder = "github_pat_{}"`. Which upstream `COPILOT_API_URL` replaces
  is not settled. Copilot reaches both `api.github.com` and
  `api.githubcopilot.com`, and a route forwards to one host.
- **Cursor cannot be brokered.** It ignores `NO_PROXY`, so it sends even a
  loopback endpoint as `CONNECT 127.0.0.1:<port>`, and the proxy refuses a
  tunnel to itself (measured). It would be weak even if it worked: Cursor
  trades the key for a session token on its first call, and that token
  comes back into the sandbox. Use `[credentials] env` for it, and read the
  threat model's "custody" section first.
- **The endpoint is a root.** Whatever path the agent appends is sent to
  `https://<host>` unchanged. Some OpenAI-convention clients expect their base
  URL to end in `/v1`. Whether each of those adds it itself is measured only
  where the table says so.

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
  not: Gemini CLI (since replaced by Antigravity), Qwen and Cursor sent no credentials (407), and OpenCode and
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

## When the policy refuses

A refused tool call reaches the agent as the kernel's bare error, and a path
the sandbox hides reads as plain "no such file or directory". The likely
inference is wrong: the file does not exist, or the task needs `sudo`.

For Claude Code, `devcroft init --agent claude-code` writes a hook, to be
committed with the project, for its `PostToolUseFailure` event. When a failed call names a path or
host that the sandbox's policy refuses, it tells the agent so, names
what the sandbox grants, and says that the user changes the policy with
`up --recreate`. The policy decides, not the message: a missing file inside
the grants, or any other failure, gets nothing added. Outside a sandbox the
script does nothing.

The other ten agents have no hook yet. Each has its own hook system, so this
is wiring per agent, not one mechanism. `devcroft why` works for every agent,
run inside as `$HOME/../devcroft why --path <p> --op read`.

Not measured: Claude Code itself running the hook, which needs a login. The
test feeds the hook the event's documented input, built from errors that the
sandbox itself produced.

## Recording what an agent runs

`add-own-userland` admits `grep`, `sed` or `find` to the base runtime only
against a corpus of what agents actually run, so the corpus is collected, not
recalled. On macOS, bring the sample up with `DEVCROFT_LOG_EXECS` set, use the
agent with a real key on a typical task, then turn the log into a table:

```sh
cd samples/devenv-claude-sample
DEVCROFT_LOG_EXECS=1 devcroft up --recreate --yes
devcroft shell            # run the agent, give it a task, let it use its shell tool
devcroft down
python3 ../../scripts/exec-corpus.py .devcroft/*/run/exec.log
```

The log is `.devcroft/<name>/run/exec.log`. It holds one line per exec any
process with the redirect library makes, logged before the exec happens.
Each line is the path as the caller named it, then the arguments; each command
is logged once, even when its caller searches `PATH` itself. The script prints
each command name with its option tokens (`-E`, `-name`), never the operands,
which are the agent's own file names and patterns. Before you share a log,
read it: arguments are recorded in full, and a token passed as an argument
would be too.

The log records what ran, not what was wanted. Bash searches `PATH` itself,
so a command it cannot find never reaches an exec. A program that drops the
library (an Apple-signed or hardened binary) logs nothing for what it starts.
Linux has no library, so `up` warns there and logs nothing. A command line
longer than 16 KiB is cut, and the line ends in `[truncated]`.

## Not measured

- **A real key.** Every run used a dummy one, so a full conversation, tool
  use and file edits inside the sandbox were not exercised.
- **Logins.** Subscription and OAuth flows (`claude /login`, `codex login`,
  `amp login`, Copilot's device flow) were not tried.
- **macOS.** Not run. Its boundary is Seatbelt with no view, so none of the
  three fixes above is what decides there, and nothing here predicts it.
