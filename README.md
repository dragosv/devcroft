# devcroft

**Isolated, reproducible development environments in seconds, with no daemon,
no image build, no container, and no VM.** Each one gets its toolchain from a
lockfile, a boundary the kernel enforces, its own network namespace, and a real
SSH server your editor connects to like any remote machine.

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE-APACHE)
[![Rust](https://img.shields.io/badge/Rust-edition%202024-orange.svg)](Cargo.toml)
[![Status](https://img.shields.io/badge/release-0.0.1-yellow.svg)](#status)

Built for running several coding agents, or several branches of one project, on
a single machine at the same time.

---

## The problem

Git worktrees give each agent its own directory, not its own *environment*. They
still share your installed toolchain, one `target/`, and — the part with no good
workaround — your services and their ports.

Your `compose.yaml` is committed, so every branch declares Postgres on 5432.
Start two, and the second fails. The usual escape is one shared database with a
schema per branch, which means a destructive migration from one agent takes out
everybody. A container per branch really does fix it, and costs enough that
people run four and stop.

devcroft gives each sandbox **its own network namespace**. The same committed
port in every sandbox, no collision, no allocation, nothing for the service to
cooperate with — and outbound access still works inside it, so an agent gets both
its own Postgres and the registries it needs.

## Install

```sh
git clone https://github.com/dragosv/devcroft && cd devcroft
cargo build --release
```

Requires Rust stable (edition 2024) and one of `flox`, `nix`, or `devbox` —
devcroft does not manage packages, it sandboxes an environment one of those
produces. Not on crates.io yet; see [Status](#status).

## Run it!

devcroft needs a project that already has an environment. From an empty
directory:

```console
$ flox init
✔ Created environment 'demo-project' (aarch64-linux)

$ flox install bash coreutils ripgrep
✔ 'bash', 'coreutils', 'ripgrep' installed to environment 'demo-project'

$ devcroft init
devcroft: wrote /home/you/demo-project/devcroft.toml
devcroft: found an existing flox environment (.flox/); ready for `devcroft up`.

$ devcroft up
devcroft: bringing up sandbox 'demo-project'...
devcroft: sandbox 'demo-project' is started.

$ devcroft exec -- bash -c 'echo hello from inside; pwd'
hello from inside
/home/you/demo-project
```

That's it. Commands now run against the packages in that lockfile, with
read/write access to the project directory and **nothing else** — your SSH keys,
your cloud credentials, and the rest of your disk are refused by the kernel:

```console
$ devcroft exec -- cat ~/.ssh/known_hosts
cat: /home/you/.ssh/known_hosts: Permission denied

$ devcroft exec -- touch /etc/devcroft-probe
touch: cannot touch '/etc/devcroft-probe': Permission denied

$ devcroft exec -- rm -rf ~
rm: cannot remove '/home/you': Permission denied
```

Nothing was asked politely and nothing cooperated: an agent that decides to
delete your home directory gets `EPERM`, whatever it intended.

## Make it your own!

`devcroft init` writes a minimal file. Everything outside the project directory
stays denied until you name it:

```toml
[sandbox]
name = "my-project"

[env]
provider = "flox"          # or "nix", "devbox"

[filesystem]
read = ["/tmp"]            # a shared scratch dir, read-only

[network]
default = "deny"
allow = ["api.example.com", "index.crates.io"]
ports = [5432]
```

That config compiles to a profile that is deterministic and inspectable — every
rule carries the reason it exists, and nothing reaches the kernel that
`policy --render` won't show you:

```console
$ devcroft policy --render
sandbox: my-project

filesystem.allow:
  .                                        manifest:filesystem.allow
  /dev/pts                                 baseline
  /dev/null                                baseline
filesystem.read:
  /tmp                                     manifest:filesystem.read
  /lib                                     baseline
  /usr/lib                                 baseline
  [...8 more baseline loader and /dev paths, elided here...]
  /nix/store                               provider:flox
filesystem.deny:
  ~/.local/share/devcroft                  baseline
  ~/.ssh                                   baseline
  ~/.aws                                   baseline
  ~/.config/gcloud                         baseline
  ~/.kube                                  baseline

network.block: true
network.allow_domain:
  api.example.com                          manifest:network.allow
  index.crates.io                          manifest:network.allow
network.ports:
  5432                                     manifest:network.ports
network.namespace: own (declared ports are reachable inside the sandbox and via `devcroft ssh -L`, not on the host's loopback)
network.proxy: 127.0.0.1:34275 (running)

$ devcroft why --host evil.example.net
DENIED
denied by rule manifest:network.default (host evil.example.net is not in the allowlist)

$ devcroft why --path ~/.ssh/id_rsa --op read
DENIED
denied by rule baseline
```

Baseline denials always win, including over devcroft's own data directory.

## Every branch gets the same port

This is the part a container gives you and a worktree doesn't. A sandbox that
declares `network.ports` or `[services]` gets its own network namespace, so its
ports belong to it alone — eight checkouts of one project can all bind 5432, and
none of them know about the others.

The tradeoff is stated out loud at `up`, not discovered later:

```console
$ devcroft up
devcroft: bringing up sandbox 'my-project'...
devcroft: note: this sandbox has its own network namespace, so its declared port(s) 5432 are reachable from inside it and through `devcroft ssh -L <local>:127.0.0.1:<port> my-project`, but not directly on the host's own loopback
devcroft: sandbox 'my-project' is started.
```

So reach a dev server through the tunnel:

```sh
ssh -L 3000:127.0.0.1:3000 -N my-project.devcroft   # then open localhost:3000
```

A sandbox with `network.default = "allow"` isn't isolated, and its ports stay
directly reachable.

Services declared in your environment are supervised with the sandbox — devcroft
generates its own process-compose config from them, starts them in the keeper
before your hooks run, reports each one's state, and reaps them at `down`:

```console
$ devcroft status
sandbox: my-project
keeper: healthy (uptime 0s, 1 session(s))
env: fresh
isolation: process
service api: running pid=49790
policy: no degraded capabilities on this host
```

`process-compose` has to be a member of the project's environment; devcroft
refuses at `up` if it isn't, rather than coming up with services that silently
never start. A shell is *not* your job: devcroft resolves one out of the
closure itself, and `policy --render` shows the grant it added to reach it.

## Connect your editor

```sh
devcroft ssh-config --write     # adds a block to ~/.ssh/config, once
```

Each sandbox then answers at `<name>.devcroft` — open it with VS Code or Cursor
Remote-SSH, or use `ssh`, `scp`, `sftp`, `rsync` and `-L` forwarding directly.
Nothing listens on a TCP port; the connection goes over a unix socket only your
user can open. What each editor actually needs, negative results included, is
measured in [docs/ssh-validation.md](docs/ssh-validation.md).

## Commands

The surface is closed for 0.0.x — this is all of it, and `devcroft --help`
prints the same thing:

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

Three environment providers are supported — **flox**, **nix flakes**, and
**devbox**. Each builds a *closure*: a complete, self-contained package set, so
what runs inside doesn't depend on what you happen to have installed. There is no
"just use the host" fallback, on purpose. Eight sandboxes of one project cost one
build, because they share a single content-addressed store.

## How it compares

Dev Containers is the closest thing in wide use, and the difference is a trade
rather than a win:

| | devcroft | Dev Containers |
|---|---|---|
| Isolation | Kernel primitives (Landlock/Seatbelt) — accident protection, not a security boundary | A real container boundary |
| Cost per environment | Low — shared Nix store, no rootfs or guest kernel | Image layers, plus a VM on macOS |
| Reproducibility | Mandatory — config + lockfile, no host fallback | Optional — you write a Dockerfile and hope |
| Editor access | A real SSH server per sandbox | Native, through the container |

Run code you genuinely don't trust in a container or a VM. Run eight agents on
one laptop in devcroft. [docs/comparison.md](docs/comparison.md) has that in
full, plus `nono-cli`, flox alone, and how today's coding-agent products
provision environments.

## Status

**0.0.1 — working, and used daily in this repo's own development.** The number is
deliberate: `0.0.z` is the only range cargo treats as incompatible with itself,
so nothing here promises compatibility with the next version. `0.1.0` is held
back rather than skipped, until the boundary matches what this page says about it
— see [docs/roadmap.md](docs/roadmap.md).

**The boundary catches mistakes, not attacks.** The full host kernel is reachable
from inside, so a kernel exploit escapes, and **unix sockets bypass the policy
entirely** — Landlock mediates TCP, not AF_UNIX, so a sandbox can reach any unix
socket the filesystem permits. For a real boundary, run devcroft inside a VM;
that is the supported answer, and already how the macOS path works.
[docs/threat-model.md](docs/threat-model.md) says which use case each one backs.

The rest are written up rather than summarised away — no rollback, no cgroup
limits, network isolation needing unprivileged user namespaces, macOS domain
filtering unverified, no inter-sandbox process visibility separation, Zed's
remote server: [docs/known-gaps.md](docs/known-gaps.md).

| Platform | Mechanism | Minimum version |
|----------|-----------|-----------------|
| Linux | Landlock | Kernel 5.13+ |
| macOS | Seatbelt | 10.5+ |

Same floor as [nono](https://github.com/nolabs-ai/nono), the sandboxing library
devcroft is built on. Verified end to end against real tooling in this repo's own
Linux devcontainer; macOS is implemented but has no host measuring it yet.

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
including what turned out to be wrong, and
[openspec/changes/](openspec/changes/) the specs (`openspec list` shows live
progress).

## Security

The isolation tier is accident protection and is documented as such — please read
[docs/threat-model.md](docs/threat-model.md) before relying on it. This is `0.0.x`,
pre-1.0, unaudited, single-maintainer software with no security SLA — if you find
a problem, [open an issue](https://github.com/dragosv/devcroft/issues) like any
other bug. A private disclosure process is worth having once there's a userbase
for it to protect; see [docs/roadmap.md](docs/roadmap.md).

## License

## License

[Apache-2.0](LICENSE-APACHE), matching `nono` and the sigstore crates devcroft
links, rather than the Rust-conventional `MIT OR Apache-2.0` — a dual license
would let a user take MIT and no patent grant while changing none of their real
obligations. Dependency license texts are in
[THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).
