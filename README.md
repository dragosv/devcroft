# devcroft

Run your project's builds, tests and services in a sandbox, so a command that
goes wrong can't reach the rest of your machine — and so every branch gets its
own tools, ports and databases.

No daemon running in the background, no container, no VM. Your code runs
natively on your own machine, which on a Mac means it actually runs on macOS.

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE-APACHE)
[![Status](https://img.shields.io/badge/release-0.0.1-yellow.svg)](#what-works-today)

**Nothing escapes the project folder.** Anything you run through devcroft — a
build, a test suite, a script a coding agent decided to run — cannot read your
SSH keys or cloud credentials, cannot write outside the project, and cannot
reach a server you didn't allow. Whatever it starts is caught too — a script
that runs a script that runs a compiler, the whole tree. And there is no way
back out: no flag, no setting, nothing a program inside can call to ask for
more.

**Everyone gets the same tools.** The toolchain comes from the project's own
lockfile, so a fresh checkout gets the same Go, the same Postgres, the same
everything, without depending on what you happen to have installed.

**Every branch gets its own services.** Eight checkouts can each run their own
Postgres on 5432, at the same time, without editing the committed config or
juggling port numbers.

> **This is not a security boundary.** It catches mistakes and keeps projects
> out of each other's way. It will not stop code that is deliberately trying to
> break out, and it protects less on macOS than on Linux. An unfamiliar
> repository, a stranger's pull request, or an agent turned loose on code nobody
> has read belongs in a container or a VM —
> [docs/threat-model.md](docs/threat-model.md) explains where the line is.

## Install

```sh
git clone https://github.com/dragosv/devcroft && cd devcroft
cargo install --path .
```

You need Rust 1.85+ to build it (only until there's a crates.io release), and
one of **flox**, **nix** or **devbox** on the machine to use it. devcroft does
not install packages — it sandboxes an environment one of those produces. If
you have no preference, use flox; use nix if the project already has a
`flake.nix`.

Be aware of the real price: if your project already has a flake, a flox
environment or a `devbox.json`, devcroft costs you one small config file. If it
has none of them, the actual decision in front of you is adopting Nix, and
devcroft is the smaller half of that.

`devcroft doctor` tells you what this machine is missing.

## Try it

Every sample in this repo is a working project:

```console
$ cd samples/nix-probe-sample
$ devcroft up
devcroft: bringing up sandbox 'nix-probe-sample'...
devcroft: sandbox 'nix-probe-sample' is started.

$ devcroft exec -- go run .
hello from inside
```

That's the whole loop. `up` gets the environment ready and starts the sandbox,
`exec` runs something inside it, `down` stops it, `ps` lists what's running.

In your own project you need an environment file one of the three tools
understands, and then:

```sh
cd my-project
flox init && flox install go     # or: nix flake init / devbox init
devcroft init
devcroft up
devcroft exec -- go version
```

The first `up` in a project takes as long as the environment takes to build —
minutes, possibly. Every one after that is about a fifth of a second, and `exec`
about 20 milliseconds (measured on an M4 Pro, best of five).

One thing that will bite you: since nothing outside the project folder is
writable, tools that keep their cache in your home directory need redirecting.
Go wants `GOCACHE` and `GOTMPDIR`, Rust wants `CARGO_HOME`. Each sample shows
how for its language.

## Watch it refuse something

Don't take the claim on faith. [samples/nix-probe-sample](samples/nix-probe-sample/)
is a small program that tries three things outside the project folder:

```console
$ cd samples/nix-probe-sample
$ touch ~/devcroft.tmp                      # the file it will try to delete
$ devcroft exec -- go run . probe "$HOME"
open /home/you/.ssh/known_hosts: permission denied
open /etc/devcroft-probe: permission denied
remove /home/you/devcroft.tmp: permission denied

$ ls ~/devcroft.tmp                         # still there
/home/you/devcroft.tmp
```

Nothing asked politely and nothing cooperated. The sample's README runs the same
program *without* devcroft, where it deletes the file — a refusal only proves
something if the same operation succeeds without the sandbox.

**On macOS, only the third line is reliable.** Reads and writes are refused, but
a program naming a full path to a host tool still gets it.
[docs/known-gaps.md](docs/known-gaps.md) has the details.

## Branches, ports and services

If your project declares services or ports, each sandbox gets its own private
set of them. Eight branches, eight Postgres instances, all on 5432, none aware
of the others — no allocation, no config edits.

**Git worktrees need one thing from you today.** A sandbox is identified by the
name in `devcroft.toml`, and that file is committed — so every worktree carries
the same name and ends up sharing one sandbox, with the second `up` serving the
first one's code. Give each worktree its own name until this is fixed;
[docs/known-gaps.md](docs/known-gaps.md) has the measurement.

The catch with private ports is that they're private: your dev server is no
longer on your own `localhost`. You reach it through a tunnel —

```sh
ssh -L 3000:127.0.0.1:3000 -N my-project.devcroft   # then open localhost:3000
```

— which is honest but not where this should end up; devcroft handling the
mapping itself is planned, not built.

Services declared in your environment start with the sandbox, show up in
`devcroft status`, and stop with it. **Note where they're declared:** in the
environment's own file, not in a `compose.yaml`. They run as ordinary processes
from your lockfile — Postgres from nixpkgs, not the `postgres:16` image — so a
compose-based project has service definitions to port over once. Running
containers *inside* a sandbox is refused on purpose, not missing:
[docs/decisions.md](docs/decisions.md) explains why.

## Your editor

```sh
devcroft ssh-config --write     # one time
```

Each sandbox then answers at `<name>.devcroft`, so VS Code and Cursor open it
with Remote-SSH, and `ssh`, `scp`, `rsync` and port forwarding all work. Nothing
is exposed on a network port — it goes over a local socket only your user can
open. [docs/ssh-validation.md](docs/ssh-validation.md) has the per-editor detail.

## The config file

`devcroft init` writes it. Everything outside the project folder stays blocked
until you name it:

```toml
[sandbox]
name = "my-project"

[env]
provider = "flox"          # or "nix", "devbox"

[filesystem]
read = ["/tmp"]            # read-only, and only if something needs it

[network]
default = "deny"
allow = ["api.example.com", "index.crates.io"]
ports = [5432]
```

Two commands answer "why did that fail?" — `devcroft policy --render` prints
every rule in force and where it came from, and `devcroft why` answers for one
path or one host:

```console
$ devcroft why --host bad.example.net
DENIED
denied by rule manifest:network.default (host bad.example.net is not in the allowlist)
```

## Commands

```console
init            write a devcroft.toml here          logs            the sandbox's log
up              start the sandbox                   ps              every sandbox on this machine
down            stop it, keep its state             policy --render every rule in force
rm              stop it and delete its state        why             is this path/host allowed?
exec -- <cmd>   run one command inside              doctor          check this machine
shell           open a shell inside                 ssh             connect over SSH
status          is it up, and since when            ssh-config      set up ~/.ssh/config
```

`devcroft --help` prints the same list with the flags.

## What works today

devcroft is `0.0.1`, written by one person, and used every day in its own
development. It is honest software with real gaps, all of them written down.

**Works now**

- Several branches of one project at once, each with its own tools, ports and services
- VS Code and Cursor over SSH, plus `ssh`, `scp` and `rsync`
- Linux and macOS

**Not yet**

- Running a coding agent *inside* a sandbox: its runtime and its logins don't reach in there, so today you sandbox the commands it runs, not the agent itself
- No undo — a sandboxed command that wrecks your working tree has still wrecked it
- No CPU or memory limits
- macOS enforces meaningfully less than Linux does

Everything else that's missing or partial is in
[docs/known-gaps.md](docs/known-gaps.md), written out rather than summarised
away. If something breaks, [open an issue](https://github.com/dragosv/devcroft/issues).

| Platform | Sandboxing | Needs |
|---|---|---|
| Linux | Landlock | kernel 5.13 or newer |
| macOS | Seatbelt | 10.5 or newer |

The sandboxing itself is [nono](https://github.com/nolabs-ai/nono), a library
from the Sigstore team; devcroft is the part that builds environments, runs
services and speaks SSH.

## More

| | |
|---|---|
| [docs/comparison.md](docs/comparison.md) | How this differs from Dev Containers, flox on its own, and agent products |
| [docs/known-gaps.md](docs/known-gaps.md) | Every gap, in full |
| [docs/threat-model.md](docs/threat-model.md) | What the sandbox does and doesn't protect against |
| [docs/decisions.md](docs/decisions.md) | "Why doesn't it support X?", answered |
| [docs/roadmap.md](docs/roadmap.md) | What's coming, and why in that order |
| [samples/](samples/) | Working example projects |

Contributing: [CLAUDE.md](CLAUDE.md) has the architecture rules,
[docs/implementation-log.md](docs/implementation-log.md) the build history.

## License

[Apache-2.0](LICENSE-APACHE). Dependency licenses are in
[THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).
