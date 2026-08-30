# devcroft

Isolated, reproducible development environments built on OS-level sandboxing
— kernel primitives instead of containers or VMs — each reachable over SSH.

## Why

Running several coding agents, or several branches of the same project,
on one machine gets messy fast. Git worktrees give each one its own
directory but not its own *environment*: they still share your installed
toolchain, one `target/` directory, and — the part that actually stops
you — your services and their ports.

That last one has no good workaround. Your `docker-compose.yml` or
`devcroft.toml` is committed, so every branch declares the same Postgres
on 5432. Start two and the second fails. The usual escape is one shared
database with a schema per branch, which means migrations from one agent
are visible to another, a destructive migration takes out everybody, and
"works on my branch" stops meaning anything. Today, only a container or a
VM per branch really solves it, and both are heavy enough that people
run four and stop.

That is the gap devcroft closes: a sandbox that declares services or
ports and wants no outbound network of its own — the common shape for a
local Postgres used only by tests — gets its own network namespace. Same
committed port, every sandbox, no collision, no config to write. It's
narrower than it sounds: a sandbox that *also* wants to reach the
internet can't get this yet, since an isolated namespace has no route out
without a forwarding helper devcroft doesn't have. That combination still
collides today, same as before this existed — see [Status](#status).

Containers solve the isolation but cost too much to run eight of, and
they don't give you a reproducible toolchain by themselves — you still
write a Dockerfile and hope it matches what everyone else built.

devcroft is the middle: each project or agent gets a reproducible
environment and a boundary the kernel enforces, cheap enough to run many
side by side, reachable over SSH so your editor connects to any of them.

## Overview

You describe a project's environment and what it's allowed to touch in a
config file. devcroft turns that into a sandbox the operating system
itself enforces — not a container, not a VM — and gives you SSH into it,
so your existing editor connects like it would to any other remote
machine. It doesn't build the sandboxing itself; it configures and
supervises the OS features that do (Landlock on Linux, Seatbelt on
macOS).

The environment comes from a package manager that can rebuild it exactly
— flox, nix, or devbox — so what's inside the sandbox is the same on
every machine, and eight sandboxes of the same project cost one build,
not eight, because they share a single content-addressed store.

The closest relative is [nono-cli](https://github.com/nolabs-ai/nono),
the CLI for the sandboxing library devcroft depends on. It's
general-purpose: point it at any command and open up whatever host files
that command needs, case by case. devcroft is narrower on purpose —
built for development environments, where what's inside comes from the
package manager rather than from your machine. See
[docs/comparison.md](docs/comparison.md) for that in full, plus Dev
Containers, flox alone, and how the major coding-agent products
provision environments today.

## Installation

Not yet published to crates.io ([status](#status)). Build from source:

```sh
git clone https://github.com/dragosv/devcroft
cd devcroft
cargo build --release
```

Requires Rust stable and edition 2024 to build, plus one of `flox`, `nix`,
or `devbox` installed — devcroft does not manage packages itself, it
sandboxes an environment one of those produces.

## Usage

devcroft needs a project that already has an environment. If yours
doesn't, create one first — devcroft won't do this for you:

```sh
flox init                  # create the environment
flox install bash coreutils ripgrep   # whatever your project needs
```

An environment with nothing installed produces an empty sandbox where
even `bash` won't run, so install something before continuing.

Then:

```sh
devcroft init                  # write devcroft.toml
devcroft up                    # build the environment, apply the policy, start the sandbox
devcroft exec -- cargo test    # run one command inside it
devcroft shell                 # or get an interactive shell
devcroft status
devcroft down                  # stop it; `rm` also deletes its state
```

`devcroft init` reports whether it found an environment, so you'll know
before `up` whether you skipped a step.

A minimal `devcroft.toml`:

```toml
[sandbox]
name = "my-project"

[env]
provider = "flox"   # or "nix", "devbox"
```

The project directory is readable and writable by default. Everything
else — other paths, network access, ports — is denied unless you ask for
it:

```toml
[filesystem]
read = ["/path/to/a/read-only/dir"]

[network]
default = "deny"
allow = ["api.example.com"]
```

### Connecting an editor

```sh
devcroft ssh-config --write    # adds a block to ~/.ssh/config, once
```

Each sandbox then answers at `<name>.devcroft` — open it with VS Code or
Cursor's Remote-SSH, or use it directly with `ssh`, `scp`, `rsync`, and
`ssh -L` port forwarding. Nothing listens on a TCP port; the connection
goes over a unix socket only your user can reach.

## Features

- **Three environment providers** — flox, nix flakes, devbox. Each builds
  a *closure*: a complete, self-contained set of packages, so what runs
  inside the sandbox doesn't depend on what you happen to have installed.
  A config file plus its lockfile is what makes it reproducible; there's
  no "just use whatever's on the host" fallback.
- **A kernel-enforced boundary** — Landlock (Linux) or Seatbelt (macOS).
  Catches mistakes, not attacks — see [Limitations](#limitations).
- **A real SSH server per sandbox** — `exec`, `shell`, and SSH all work
  with existing editors (VS Code, Cursor; OpenSSH and rsync validated
  end to end).
- **Background services** — Postgres, Redis, whatever your environment
  declares — started and stopped with the sandbox.
- **Network allowlists** — name the hosts a project may reach; everything
  else is refused by the kernel, not by asking nicely. Enforced on Linux.
- **Policy you can read** — the same config always produces the same
  rules, and `devcroft policy --render` shows every one of them with the
  reason it exists. To check one thing:
  `devcroft why --path /some/file --op read`.

## Platform Support

| Platform | Mechanism | Minimum Version |
|----------|-----------|-----------------|
| Linux | Landlock | Kernel 5.13+ |
| macOS | Seatbelt | 10.5+ |

Same floor as [nono](https://github.com/nolabs-ai/nono), the sandboxing
library devcroft is built on. Verified end to end against real tooling in
this repo's own Linux devcontainer; `cargo test` self-skips where a
provider or kernel feature is missing rather than passing vacuously. macOS
is implemented but has no CI host measuring it yet — see
[known-gaps.md](docs/known-gaps.md).

## Status

Working and used daily in this repo's own development, but **not yet
released**: publishing to crates.io is deliberately held until the last
MVP task closes (an editor-compatibility matrix — everything but Zed
passes). Until then, build from source. Expect the command surface to be
stable and the rough edges below to be real.

**Commands**: `init`, `up`, `down`, `rm`, `status`, `logs`, `ps`,
`shell`, `exec`, `ssh`, `proxy`, `ssh-config`, `policy`, `why`, `doctor`.

Known gaps are published rather than hidden — see
[docs/known-gaps.md](docs/known-gaps.md) for the detail behind each of
these:

- **Sandboxes with any outbound network still share the host's ports.**
  Fixed for sandboxes that want zero outbound network (`network.default =
  "deny"`, no `network.allow`) and declare services or ports — each gets
  its own network namespace, so the same committed port works in every
  one. A sandbox that also wants to reach the internet can't get this yet
  and still collides, same as before. This is the gap the
  [Why](#why) section is about.
- **Unix sockets bypass the policy.** Landlock mediates TCP, not AF_UNIX, so
  a sandbox reaches any unix socket the filesystem permits — including a nix
  daemon socket, which grants it that daemon's authority.
- No inter-sandbox process visibility separation.
- Domain filtering is enforced on Linux; unverified on macOS.
- No cgroup resource limits.
- Provisioning runs on the host, except flox's activation hook, which now
  runs inside the sandbox.
- A `filesystem.allow` grant for a path that doesn't exist yet is silently
  dropped.
- Zed's remote server connects and transfers but does not start.

**Being worked on:** extending network isolation to sandboxes that also
want outbound access, which needs a forwarding helper this host doesn't
have yet; running many agents on one host with per-agent resource
budgets; building the environment itself inside a sandbox rather than on
the host; and a way for an agent that gets stuck to say so instead of
looking identical to one that's busy. Specs for all of these are in
[openspec/changes/](openspec/changes/) — `openspec list`
shows live progress if you've cloned the repo.

The full build history — what was built, what turned out to be wrong, and
what the corrections cost — is in
[docs/implementation-log.md](docs/implementation-log.md).

## Limitations

**What the boundary does stop.** The kernel enforces it, so it holds
against ordinary mistakes regardless of what the process intends:

```
$ devcroft exec -- bash -c 'cat ~/.ssh/known_hosts'
cat: /home/vscode/.ssh/known_hosts: Permission denied
```

Writes outside the project directory, reads of your credentials, and
connections to hosts you didn't allowlist all fail the same way. An agent
that decides to `rm -rf ~` gets `EPERM`, not your home directory.

**What it doesn't stop.** The full host kernel is still reachable from
inside, so a kernel exploit escapes. This catches an agent that
misbehaves, deletes the wrong directory, or fights another agent for a
port — it does not contain code written by someone trying to break out.

For that, run devcroft inside a VM. That's the supported answer, not a
deflection: it's already how the macOS path works. See
[docs/threat-model.md](docs/threat-model.md) for which use case each one
actually backs.

## Documentation

| | |
|---|---|
| [openspec/changes/add-mvp-core/](openspec/changes/add-mvp-core/) | The MVP — proposal, design, tasks, 7 capability specs |
| [docs/comparison.md](docs/comparison.md) | How devcroft compares to Dev Containers, flox alone, `nono-cli`, and today's coding-agent products |
| [docs/known-gaps.md](docs/known-gaps.md) | Full detail behind the Status section's gap list |
| [docs/decisions.md](docs/decisions.md) | Every "why doesn't devcroft support X", answered falsifiably |
| [docs/threat-model.md](docs/threat-model.md) | Which use case the isolation tier backs, and which it doesn't |
| [docs/implementation-log.md](docs/implementation-log.md) | Build history — what was built, and what turned out to be wrong |
| [docs/ssh-validation.md](docs/ssh-validation.md) | SSH client/editor validation matrix — OpenSSH, rsync, VS Code Remote-SSH, Cursor, Zed |
| [CLAUDE.md](CLAUDE.md) | Architecture invariants and repo conventions |
| [samples/](samples/) | Real, verified sample projects — one per environment provider |
| [THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md) | License texts for every dependency linked into the binary |

```sh
openspec list             # active changes and task progress
openspec validate --all   # validate delta specs
```

## License

[Apache-2.0](LICENSE-APACHE) — matching `nono`, the sandboxing library
devcroft is built on, and the sigstore crates in its dependency tree.
Chosen over the Rust-conventional `MIT OR Apache-2.0` deliberately: a
dual license would let a user select MIT and take no patent grant, while
changing nothing about their actual obligations, since `nono`, `russh`
and every `sigstore-*` are Apache-2.0-only and linked in regardless.

A compiled binary statically links 335 dependencies, 189 of them
Apache-2.0. Their license texts are reproduced in
[THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md), regenerated by
`python3 scripts/gen-third-party-licenses.py` whenever `Cargo.lock`
changes.
