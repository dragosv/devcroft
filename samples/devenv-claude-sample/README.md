# devenv-claude-sample

A coding agent, Claude Code, running **inside** a devcroft sandbox, with the
agent itself coming from the project's closure rather than from your machine.

There is no application code here. The subject is the agent: what it takes
for one to run confined, and what it can and cannot reach once it does.

Ten other agents (Codex, Gemini CLI, OpenCode, Copilot, Aider, Goose, Amp,
Cursor, Qwen Code, Crush) were run the same way; the matrix of what each
needs is [docs/agents.md](../../docs/agents.md).

## Run it

```sh
devcroft up
devcroft exec devenvclaude -- claude --version     # 2.1.293 (Claude Code)
```

With an API key, which reaches the sandbox from your shell and is never
written to a file here:

```sh
ANTHROPIC_API_KEY=sk-ant-... devcroft up --recreate --yes
devcroft shell devenvclaude                        # then: claude
```

`devenv` must be on your `PATH` for `up`, as for every devenv sample.

## Where the agent comes from

`devenv.nix` lists `pkgs.claude-code`, so the agent is a closure-tier
dependency like any other: pinned by `devenv.lock`, the same on every
machine that brings this project up, and absent from your own shell. It is
**unfree** in nixpkgs, which is why `devenv.yaml` sets `allowUnfree: true`;
without it devenv refuses to evaluate the package.

The alternative is the vendor's installer run inside the sandbox
(`npm install -g`), which works too but is unpinned and host-linked
(`add-agent-workload` design.md, option (e)). This sample is the
reproducible shape.

The other packages are not decoration. The agent's shell tool runs `ls`,
`grep`, `git` and the like, no closure provides them unless asked, and the
host's are denied inside the sandbox, so without them the agent cannot read
a directory.

## What it reaches

`devcroft.toml` denies the network by default and allows
`api.anthropic.com` alone. Measured on one `claude -p` call: fourteen
requests to the API were allowed, and its telemetry host
(`http-intake.logs.us5.datadoghq.com`) was refused. Both are in the
sandbox's egress log.

The agent's config and history (`~/.claude`, `~/.claude.json`) land in the
sandbox's own home, `.devcroft/devenvclaude/home`, and not in yours. Your
real `~/.claude` is not readable from inside.

## What it depends on

Measured 2026-10-18, each of these stopped an agent before it was fixed:

- **A procfs of its own.** Claude Code is a Bun binary (the nixpkgs one
  too: a native ELF with its interpreter pointed at the closure's glibc),
  and Bun aborts within milliseconds without `/proc/self/maps` and
  friends. devcroft grants `/proc` only when the sandbox has its own PID
  namespace, so `policy --render` shows `/proc  pid-namespace` here. On a
  host that cannot give it one (Docker without `--security-opt
  systempaths=unconfined`, or macOS), `up` warns and the agent will not
  start.
- **A shell by name.** The agent and npm look for `sh` on `PATH` and in
  `SHELL`; devcroft supplies its resolved shell for both.
- **CA certificates.** devenv's environment names no bundle at all, so
  devcroft grants the host's system one and sets `SSL_CERT_FILE` to it
  (`trust-store` in `policy --render`). Claude Code brings its own roots
  and would not notice; Go and Rust agents (Crush, Codex, Goose) do.

## What is not covered

- **Subscription login is unmeasured.** `claude` stores a login in
  `~/.claude/.credentials.json`, which here is the sandbox's own home, so a
  login made inside persists across `up` and is removed by `rm`. Whether
  the login flow itself completes from inside a sandbox has not been tried.
- **The agent can read what the sandbox can.** That is the project root,
  writable, and the closure. An API key forwarded in is readable by any
  process in the sandbox, including the code the agent is editing; the
  boundary is between the agent and the rest of your machine, not between
  the agent and the project.
