# devcroft

Reproducible development environments, sandboxed by the operating system, each
reachable over SSH.

**Run several branches of the same project side by side on one machine, each in
its own sandbox — its own tools, its own ports, its own services, and no access
to anything outside its project directory — without Docker or a VM.**

A process inside a sandbox cannot read your `~/.ssh`, write to `/etc`, or delete
a file in your home directory — the kernel refuses, nothing has to cooperate.
Your code still runs natively on your own OS (on a Mac, on macOS), and your
editor connects like to any remote machine. Nothing to install system-wide and
no image to build: once a project's environment exists, `up` takes about 0.2 s
and `exec` into a running sandbox about 20 ms.

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE-APACHE)
[![Rust](https://img.shields.io/badge/Rust-edition%202024-orange.svg)](Cargo.toml)
[![Status](https://img.shields.io/badge/release-0.0.1-yellow.svg)](#status)

> **Not a security boundary.** devcroft contains accidents and keeps projects out
> of each other's way. It does not stop code that is actively trying to escape,
> and on macOS it does not restrict which binaries run at all. Use a container or
> a VM for code you don't trust — [docs/threat-model.md](docs/threat-model.md)
> says which use case each one backs.

## The problem

Git worktrees give each branch its own directory, not its own *environment*. They
still share your installed toolchain, one `target/`, and — the part with no good
workaround — your services and their ports. Your `compose.yaml` is committed, so
every branch declares Postgres on 5432; start two, and the second fails. The
usual escape is one shared database with a schema per branch, which means one
destructive migration takes out everybody. A container per branch really does fix
it, and costs enough that people run four and stop.

devcroft gives each sandbox **its own network namespace** whenever
`network.default = "deny"` — the setting anything declaring ports or services
needs anyway. The same committed port in every sandbox, no collision, no
allocation, nothing for the service to cooperate with, and outbound access still
works inside it.

## Install

```sh
git clone https://github.com/dragosv/devcroft && cd devcroft
cargo install --path .
```

**To build it** you need Rust 1.85 or newer, and only because there is no
crates.io release yet — see [Status](#status).

**To use it** you need `flox`, `nix`, or `devbox` on the machine. devcroft does
not manage packages; it sandboxes an environment one of those produces, and
without one there is nothing for it to sandbox. (Devbox is built on Nix, so it
needs a working Nix underneath either way.) Run `devcroft doctor` to check this
host before the first `up`.

## Run it!

Every sample in this repo is a working project. The smallest one needs nothing but
a `nix` that can build:

```console
$ cd samples/nix-probe-sample
$ devcroft up
devcroft: bringing up sandbox 'nix-probe-sample'...
devcroft: sandbox 'nix-probe-sample' is started.

$ devcroft exec -- go run .
hello from inside
/path/to/devcroft/samples/nix-probe-sample
```

That is the whole loop: `up` materializes the environment and applies the policy,
`exec` runs a command inside it. `devcroft down` stops the sandbox; `devcroft ps`
lists every sandbox on the host.

In your own project you need an environment file one of the providers understands
— a `flox` environment, a `flake.nix`, or a `devbox.json`. `devcroft init` writes
the devcroft side of it:

```sh
cd my-project
flox init && flox install go     # or: nix flake init / devbox init
devcroft init
devcroft up
devcroft exec -- go version
```

Two things worth knowing up front. The **first** `up` in a project is however long
the provider needs to build the closure — minutes, possibly. Every one after that
is ~0.23 s, and `exec` ~0.02 s (M4 Pro, macOS 15, best of five), because the
closure is already in the store. And **caches**: anything outside the project root
is denied, while most toolchains default their cache to `$HOME` — Go wants
`GOCACHE` and `GOTMPDIR` redirected, Rust wants `CARGO_HOME`. Each sample's README
shows the redirect for its ecosystem.

## Every branch gets the same port

This is the part a container gives you and a worktree doesn't. A sandbox that
declares `network.ports` or `[services]` gets its own network namespace, so its
ports belong to it alone — eight checkouts of one project can all bind 5432, and
none of them know about the others.

The tradeoff is stated out loud at `up`, not discovered later:

```console
$ devcroft up
devcroft: bringing up sandbox 'my-project'...
devcroft: note: this sandbox has its own network namespace, so its declared port(s) 5432 are reachable from inside it and through `ssh -L <local>:127.0.0.1:<port> my-project.devcroft`, but not directly on the host's own loopback
devcroft: sandbox 'my-project' is started.
```

So reach a dev server through the tunnel — `ssh -L 3000:127.0.0.1:3000 -N
my-project.devcroft`, then open `localhost:3000`. A sandbox with
`network.default = "allow"` isn't isolated, and its ports stay directly reachable.

Services declared in your environment are supervised with the sandbox: devcroft
generates its own process-compose config from them, starts them before your hooks
run, reports each one's state in `devcroft status`, and reaps them at `down`.
`process-compose` has to be a member of the project's environment; devcroft
refuses at `up` if it isn't, rather than coming up with services that silently
never start. A shell is *not* your job — devcroft resolves one out of the closure
itself.

## Verify the boundary

The claim is testable, so test it.
[samples/nix-probe-sample](samples/nix-probe-sample/) is a small Go program that
asks for three things outside the project root — reading `~/.ssh/known_hosts`,
writing `/etc/devcroft-probe`, deleting a throwaway file in your home directory:

```console
$ cd samples/nix-probe-sample
$ touch ~/devcroft.tmp                      # the deletion target, yours to make
$ devcroft exec -- go run . probe "$HOME"
probing home: /home/you
open /home/you/.ssh/known_hosts: permission denied
open /etc/devcroft-probe: permission denied
remove /home/you/devcroft.tmp: permission denied

$ ls ~/devcroft.tmp                         # still there: the refusal held
/home/you/devcroft.tmp
```

Nothing was asked politely and nothing cooperated. The sample's README has the
control run — the same program, unconfined, deleting the same file — because a
refusal only demonstrates a boundary if the same operation succeeds without one.

**On macOS the third line is the only one you can rely on.** Seatbelt does not
mediate execution, so a process naming an absolute host path gets the host's
binary whatever the policy says; reads and writes are still refused. `devcroft
doctor` prints this as a warning on macOS, and
[docs/known-gaps.md](docs/known-gaps.md) has the measurement.

## Connect your editor

```sh
devcroft ssh-config --write     # adds a block to ~/.ssh/config, once
```

Each sandbox then answers at `<name>.devcroft` — open it with VS Code or Cursor
Remote-SSH, or use `ssh`, `scp`, `sftp`, `rsync` and `-L` forwarding directly.
Nothing listens on a TCP port; the connection goes over a unix socket only your
user can open. What each editor actually needs, negative results included, is
measured in [docs/ssh-validation.md](docs/ssh-validation.md).

## Configure it

`devcroft init` writes a minimal file. Everything outside the project directory
stays denied until you name it:

```toml
[sandbox]
name = "my-project"

[env]
provider = "flox"          # or "nix", "devbox"

[filesystem]
read = ["/tmp"]            # read-only; grant it at all only if a tool needs it

[network]
default = "deny"
allow = ["api.example.com", "index.crates.io"]
ports = [5432]
```

That compiles to a profile that is deterministic and inspectable — every rule
carries the reason it exists, and nothing reaches the kernel that `policy
--render` won't show you:

```console
$ devcroft policy --render
filesystem.allow:
  .                                        manifest:filesystem.allow
filesystem.read:
  /tmp                                     manifest:filesystem.read
  /nix/store                               provider:flox
  /lib                                     baseline
filesystem.deny:
  ~/.ssh                                   baseline
  ~/.aws                                   baseline
[...elided: the rest of the baseline loader, /dev and credential paths...]

$ devcroft why --host evil.example.net
DENIED
denied by rule manifest:network.default (host evil.example.net is not in the allowlist)
```

Baseline denials always win, including over devcroft's own data directory.

## Commands

The surface is closed for 0.0.x — this is all of it, and `devcroft --help` prints
the same thing:

```console
sandboxes
  init [--force]              write a devcroft.toml for this project
  up [name] [--recreate]      build the environment, apply the policy, start the sandbox
  down [name]                 stop a sandbox, keeping its state
  rm [name] [--yes]           stop a sandbox and delete its state

running things
  exec [name] -- <cmd>        run one command inside a sandbox
  shell [name]                open an interactive shell inside a sandbox

inspecting
  status [name]               whether a sandbox is up, and since when
  logs [name] [--tail N]      the keeper's log
  ps                          every sandbox on this host
  policy --render [name]      the compiled profile, every rule with its origin
  why --path P --op <mode>    whether one operation is allowed, and which rule decides
  why --host <domain>         the same question for an outbound host
  doctor                      check this host for what devcroft needs

ssh
  ssh [name]                  connect over the sandbox's own SSH server
  ssh-config [--write]        emit (or install) the ~/.ssh/config block
  proxy <name>.devcroft       ProxyCommand handler; not typed directly
```

## Environments

Use **flox** (the default `devcroft init` writes), **nix flakes** (pick this if the
project already has a `flake.nix`), or **devbox**. Each sandbox installs exactly
what the project's lockfile says — a complete, self-contained package set — so
every checkout gets the same environment regardless of what you happen to have
installed, and eight sandboxes of one project cost one build.

There is no "just use the host" fallback, on purpose: if a project has no
environment yet, the answer is `flox init`, not a degraded mode. Why devenv, mise,
pixi and hermit aren't here — each for a stated, falsifiable reason — is in
[docs/decisions.md](docs/decisions.md).

## How it compares

Dev Containers is the closest thing in wide use, and the difference is a trade
rather than a win:

| | devcroft | Dev Containers |
|---|---|---|
| What your code runs on | The host OS — on a Mac, macOS | Linux, always |
| Isolation | Kernel primitives (Landlock/Seatbelt) — accident protection, not a security boundary | A real container boundary |
| Cost per environment | Low — shared Nix store, no rootfs or guest kernel | Image layers, plus a VM on macOS |
| Reproducibility | Mandatory — config + lockfile, no host fallback | Optional — you write a Dockerfile and hope |
| Editor access | A real SSH server per sandbox | Native, through the container |

**The first row decides it for a lot of people.** On a Mac, a container or a VM
means your code never executes on macOS at all — you cannot build or test a macOS
binary, an Apple framework, or a codesigned artifact, and you cannot reproduce a
macOS-only bug on a Linux kernel. devcroft never virtualizes anything: your
processes are native host processes with a policy applied to them. The cost of that
choice is the whole Status section below — a native process under Landlock or
Seatbelt is a weaker boundary than a guest kernel. Same trade in both directions,
which is why this is a table and not a scoreboard.

Run code you genuinely don't trust in a container or a VM. Run eight branches on
one laptop in devcroft. [docs/comparison.md](docs/comparison.md) has that in full,
plus `nono-cli`, flox alone, and how today's coding-agent products provision
environments.

## Status

**0.0.1 — working, and used daily in this repo's own development.** The number is
deliberate: `0.0.z` is the only range cargo treats as incompatible with itself, so
nothing here promises compatibility with the next version. `0.1.0` is held back
rather than skipped, until the boundary matches what this page says about it — see
[docs/roadmap.md](docs/roadmap.md).

**The boundary catches mistakes, not attacks.** The full host kernel is reachable
from inside, so a kernel exploit escapes, and **unix sockets bypass the policy
entirely** — Landlock mediates TCP, not AF_UNIX. For a real boundary, run devcroft
inside a VM; that is the supported answer, and already how the macOS path works.

**Agent fleets are not runnable end to end yet.** Several branches of one project,
each with its own environment, ports and services, works today and is what this
page demonstrates. Running a coding agent *inside* a sandbox needs its runtime and
credentials reachable there, which is `add-agent-workload` — specified, not built.

| Platform | Mechanism | Minimum version |
|----------|-----------|-----------------|
| Linux | Landlock | Kernel 5.13+ |
| macOS | Seatbelt | 10.5+ |

**The two backends do not enforce the same things.** Measuring macOS turned up four
gaps Linux does not have: execution is not mediated at all; grants match paths as
spelled rather than per directory; `network.ports` does not limit what a process
may bind; and `/dev` is granted read-write. On macOS, treat the boundary as
protection against accidental *reads and writes* outside the project, and nothing
more — `devcroft doctor` reports these on the host it runs on.

Everything else is written up rather than summarised away — no rollback, no cgroup
limits, network isolation needing unprivileged user namespaces, no inter-sandbox
process visibility separation, Zed's remote server:
[docs/known-gaps.md](docs/known-gaps.md). This is pre-1.0, unaudited,
single-maintainer software with no security SLA; if you find a problem,
[open an issue](https://github.com/dragosv/devcroft/issues) like any other bug.

## Ready to go deep?

| | |
|---|---|
| [docs/comparison.md](docs/comparison.md) | Dev Containers, `nono-cli`, flox alone, and today's coding-agent products |
| [docs/known-gaps.md](docs/known-gaps.md) | Every published gap, in full |
| [docs/threat-model.md](docs/threat-model.md) | Which use case the isolation backs, and which it doesn't |
| [docs/roadmap.md](docs/roadmap.md) | What 0.2 through 1.0 each have to be true for, and why in that order |
| [docs/decisions.md](docs/decisions.md) | Every "why doesn't devcroft support X", answered falsifiably |
| [docs/ssh-validation.md](docs/ssh-validation.md) | SSH client and editor matrix — OpenSSH, rsync, VS Code, Cursor, Zed |
| [samples/](samples/) | Verified sample projects, one per environment provider |

Contributors: [CLAUDE.md](CLAUDE.md) holds the architecture invariants,
[docs/implementation-log.md](docs/implementation-log.md) the build history
including what turned out to be wrong, and [openspec/changes/](openspec/changes/)
the specs (`openspec list` shows live progress).

## License

[Apache-2.0](LICENSE-APACHE), matching `nono` and the sigstore crates devcroft
links, rather than the Rust-conventional `MIT OR Apache-2.0` — a dual license would
let a user take MIT and no patent grant while changing none of their real
obligations. Dependency license texts are in
[THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).
