# devcroft

devcroft gives each project, branch or coding agent on your machine its own
sandboxed development environment. Each sandbox gets its toolchain from a
lockfile, a filesystem and network boundary that the kernel enforces, its own
network namespace on Linux, and an SSH server your editor connects to like any
remote machine. There's no daemon, image build, container or VM.

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE-APACHE)
[![Rust](https://img.shields.io/badge/Rust-edition%202024-orange.svg)](Cargo.toml)
[![Status](https://img.shields.io/badge/release-0.0.1-yellow.svg)](#status)

**devcroft contains accidents in code you already trust.** It's not a security
boundary against hostile code, a hostile prompt or a compromised agent: the
workload shares the host kernel. Run an unfamiliar repository or a stranger's
pull request in a container or a VM. [The threat model](docs/threat-model.md)
says where the line is.

## Why devcroft

Git worktrees give each agent its own directory, not its own environment. The
worktrees still share your installed toolchain and your services' ports: every
branch's committed `compose.yaml` wants Postgres on 5432, and the second one
fails. devcroft gives each sandbox:

- **Its own toolchain**, resolved from the project's lockfile through flox,
  nix, devbox or devenv, with one shared store so eight sandboxes of a project
  cost one build.
- **A boundary the kernel enforces**: Landlock on Linux, Seatbelt on macOS.
  Your SSH keys, cloud credentials and the rest of your disk are refused, not
  hidden by convention.
- **Its own ports on Linux.** Each sandbox gets its own network namespace, so
  every branch can bind 5432, while outbound access goes through a
  domain-filtered proxy.
- **A real SSH endpoint** for VS Code, Cursor, `ssh`, `scp` and `rsync`, over
  a Unix socket only you can open.

## Install

Before you install devcroft, you need:

- Rust 1.95 or later.
- One environment provider: flox, nix, devbox or devenv. mise and pixi are
  supported for a narrower profile, and on macOS a SwiftPM project can use the
  installed Xcode. devcroft doesn't manage packages; it sandboxes an
  environment that one of these produces.

devcroft isn't on crates.io yet. Install it from a clone:

```sh
git clone https://github.com/dragosv/devcroft
cd devcroft
cargo install --path .
```

To check what your machine has and what it lacks, run `devcroft doctor`. It
names a fix for each missing piece.

## Get started

devcroft needs a project that already has an environment. These steps use
flox in an empty directory:

1. Create an environment with a few packages:

   ```sh
   flox init
   flox install bash coreutils ripgrep
   ```

2. Write a `devcroft.toml` for the project:

   ```sh
   devcroft init
   ```

3. Start the sandbox:

   ```sh
   devcroft up
   ```

4. Run a command inside it:

   ```sh
   devcroft exec -- bash -c 'echo hello from inside; pwd'
   ```

   The output is the greeting and the project directory.

Commands inside the sandbox use the packages from the lockfile. They can read
and write the project directory and nothing else. For example, this command
fails with `Permission denied`:

```sh
devcroft exec -- cat ~/.ssh/known_hosts
```

## Configure a project

`devcroft init` writes a minimal manifest. Everything outside the project
directory stays denied until you name it:

```toml
[sandbox]
name = "my-project"

[env]
provider = "flox"          # or "nix", "devbox", "devenv", "mise", "pixi", "swift"
forward = ["GH_TOKEN"]     # the sandbox doesn't inherit your shell

[filesystem]
read = ["/opt/reference-data"]

[network]
default = "deny"
allow = ["api.example.com", "index.crates.io"]
ports = [5432]

[hooks]
post_create = "./scripts/bootstrap"   # runs inside the sandbox, once
```

Each sandbox has its own writable `HOME` inside the project's `.devcroft/`
directory, so tools that write to the home directory work without reaching
yours. Hooks run inside the boundary, so a hook that needs the network needs
an `allow` entry too.

The manifest compiles to a profile in which every rule names its origin. To
see the whole profile, or ask why one operation is allowed, use these
commands:

```sh
devcroft policy --render
devcroft why --host evil.example.net
devcroft why --path ~/.ssh/id_rsa --op read
devcroft why --env GH_TOKEN
```

## Run branches side by side

A committed manifest carries the same sandbox name into every worktree. To give
each worktree its own sandbox, name it at `up`:

```sh
devcroft up --name feature-auth      # in worktree A
devcroft up --name feature-search    # in worktree B
```

On Linux, each of these sandboxes can bind the same declared port. A port is
reachable inside its sandbox and through an SSH tunnel, not on the host's
loopback. To reach a dev server on port 3000:

```sh
ssh -L 3000:127.0.0.1:3000 -N feature-auth.devcroft
```

Where sandboxes share the host's loopback (macOS, or
`network.default = "allow"`), devcroft can pick a free port per sandbox for a
service that reads its port from a variable. Services declared in a flox,
devenv or devbox environment run under the sandbox's supervisor, and
`devcroft status` reports each one.

## Connect your editor

Add the SSH configuration once:

```sh
devcroft ssh-config --write
```

Each sandbox then answers at `NAME.devcroft`, where *`NAME`* is the sandbox
name. Open it with VS Code or Cursor Remote-SSH, or use `ssh`, `scp`, `sftp`
and `rsync`. Each editor has its own preconditions, and some don't work yet;
[the SSH validation matrix](docs/ssh-validation.md) has the details.

## Use a coding agent

You can keep the agent on the host and run its consequential commands through
devcroft:

```sh
devcroft exec feature-auth -- cargo test
```

You can also run an agent inside the sandbox from a locked tooling layer of
your own, which the project never names. Configure it in
`~/.config/devcroft/config.toml`:

```toml
[tools]
path = "~/agents"
```

An API key can be brokered so that the sandbox never holds it.
[The agent matrix](docs/agents.md) covers the eleven agents measured inside a
sandbox, brokering, and what doesn't work yet.

## Environment providers

| Provider | Guarantee | Project input |
|---|---|---|
| flox | closure | `.flox/` manifest and lock |
| nix | closure | `flake.nix` and `flake.lock` |
| devbox | closure | `devbox.json` and lock |
| devenv | closure | `devenv.nix` and lock |
| mise | artifact | `mise.toml` and `mise.lock`, `http:` tools only |
| pixi | artifact | `pixi.toml` and `pixi.lock`, Conda packages only |
| swift | artifact, macOS only | a SwiftPM package and the installed Xcode |

A **closure** is a complete package set from a shared, content-addressed
store, so nothing inside depends on what's installed on your host. An
**artifact** environment depends on the host. mise and pixi install
checksum-pinned objects that link against host libraries, and swift uses the
Xcode installed on that Mac, with no shared store. No provider's own
activation hook runs on your host. There's no `host` or `none` provider.
[The decisions](docs/decisions.md) explain each provider and each rejection.

## Commands

`devcroft --help` prints the same list:

| Command | What it does |
|---|---|
| `init` | Write a `devcroft.toml` for this project |
| `up`, `down`, `rm` | Start, stop, or stop and delete a sandbox |
| `exec`, `shell` | Run one command, or open a shell, inside a sandbox |
| `status`, `logs`, `ps` | Inspect one sandbox, its keeper's log, or every sandbox on this host |
| `policy --render`, `why` | Show the compiled profile, or explain one decision |
| `doctor` | Check this host for what devcroft needs |
| `ssh`, `ssh-config`, `proxy` | Connect over SSH, install the SSH configuration, and the `ProxyCommand` handler |

## Platform support

| Platform | Mechanism | Minimum | Notes |
|---|---|---|---|
| Linux | Landlock, with user, mount, PID and network namespaces | Kernel 5.13 | Ubuntu 24.04 restricts unprivileged user namespaces by default, and `up` refuses until you set `kernel.apparmor_restrict_unprivileged_userns=0` |
| macOS | Seatbelt | 10.5 | No mount or network namespace, so services share ports and process visibility isn't narrowed; execution, reads, writes and egress are enforced |

## Status

devcroft `0.0.1` works and is used daily to develop devcroft itself. The
version is deliberate: nothing here promises compatibility with the next
release. Coding agents inside a sandbox are measured on Linux only, and
`devcroft fleet`, which runs many agents with resource budgets, is
experimental and Linux only.

devcroft doesn't contain hostile code or kernel exploits, undo project
changes, or deliver a subscription login to an agent inside. For a stronger
boundary, run devcroft inside a VM. `devcroft doctor` shows what your host
enforces, and [the known gaps](docs/known-gaps.md) describe every published
limitation, with measurements.

## Documentation

| Document | What it covers |
|---|---|
| [Threat model](docs/threat-model.md) | Which use case the isolation backs, and which it doesn't |
| [Known gaps](docs/known-gaps.md) | Every published limitation |
| [Decisions](docs/decisions.md) | Why devcroft does or doesn't support something |
| [Comparison](docs/comparison.md) | Dev Containers, `nono-cli`, flox alone and coding-agent products |
| [Base runtime](docs/base-runtime.md) | Where a sandbox's shell and utilities come from |
| [Agents](docs/agents.md) | Coding agents measured inside a sandbox |
| [SSH validation](docs/ssh-validation.md) | SSH clients and editors |
| [Roadmap](docs/roadmap.md) | What each release has to be true for |
| [Samples](samples/README.md) | Runnable projects, one or more per provider |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, tests and the OpenSpec
workflow. [AGENTS.md](AGENTS.md) holds the rules every change must keep.

## Security

devcroft is pre-1.0, unaudited software from a single maintainer, with no
security response commitment. Read [the threat model](docs/threat-model.md)
before you rely on it. A private reporting channel is a 1.0 requirement and
doesn't exist yet. Until it does, open an issue in
[the issue tracker](https://github.com/dragosv/devcroft/issues).

## License

devcroft is licensed under [Apache-2.0](LICENSE-APACHE), like nono and the
sigstore crates it links. Dependency license texts are in
[THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).
