# devcroft

Isolated development environments for each project, branch or coding agent on
your machine. Each sandbox gets its toolchain from a lockfile, a filesystem and
network boundary the kernel enforces, and an SSH endpoint for your editor. No
container, VM or image build.

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE-APACHE)
[![Rust](https://img.shields.io/badge/Rust-edition%202024-orange.svg)](Cargo.toml)
[![Status](https://img.shields.io/badge/release-0.0.1-yellow.svg)](#status)

**devcroft contains accidents in code you trust. It is not a security
boundary:** the sandbox shares your kernel. Run untrusted code in a container
or a VM. See [the threat model](docs/threat-model.md).

## Why

Worktrees give each branch its own directory, but they still share your
toolchain and your ports. devcroft gives each sandbox:

- **Its own toolchain**, from the project's flox, nix, devbox or devenv lock.
  All four share one store, so eight sandboxes of a project cost one build.
- **No host tools.** The shell, utilities and toolchain come from the lock or
  from devcroft itself, never from your machine (see the following section).
- **A kernel-enforced boundary** (Landlock on Linux, Seatbelt on macOS). Your
  SSH keys, cloud credentials and the rest of your disk are refused.
- **Its own ports on Linux.** Every branch can bind 5432; outbound traffic goes
  through a domain-filtered proxy.
- **An SSH endpoint** for VS Code, Cursor, `ssh`, `scp` and `rsync`.

### Why no host tools

Nothing inside a sandbox runs from your machine's `/usr/bin`, Homebrew or
`PATH`. That costs a line in the environment for every tool the project
uses, and it buys three things:

- **Builds that behave the same everywhere.** A build can't pass only because
  you happen to have a tool installed, or because your `sed` is BSD and CI's is
  GNU. What runs is what the lock says, for you, a teammate and CI.
- **Dependencies you can see.** A tool the project needs but doesn't declare
  fails on the first run, on your machine, instead of months later on someone
  else's.
- **Less for an agent to reach.** Your own `git`, `ssh` and `curl` can't run
  inside the sandbox. Copies from the project's toolchain don't find your
  credentials either: the sandbox has its own `HOME` (so no `.netrc` or
  `.gitconfig` of yours), your host's `~/.ssh` and cloud credential
  directories are denied, and your SSH agent isn't forwarded unless `devcroft.toml` sets
  `[ssh] forward_agent`. What you pass with `[env] forward` is the exception.

## Install

You need Rust 1.95 or later and an environment provider: flox, nix, devbox or
devenv (mise and pixi work for a narrower profile). devcroft isn't on
crates.io yet:

```sh
git clone https://github.com/dragosv/devcroft
cd devcroft
cargo install --path .
devcroft doctor   # what this machine has, and how to fix what's missing
```

## Quick start

In an empty directory, with flox:

```sh
flox init
flox install ripgrep        # flox 1.4+ asks you to log in to FloxHub first
devcroft init               # writes devcroft.toml
devcroft up
devcroft exec -- rg --version
```

Commands run with the packages from the lock. Basic utilities (`ls`, `cat`,
`pwd` and the rest of coreutils) and a shell come from devcroft itself when the
environment doesn't provide them, never from your host. Anything outside the
project directory is refused:

```sh
devcroft exec -- cat ~/.ssh/known_hosts   # fails with a permission error
```

## Configure

Everything outside the project directory stays denied until `devcroft.toml`
names it:

```toml
[sandbox]
name = "my-project"

[env]
provider = "flox"                 # or nix, devbox, devenv, mise, pixi
forward = ["GH_TOKEN"]            # the sandbox doesn't inherit your shell

[filesystem]
read = ["/opt/reference-data"]

[network]
default = "deny"
allow = ["api.example.com", "index.crates.io"]
ports = [5432]

[hooks]
post_create = "./scripts/bootstrap"   # runs inside the sandbox, once
```

Each sandbox has its own `HOME` under `.devcroft/`, and hooks run inside the
boundary, so a hook that needs the network needs an `allow` entry. To see what
the sandbox is allowed, and why:

```sh
devcroft policy --render                      # every rule, with its origin
devcroft why --host api.example.com
devcroft why --path ~/.ssh/id_rsa --op read
devcroft why --env GH_TOKEN
```

## Everyday use

**Editors.** Run `devcroft ssh-config --write` once. Each sandbox then answers
at `NAME.devcroft` for Remote-SSH, `ssh`, `scp`, `sftp` and `rsync`. Editor
support varies; see [the SSH validation matrix](docs/ssh-validation.md).

**Worktrees.** Give each worktree its own sandbox:

```sh
devcroft up --name feature-auth      # in worktree A
devcroft up --name feature-search    # in worktree B
```

**Ports.** On Linux a sandbox's ports are reachable inside it, not on your
loopback. Reach a dev server through SSH:

```sh
ssh -L 3000:127.0.0.1:3000 -N feature-auth.devcroft
```

On macOS sandboxes share your loopback, so devcroft can pick a free port per
sandbox for a service that reads its port from a variable.

**Services.** Services declared in a flox, devenv or devbox environment run
under the sandbox and show in `devcroft status`. The environment must include
`process-compose`.

**Coding agents.** Run an agent's commands through devcroft
(`devcroft exec feature-auth -- cargo test`), or run the agent inside from a
tooling layer of your own (`[tools] path = "~/agents"` in
`~/.config/devcroft/config.toml`). An API key can be brokered so the sandbox
never holds it. See [the agent matrix](docs/agents.md).

## Providers

| Provider | Guarantee | Input |
|---|---|---|
| flox | closure | `.flox/` manifest and lock |
| nix | closure | `flake.nix` and `flake.lock` |
| devbox | closure | `devbox.json` and lock |
| devenv | closure | `devenv.nix` and lock |
| mise | artifact | `mise.toml` and `mise.lock`, `http:` tools only |
| pixi | artifact | `pixi.toml` and `pixi.lock`, Conda packages only |

A **closure** is a complete package set from the shared store, independent of
your host. An **artifact** environment is checksum-pinned but links against
host libraries. No provider's activation hook runs on your host. See
[the decisions](docs/decisions.md) for each provider and each rejection.

This release still has a native macOS `swift` provider that uses your Xcode
and host tools. It's being removed from standard builds, so don't start a
project on it; use nix or flox for Swift.

## Commands

| Command | What it does |
|---|---|
| `init` | Write a `devcroft.toml` |
| `up`, `down`, `rm` | Start, stop, or stop and delete a sandbox |
| `exec`, `shell` | Run a command, or open a shell, inside a sandbox |
| `status`, `logs`, `ps` | Inspect one sandbox, its log, or all sandboxes |
| `policy --render`, `why` | Show the policy, or explain one decision |
| `doctor` | Check this host |
| `ssh`, `ssh-config` | Connect over SSH, or install the SSH configuration |

`devcroft --help` lists everything, including the experimental `fleet`
commands (Linux) and the `agent` commands used inside a sandbox.

## Platforms

| Platform | Mechanism | Minimum | Notes |
|---|---|---|---|
| Linux | Landlock, with user, mount, PID and network namespaces | Kernel 5.13 | On Ubuntu 24.04, set `kernel.apparmor_restrict_unprivileged_userns=0` |
| macOS | Seatbelt | 10.5 | No per-sandbox ports or process isolation; files, execution and egress are enforced |

## Status

`0.0.1`: used daily to develop devcroft, with no compatibility promise between
releases. Agents inside a sandbox are measured on Linux only, and `fleet` is
experimental. devcroft doesn't contain hostile code or undo project changes.
Every known limitation is in [the known gaps](docs/known-gaps.md).

**Documentation:** [threat model](docs/threat-model.md) ·
[known gaps](docs/known-gaps.md) · [decisions](docs/decisions.md) ·
[comparison](docs/comparison.md) · [base runtime](docs/base-runtime.md) ·
[agents](docs/agents.md) · [SSH and editors](docs/ssh-validation.md) ·
[roadmap](docs/roadmap.md) · [samples](samples/README.md)

## Contributing, security, license

See [CONTRIBUTING.md](CONTRIBUTING.md). devcroft is pre-1.0 and unaudited; a
private security channel is planned for 1.0, and until then report problems
in [the issue tracker](https://github.com/dragosv/devcroft/issues). Licensed
under [Apache-2.0](LICENSE-APACHE); dependency licenses are in
[THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).
