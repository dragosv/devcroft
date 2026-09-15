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

> **For repositories you already trust, to contain accidents.** devcroft is not
> a security boundary against hostile code, hostile prompts, or a compromised
> agent: the workload shares the host kernel. Put an unfamiliar repository, a
> stranger's pull request, or code that is trying to escape in a container or a
> VM — [docs/threat-model.md](docs/threat-model.md) says where the line is.

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
cargo install --path .
```

Requires Rust 1.95+ (edition 2024) and one of **flox**, **nix**, **devbox** or
**devenv** on the machine — devcroft does not manage packages, it sandboxes an
environment one of those produces. On a Mac, a SwiftPM project can use the
installed Xcode or Command Line Tools instead (see [Environments](#environments)).
Not on crates.io yet; see [Status](#status).

`devcroft doctor` reports what this machine has and what it lacks, provider and
enforcement both, and names the fix for each.

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
provider = "flox"          # or "nix", "devbox", "devenv" — "swift" on macOS
forward = ["GH_TOKEN"]     # the sandbox does not inherit your shell

[env.vars]
APP_ENV = "development"    # literal values that belong in the committed file

[filesystem]
read = ["/opt/reference-data"]   # read-only, and only if something needs it

[network]
default = "deny"
allow = ["api.example.com", "index.crates.io"]
ports = [5432]

[ssh]
forward_agent = false      # off unless the project says otherwise

[hooks]
post_create = "./scripts/bootstrap"      # run inside, once, after the first up
post_start = "./scripts/check-state"     # run inside, on every start
```

The project directory is writable; everything else is denied until named. Each
sandbox gets its own writable `HOME` inside the project's artifact directory, so
tools that write to the home directory work without reaching yours. Hooks run
*inside* the boundary, after restriction, with no provisioning privileges — a
hook that needs the network needs a `network.allow` entry like anything else.

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

**Your shell is not part of the environment.** A sandbox gets what the provider's
activation resolved, plus what the manifest names — and nothing else. That is the
point: the baseline denies `~/.aws` and `~/.ssh`, so a sandbox holding those same
credentials as environment variables because your shell exported them would be
undoing its own policy.

So a project may need one `forward` line. The failure mode is a tool that worked
yesterday not finding a variable, and it surfaces inside the sandbox, far from
its cause — ask directly:

```console
$ devcroft why --env GH_TOKEN
ABSENT
it is set in your shell and the sandbox does not inherit your shell
  add it: [env] forward = ["GH_TOKEN"]
```

Use `forward` for values that live on your host, and `[env.vars]` for values that
belong in the committed manifest — which is why a name in both is refused rather
than resolved by precedence.

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

**Git worktrees.** A committed manifest carries the same `sandbox.name` into
every checkout. devcroft binds a sandbox's state to the canonical project root
and refuses to adopt state created for a different checkout, so two worktrees
cannot silently share one sandbox — give each its own name at `up`, and the
committed file is untouched:

```sh
devcroft up --name feature-auth      # in worktree A
devcroft up --name feature-search    # in worktree B
```

The override follows the sandbox everywhere: state, `status`, `logs`, and its
`<name>.devcroft` SSH host.

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
user can open. Editors have real preconditions rather than a blanket "works":
VS Code needs its server directory redirected into the project and its dynamic
listener allowed; Cursor is validated manually; Zed connects and transfers its
server but its remote daemon does not yet start. Commands, versions and
evidence, negative results included, are in
[docs/ssh-validation.md](docs/ssh-validation.md).

## Commands

The surface is closed for 0.0.x — this is all of it, and `devcroft --help`
prints the same thing:

```console
sandboxes
  init [--force]              write a devcroft.toml for this project
  up [name] [--name <n>]      build the environment, apply the policy, start the sandbox
      [--recreate]              (--name gives this project root its own sandbox)
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
  why --env <NAME>            why a variable is or is not in the sandbox
  doctor                      check this host for what devcroft needs

ssh
  ssh [name]                  connect over the sandbox's own SSH server
  ssh-config [--write]        emit (or install) the ~/.ssh/config block
  proxy <name>.devcroft       ProxyCommand handler; not typed directly
```

## Environments

Provider resolution happens once, at `up`. The environment it produces — the
variables, the unsets, the read-only grants for the toolchain — is recorded and
injected into the keeper; sessions inherit it and never re-activate anything.
Editing the provider's manifest or lockfile marks the sandbox stale, and
`up --recreate` resolves again.

Four providers build a **closure** — a complete, self-contained package set, so
what runs inside doesn't depend on what you happen to have installed — and all
four share one content-addressed Nix store, so eight sandboxes of one project
cost one build (measured, at eight). The provider's own activation hook is
project code, and none of them runs it on your host:

| Provider | Project input | How the environment is captured without running the hook |
|---|---|---|
| **flox** | `.flox/` manifest and lock | a derived, hook-free copy of the environment is materialized; the hook runs *inside* the sandbox |
| **nix** | `flake.nix` and `flake.lock` | `print-dev-env --json`, where `shellHook` arrives as inert data |
| **devbox** | `devbox.json` and lock | `shellenv --pure`, which never runs `init_hook` |
| **devenv** | `devenv.nix` and lock | `build shell` for the environment and `eval enterShell` for the hook, separately; the hook runs inside |

There is no `host` or `none` provider, on purpose: a sandbox whose tools come
from whatever happens to be installed would not be reproducible.

**swift** is the fifth, and the one exception: an **artifact** tier, macOS only,
for projects that need Xcode, Apple SDKs, code signing or Darwin itself — the
reason devcroft runs natively at all. It resolves the toolchain `xcode-select`
names as data (`swift -print-target-info`, `xcrun`) and never evaluates
`Package.swift`, which is a program; dependency resolution and the build happen
inside the sandbox, through a twelve-line `swift` shim the provider writes and
`up` names. A plain `swift build` works with the project root as the manifest's
only grant; everything the toolchain reads from the host — the Xcode bundle,
the SDK, Foundation's data, the host userland — appears in `policy --render`
with a `provider:swift` origin, and `doctor` checks the toolchain and the Xcode
license before `up`. The cost is stated plainly: no shared store, so eight Swift
sandboxes cost eight builds, and what builds depends on that Mac's Xcode. A
portable Swift package draws a warning naming nix or flox instead, which
`[env] require_native_apple_evidence = true` turns into a refusal.

Long-lived **services** — databases, dev servers — are declared in the
provider's own manifest and supervised by the sandbox's keeper, so parallel
sandboxes get their own instances. Supported for flox, devenv and devbox (whose
services come from its plugins, a deliberate call recorded in
[docs/decisions.md](docs/decisions.md)); nix has no service concept. A service
that declares a readiness probe is reported ready only once it passes, and a
dependent waits for that. A provider field devcroft cannot carry is refused by
name, never dropped.

**mise, pixi and hermit are a different answer, and not "they failed the
test".** mise passes devcroft's six-criterion provider test, and the shape an
implementation would take is written down. What stops it is structural: those
tools link against whichever libc a host has, which makes them artifact tier —
the weaker guarantee swift ships with — and it is gated on demonstrated demand
rather than shipped on spec. [docs/decisions.md](docs/decisions.md) has the test
and an entry per answer.

## Using it with an agent today

The boundary is the sandbox's process tree, not automatic interception of every
command an editor-side agent runs. Keep the agent on the host and route the
consequential commands through devcroft:

```sh
devcroft up --name review-42
devcroft exec review-42 -- cargo test
devcroft exec review-42 -- ./scripts/run-migration.sh
```

An agent can also be installed in the project environment and run inside, but
its host logins do not follow it in, and there is no credential broker yet: a
secret you `forward` is visible to every process in the sandbox. Before
unattended work: commit, or use a disposable worktree — project writes are real
and there is no undo; read `devcroft policy --render`; keep grants narrow; and
treat every allowed network destination as an outbound data channel.

## How it compares

Dev Containers is the closest thing in wide use, and the difference is a trade
rather than a win:

| | devcroft | Dev Containers |
|---|---|---|
| Workload runs on | Your host OS — on a Mac, macOS | The container's Linux |
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
from inside, so a kernel exploit escapes. For a real boundary, run devcroft inside
a VM; that is the supported answer, and already how the macOS path works.
[docs/threat-model.md](docs/threat-model.md) says which use case each one backs.

**What devcroft actually enforces is declared data, not summarised here** —
run `devcroft doctor` for the full capability matrix against your own host, or
see [docs/known-gaps.md](docs/known-gaps.md) for the write-up behind each
still-open entry. Treat any capability claim elsewhere on this page as a
pointer to that matrix, not a restatement of it — if the two ever disagree, the
matrix is right.

**Measured, with tests or recorded live runs:** flox, nix, devbox and devenv
closures; the swift toolchain on Xcode and Command Line Tools; a clean sandbox
environment with a project-local home; deterministic policy rendering and `why`;
Linux mount and network namespaces; authenticated, domain-filtered egress
through a per-sandbox proxy; flox, devenv and devbox services with readiness;
worktree identity; SSH command and pty sessions, `sftp`, `scp` and port
forwarding; macOS Seatbelt, with its differences reported per capability.

**Not claims today:** containment of hostile code or kernel exploits; undo of
project changes; CPU, memory or PID budgets; a multi-agent supervisor; a
credential broker that keeps secrets out of the sandbox; running a project with
no environment file; Linux-equivalent port isolation on macOS.

| Platform | Mechanism | Minimum version | Worth knowing |
|----------|-----------|-----------------|---------------|
| Linux | Landlock, plus user, mount and network namespaces | Kernel 5.13+; namespaces need to be creatable unprivileged | Ubuntu 24.04 restricts unprivileged user namespaces by default and `up` refuses until `kernel.apparmor_restrict_unprivileged_userns=0` — an opt-in fallback is specified, not built |
| macOS | Seatbelt | 10.5+ | No mount or network namespace, so services collide on ports and visibility is not narrowed; execution, reads, writes and egress are enforced |

Same floor as [nono](https://github.com/nolabs-ai/nono), the sandboxing library
devcroft is built on. Verified end to end against real tooling in this repo's own
Linux devcontainer and on a macOS 15 host; where the two platforms differ,
[docs/known-gaps.md](docs/known-gaps.md) says how, with the measurement.

## Ready to go deep?

| | |
|---|---|
| [docs/comparison.md](docs/comparison.md) | Dev Containers, `nono-cli`, flox alone, and today's coding-agent products |
| [docs/known-gaps.md](docs/known-gaps.md) | Every published gap, in full |
| [docs/threat-model.md](docs/threat-model.md) | Which use case the isolation backs, and which it doesn't |
| [docs/roadmap.md](docs/roadmap.md) | What 0.2 through 1.0 each have to be true for, and why in that order |
| [docs/decisions.md](docs/decisions.md) | Every "why doesn't devcroft support X", answered falsifiably |
| [docs/ssh-validation.md](docs/ssh-validation.md) | SSH client and editor matrix — OpenSSH, rsync, VS Code, Cursor, Zed |
| [samples/](samples/) | Verified sample projects, one per environment provider, plus the boundary probe |

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

[Apache-2.0](LICENSE-APACHE), matching `nono` and the sigstore crates devcroft
links, rather than the Rust-conventional `MIT OR Apache-2.0` — a dual license
would let a user take MIT and no patent grant while changing none of their real
obligations. Dependency license texts are in
[THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).
