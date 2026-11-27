# Samples

Each sample is a standalone project with its own `devcroft.toml` and a README
that says what it demonstrates and how to run it. To try one, change to its
directory and run `devcroft up`.

The Rust samples (`flox-clap-sample`, `flox-rustup-sample`,
`nix-flake-sample` and `devbox-citytime-sample`) each have a `[workspace]`
table of their own, so cargo doesn't pull them into devcroft's workspace.

## By provider

| Sample | Provider | What it shows |
|---|---|---|
| [flox-clap-sample](flox-clap-sample/) | flox | A Rust CLI (`citytime`) built with clap |
| [flox-rustup-sample](flox-rustup-sample/) | flox | The toolchain split `devcroft init` is built around |
| [flox-services-sample](flox-services-sample/) | flox | `network.ports` and supervised services |
| [port-allocation-sample](port-allocation-sample/) | flox | Two sandboxes of one project running the same service on a shared loopback |
| [nix-flake-sample](nix-flake-sample/) | nix | The same `citytime` CLI from a flake |
| [nix-go-sample](nix-go-sample/) | nix | A Go HTTP server (Gin) |
| [nix-probe-sample](nix-probe-sample/) | nix | The boundary probe: reads and deletions the sandbox refuses |
| [kotlin-ktor-sample](kotlin-ktor-sample/) | nix | A Kotlin HTTP server (Ktor) with Gradle |
| [devbox-citytime-sample](devbox-citytime-sample/) | devbox | The `citytime` CLI with no dependency beyond `std` |
| [devbox-services-sample](devbox-services-sample/) | devbox | Postgres from the package's devbox plugin |
| [devenv-sample](devenv-sample/) | devenv | A project hook that runs inside the sandbox |
| [devenv-services-sample](devenv-services-sample/) | devenv | Services, in the same shape as `flox-services-sample` |
| [mise-sample](mise-sample/) | mise | A locked Go build from `http:` tools |
| [pixi-sample](pixi-sample/) | pixi | Python and numpy from conda-forge |
| [pixi-go-sample](pixi-go-sample/) | pixi | A Go service with its whole toolchain from pixi |
| [swift-spm-sample](swift-spm-sample/) | swift | A SwiftPM package on macOS, the only artifact-tier sample with no shared store |

## Coding agents

Eleven `devenv-AGENT-sample` projects each run one coding agent inside a
sandbox, with that agent's provider host as the only allowed destination:
[aider](devenv-aider-sample/), [amp](devenv-amp-sample/),
[antigravity](devenv-antigravity-sample/), [claude](devenv-claude-sample/),
[codex](devenv-codex-sample/), [copilot](devenv-copilot-sample/),
[crush](devenv-crush-sample/), [cursor](devenv-cursor-sample/),
[goose](devenv-goose-sample/), [opencode](devenv-opencode-sample/) and
[qwen](devenv-qwen-sample/). [The agent matrix](../docs/agents.md) has the
results.

## Facts the samples established

These matter when you add a sample or touch the code it exercises:

- **`flox-services-sample` is the regression case for the shell invariant.**
  Its manifest declares no shell, like every real flox manifest.
- **`devenv-antigravity-sample` is the one agent that needs
  `network.loopback = "any"`**, because `agy` binds its language server on
  `127.0.0.1:0`. That key drops Landlock's TCP filter and relies on the
  route-less namespace, so it's refused wherever there's none. Without the
  namespace, the agent reached `1.1.1.1` directly, past the proxy (measured).
  The agent is a Bun binary that aborts without `/proc`, so it needs the
  sandbox's own PID namespace. It replaced `devenv-gemini-sample` on
  2026-11-14.
- **`port-allocation-sample` uses `network.default = "allow"` on purpose.**
  Every `"deny"` sandbox on Linux has its own network namespace and allocates
  nothing.
- **`nix-probe-sample` measures the README's boundary claims.** It also
  established three nix facts:
  - a `shellHook` export never arrives, so redirects must be `mkShell`
    attributes;
  - nix reports a `TMPDIR` that no longer exists at session time, and
    `GOTMPDIR` is the lever;
  - `[env.vars]` used to parse and do nothing, which is now fixed and covered
    by `tests/env_vars_injected.rs`.

  Its deletion probe targets a file you create by hand, never one the program
  creates: a demonstration of a boundary must not be destructive when the
  boundary is absent.
- **`devbox-citytime-sample` depends only on `std`**, because devcroft never
  runs devbox's `shell.init_hook`, so there's no host-side hook to fetch
  crates in.
- **`devenv-services-sample` is named `devenvsvc`.** The directory slug would
  make the supervisor's socket path 112 bytes, past the 103-byte limit, and
  `up` refuses. Keep this in mind before you add a services sample with a long
  name.
- **`devbox-services-sample` runs the hardest service of any sample.** Its
  Postgres needed non-root sessions and identity files written into the view.
  It's Linux only, because Postgres needs `shmget` and Seatbelt refuses it.
- **`swift-spm-sample` differs on purpose.** Read its README before you copy
  anything from it. Its `import AppKit` is load-bearing: a Foundation-only
  package draws advice to use a closure provider instead.
- **`kotlin-ktor-sample` was `gvisor-kotlin-sample`.** It was renamed when the
  hardened tier was removed, and its `isolation = "hardened"` key was dropped.
