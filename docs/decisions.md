# Decisions: what devcroft does not do, and why

This document is the reference for every "why doesn't devcroft support X"
question. It is deliberately written to be falsifiable: each rejection
states the specific property that fails, not a preference. If a rejection's
stated reason stops being true — upstream changes, new mechanism — the
decision should be revisited, not defended.

Three categories:

- **Rejected by design** — supporting it would contradict what devcroft is.
- **Covered differently** — looks missing, is actually solved elsewhere.
- **Known gaps** — containers are genuinely better here today.

---

## 1. Environment providers

### The qualification test

A provider qualifies if it satisfies all six:

1. **Declarative manifest** — the environment is defined in a file, not
   accumulated by imperative commands.
2. **Restorable lockfile** — exact versions can be *reinstalled* from it,
   not merely recorded.
3. **Immutable-capable shared store** — packages can be materialized once
   and exposed read-only to many sandboxes without a write grant.
4. **Capturable activation** — the resolved environment can be extracted
   host-side at `up` (env-diff or equivalent) **without executing
   project code**. The second half was always what this meant and was
   never written down, which is how two shipped providers came to
   violate it unnoticed (`fix-provisioning-hooks`).

   Provisioning runs before any restriction exists, with the invoking
   user's own network and filesystem access. A provider's activation
   hook — flox's `[hook].on-activate`, a nix devShell's `shellHook`,
   devbox's `shell.init_hook` — is project code, so running it during
   provisioning voids the reason that phase is trusted at all.

   Measured, and the pattern is consistent: every provider offers a
   "run a command inside the activated shell" entry point, and every
   one of those runs the hook. Most also offer a "hand me the
   environment" entry point that does not.

   | provider | runs the hook | does not |
   |---|---|---|
   | nix | `nix develop --command` | `nix print-dev-env --json` |
   | devbox | `devbox run` | `devbox shellenv` |
   | flox | `flox activate -- <cmd>` | *none found* |

   A provider with no hook-free entry point does not fail this
   criterion outright — flox is devcroft's default and `on-activate` is
   idiomatic — but the gap must be **detected and reported at `up`**,
   never left silent. Note the near-miss: plain `nix print-dev-env`
   looks like a fix and is not, since the script it emits ends with
   `eval "${shellHook:-}"`.
5. **Completeness** — the provider can describe the whole environment, not
   one ecosystem's slice of it. A provider that covers language runtimes
   but leaves the C toolchain, linker, or system libraries to the host has
   smuggled `host` passthrough back in under another name.
6. **Verifiable preconditions** — whatever the provider requires in order
   to deliver its tier must be checkable at `up`, cheaply. A provider
   whose requirements can only be discovered mid-build produces failures
   the user will attribute to devcroft.

Providers are further sorted into guarantee tiers:

- **closure** — identical behavior, transitive (Nix-based: flox, devbox,
  nix flakes, devenv). The store hashes the entire dependency graph
  including libc.
- **artifact** — identical downloaded artifacts, host-linked runtime
  (mise, pixi, hermit). Integrity of what was fetched is guaranteed;
  behavior still depends on host libraries.

The tier is always visible in `status` and once at `up`. devcroft does not
market two different guarantees under one word.

**An artifact-tier provider must declare its host library grants**
(`own-policy-baseline`, implemented). devcroft's baseline grants no host
library paths — measured against the finished implementation: a full
Rust build from a flox closure (`samples/flox-clap-sample`) needs only
the project root, `/tmp`, and `/nix/store` (via the provider's own
grant), and a full Go build from a nix closure
(`samples/nix-go-sample`) needs the same two beyond its own store root;
neither needs anything from `/lib`, `/usr/lib`, or `/usr/bin`, and both
verified live that the corresponding host toolchain (`/usr/bin/gcc`,
`/usr/bin/git`) is denied. A provider whose runtime links against host
libraries therefore cannot inherit that access; it declares the paths
it needs as provider grants, which compile with a `provider:<name>`
origin and appear in `policy --render`.

This is the point of the rule rather than a side effect: the difference
between a self-contained closure and a host-linked runtime stops being a
tier name in this document and becomes a visible difference in the
compiled policy. Two sandboxes can be compared instead of described.

It also means host passthrough returns for such providers, which is what
the `host`/`none` entry below rejects. The distinction: there it is the
whole product contradicting itself with nothing declared; here it is a
named, attributed, rendered grant, governed by the existing rule that
provider resolution must not widen the policy. If that distinction
proves too fine in practice, the honest response is to reject the tier
outright, and this paragraph is where that argument reopens.

### Rejected: `host` / `none` passthrough

**Property that fails:** none — it does not even attempt reproducibility.

devcroft's identity is reproducible environments. A passthrough provider is
not a convenience, it is the product contradicting itself: the toolchain
would come from the host, so the same manifest would produce different
environments on different machines, and the shared-store density argument
would not apply.

The cases this would serve (a foreign repo, bootstrapping a project without
an environment yet) are correctly solved by `flox init`, not by a degraded
mode. `up` failing with a `flox init` hint is the right answer.

### Rejected: asdf, proto

**Property that fails:** 2 (restorable lockfile), partially.

These are mise's class without mise's lockfile maturity. mise supersedes
asdf and supports its plugin ecosystem; proto is comparable but lacks the
`locked`-mode enforcement and provenance verification that qualified mise
for the artifact tier. Adding either would mean a third-rate version of a
tier devcroft already serves.

Revisit if either ships a lockfile with pre-resolved URLs, checksums, and a
strict mode that forbids resolution at install time.

### Rejected: rustup (and single-ecosystem toolchain managers generally)

**Properties that fail:** 5 (completeness) and 6 (verifiable
preconditions).

rustup is the most tempting rejection in this document, because the
on-ramp is genuinely better than any qualifying provider: it is already
installed on every Rust developer's machine, and `rust-toolchain.toml` is
already committed in most repositories. It also passes criteria 1-4
cleanly — pinned channels are restorable, `~/.rustup/toolchains/<version>`
directories are additive and never modified in place (so the
devcroft-owned, append-only store model applies), and activation is just
a PATH entry.

It fails on what it deliberately does not cover:

- **rustup never produces a complete build environment.** On Linux, rustc
  invokes `cc` to link, and needs libc plus its headers. Leaving the C
  toolchain to the host is rustup's design decision, not an oversight. A
  rustup provider would therefore require every sandbox to read the host's
  C toolchain — `host` passthrough wearing a different hat, with
  reproducibility depending on whichever gcc each machine happens to have.
- **The escape hatch does not hold.** A musl target with self-contained
  linking (rustup ships musl's `libc.a` in the target) plus `rust-lld` can
  build without any host `cc`. But proc macros compile for the *host*
  target, and `serde_derive`, `thiserror`, and `tokio-macros` are proc
  macros — so nearly every real project needs the host toolchain anyway.
- **The precondition is unverifiable, which is the decisive point.**
  Compare with mise: "a `mise.lock` covering this platform exists" is
  checked in milliseconds at `up`, and failure is immediate and legible.
  The rustup equivalent is "this project will never need a C toolchain",
  which cannot be determined without resolving the full dependency tree
  and inspecting every `build.rs` — and not reliably even then. The
  failure would surface eight minutes into a build as `linker cc not
  found` buried in rustc output, and users would blame devcroft. A mode
  that works until it abruptly does not, with no warning at `up`, is worse
  than a mode that does not exist.

**What Rust users should get instead:** `devcroft init` detects
`rust-toolchain.toml` and generates a flox manifest honoring the pinned
channel (via fenix or rust-overlay) alongside a C toolchain. The familiar
file stays; the closure guarantee is preserved; nothing extra is written
by hand.

The same reasoning applies to any single-ecosystem toolchain manager —
nvm, pyenv, rbenv, sdkman, ghcup. mise qualifies where they do not
precisely because it spans ecosystems and can deliver utilities too.

### Rejected: Homebrew

**Properties that fail:** 2, 3, and the per-project environment concept.

Homebrew looks closer than it is — `Brewfile` is declarative and
`Brewfile.lock.json` exists — so the reasoning has to be precise:

- **The lockfile is descriptive, not prescriptive.** It records what was
  installed; it cannot reinstall those versions. Formulae track latest,
  versioned formulae are major-granular (`node@20`), and pinning
  homebrew/core to a git commit is unsupported and fragile. A lockfile you
  cannot restore from is a journal.
- **There is no per-project environment.** Everything installs into one
  global prefix with symlinks in `bin/`. Every sandbox would see the same
  package set; "project A's environment" cannot exist, so there is nothing
  to compile into a per-sandbox policy.
- **Bottles are prefix-hardcoded.** The devcroft-owned-store trick that
  rescued mise does not work: a custom `HOMEBREW_PREFIX` means building
  nearly everything from source, turning `up` into an hours-long build.
- **`brew upgrade` mutates globally**, removing old Cellar versions and
  moving symlinks under running sandboxes.

**What Homebrew users actually want** is usually different: their host
`git`, `ripgrep`, `fd` visible inside the sandbox. That is a filesystem
policy concern, not a provider concern:

```toml
[filesystem]
read = ["/opt/homebrew"]
```

Explicit, opt-in, and it does not pretend the environment is reproducible.
Host tools stay a host matter; the project environment comes from the
provider.

### Rejected: apt, raw conda/mamba

**Properties that fail:** 2 and 3.

Globally mutable stores by design, no restorable lockfile. Note that
**pixi** (the conda ecosystem's declarative layer) does qualify for the
artifact tier — `pixi.lock` is complete with per-package sha256 and the
package cache is content-addressed. The rejection is of raw conda usage,
not of the conda ecosystem.

---

## 2. Dev Container features

### Rejected: `initializeCommand` (host-side lifecycle hook)

**Why:** it runs project-defined code on the host, outside any boundary,
before the environment exists.

This is the one devcontainer feature devcroft deliberately refuses to
match. A repository you just cloned should not be able to execute arbitrary
commands on your machine because you opened it. devcontainers accept this
because the host is where the container gets built; devcroft has no such
need — provisioning runs pinned tooling from a lockfile, never project
code.

The trust line in devcroft is not "before or after `up`". It is **pinned
tooling from a lockfile** versus **project code**. Provisioning is the
first. Hooks and sessions are the second, even though hooks are declared in
the same manifest.

If a project needs setup work, `hooks.post_create` runs it inside the
boundary under full policy. If that setup needs the network, the manifest
must say so in `network.allow`. A hook that fails because the policy is too
narrow is the system working, not a bug.

### Rejected: `postAttachCommand` (per-session hook)

**Why:** noise without a use case that shell rc files do not already cover.

Every new session running a command is surprising behavior in a tool whose
sessions are cheap and numerous — with a fleet of agents, a per-attach hook
multiplies invisibly. Shell startup files inside the environment do this
already, under the user's own control.

### Rejected: Docker-in-Docker

**Why:** it is a boundary hole, not a feature.

Exposing the host Docker socket inside a sandbox grants root-equivalent
control of the host — the sandbox is bypassed entirely, not weakened. This
is denied by baseline policy and is not configurable.

Note the irony worth stating in the FAQ: devcontainers need DinD *because*
the dev environment is itself a container. devcroft's environments are not
containers, so a project that needs to run containers can talk to the host
runtime the ordinary way, outside the sandbox, as an explicit choice by the
user rather than a hidden grant.

### Rejected: `privileged`, `runArgs`, `capAdd` equivalents

**Why:** escape hatches that let the manifest widen the boundary defeat the
model.

devcroft has exactly one place where capabilities are declared — the
manifest's `filesystem` and `network` sections — and one place where they
are enforced. There is no second channel that passes raw flags to the
backend. If a legitimate need cannot be expressed in the policy model, the
policy model should grow a reviewed, named capability; it should not grow
an arbitrary passthrough.

---

## 3. Covered differently (not gaps)

### Features registry → the provider's package set

devcontainer Features are a composable registry of installation scripts
distributed as OCI artifacts. devcroft does not reimplement this, because
nixpkgs is already a larger registry with stronger guarantees. Adding a
"feature" is `flox install <pkg>`.

The correct framing in documentation is not "devcroft has no Features" but
"Features are `flox install`".

### Prebuilds → binary caches

devcontainer prebuilds warm an image in CI. The equivalent is a Nix binary
cache or substituter, plus running `devcroft up` in CI to populate it.
This needs documentation, not code.

### `forwardPorts` → SSH

Editors connected over SSH detect and forward listening ports themselves.
devcroft supports `direct-tcpip` (`-L`) gated by policy. VS Code-specific
metadata like `portsAttributes` is the editor's concern, not devcroft's.

### `customizations.vscode.extensions` → editor settings

`remote.SSH.defaultExtensions` in VS Code settings installs a set of
extensions on every SSH host, including all devcroft sandboxes. This
belongs in the user's editor config, not in a per-project manifest.

### `remoteUser`, UID/GID mapping → not applicable

Sessions run as the actual user on the actual filesystem. The entire class
of bind-mount ownership problems does not exist. This is an architectural
advantage, not an unimplemented feature.

### GPU access → not applicable

No virtualization layer sits between the session and the device. `/dev`
access is native, subject to filesystem policy.

---

## 4. Known gaps (containers are better here)

State these plainly in the README. Under-promising is cheaper than
retracting a claim after someone demonstrates a bypass.

### Security guarantee depends on the isolation tier

devcroft exposes two isolation tiers. The tier is derived from the backend,
shown in `status` and once at `up`, and determines what may honestly be
claimed:

- **`process`** (default; nono with Landlock on Linux, Seatbelt on macOS).
  Protects against accidents, careless commands, and simple exfiltration.
  It is **not** a defense against a determined attacker who controls the
  code running inside: the entire host kernel syscall surface remains
  reachable, so a kernel bug is an escape.
- **`hardened`** (planned; Linux only — see `add-hardened-tier`). Two
  candidate backends, both paired with Landlock as additive defense in
  depth: gVisor, whose Sentry implements syscalls in user space so the
  attack surface becomes Sentry rather than the host kernel; and LiteBox,
  a Rust library OS that links OS services into the workload and thereby
  avoids syscall traps in many cases. Both are a real security boundary,
  at a real performance cost — builds are syscall-heavy, and that is
  where the cost lands.

Two rules follow, and they are not negotiable:

1. Claims are always tier-qualified. devcroft never says "sandboxed"
   without saying which tier, and never lets `process`-tier docs borrow
   `hardened`-tier language.
2. Backends document their own limits (nono and MXC both do); devcroft
   inherits those limits rather than optimistic summaries of them.

Under-promising is cheaper than retracting a claim after someone
demonstrates a bypass.

### Rejected (for now): non-rootless gVisor for netstack

gVisor's per-sandbox netstack (`--network=sandbox`) would let a loopback
bind inside one sandbox stay invisible to every other sandbox and the
host — closing the listen-socket/port-conflict gap the process tier
already has (see `docs/ssh-validation.md`) at the `hardened` tier too.
It requires giving up rootless mode: `runsc` rejects `--network=sandbox`
combined with `--rootless` outright, and devcroft runs unprivileged
everywhere by design (Landlock itself needs no privilege; nono drops
root before exec). The property that fails: an unprivileged host process
cannot get gVisor's own network isolation, full stop — this is not a
devcroft configuration gap to work around, it is `runsc`'s own
documented behavior, confirmed by an earlier draft of `add-gvisor-backend`
having assumed otherwise and having to be corrected before any code
shipped against it.

`add-gvisor-backend`'s hardened tier therefore shares the host's network
namespace (`--network=host` when the manifest grants egress, `--network=
none` otherwise) — the tier's real, delivered guarantee is Sentry's own
user-space syscall boundary, not a network story stronger than the
process tier's. `[network]`'s domain-level allowlist is not enforced at
this tier at all (nothing threads it into anything gVisor-facing); a
manifest that grants egress gets unfiltered host network access under
`hardened`, a real known gap, not a silently-dropped enforcement.

**Corrected, verified live (add-flox-services task 6.5):** an earlier
version of this entry also claimed "Landlock defense-in-depth on Sentry
itself" as part of the delivered guarantee. That layer existed in code
(`src/gvisor/runner.rs` wrapped `runsc run` in a Landlock ruleset) but
was never exercised against a real unprivileged user namespace until
this devcontainer could finally run one — and it turned out to make
`--rootless` bootstrap fail unconditionally: `runsc run`'s own chroot
setup issues a `mount()` call to change mount propagation, which returns
`EPERM` under *any* active Landlock ruleset regardless of what it grants
(confirmed by elimination, including a maximally permissive one).
Landlock cannot mediate `mount()` in any current ABI, so this was not a
grant to widen. Removed rather than narrowed — see
`src/gvisor/runner.rs`'s module doc for the full evidence trail. The
tier's actual boundary was always Sentry's own seccomp/ptrace
confinement; the Landlock layer never added anything that worked.

**Revisit if:** a future backend change is willing to trade rootless for
netstack behind an explicit, narrowly scoped privilege grant — the same
shape as the NOPASSWD sudo rule this repo already gives flox's
nix-daemon for the one root action it actually needs, rather than a
blanket privileged container. Recorded as a real option in
`add-gvisor-backend`'s own Open Questions, not chosen there either.

### No resource limits (yet)

Landlock and Seatbelt constrain access, not consumption. A session can
exhaust host CPU or memory. With a fleet of agents this matters — one
runaway build can take down the host. Containers get cgroups for free.

Planned mitigation: cgroup v2 scope units per keeper on Linux. macOS has no
comparable mechanism, so the gap there is likely permanent.

**Revisit at the hardened tier:** `runsc` integrates with cgroups
directly (gVisor's Sentry already accounts resource use per sandbox for
its own scheduling), which the process tier has no equivalent of. MVP
explicitly punted on resource limits everywhere, and `add-gvisor-backend`
does not add them either — building them only for the tier that happens
to make it easy would be scope creep for a change that is not about
resource limits. But the door this opens is real, not hypothetical, and
worth a dedicated change once the hardened tier itself ships.

### No inter-sandbox process isolation (MVP)

Landlock does not hide processes: no PID or mount namespace separates
sandboxes, so this remains structurally true. Planned mitigation: PID and
mount namespaces layered over Landlock on Linux (still container-free, no
images involved). macOS has no namespace equivalent, so fleet-grade
separation there is Linux-only.

**Corrected, verified live (`tests/process_tier_landlock_boundaries.rs`):**
"can see and signal each other's process trees" turned out to overclaim
what actually reaches through the shared namespace. On a Landlock **ABI
V6** host, a sandboxed process can do neither: V6's signal-scoping LSM
hook blocks `kill()` against a process outside the sandbox (`Operation not
permitted`), and the pre-existing default-deny filesystem policy already
covers `/proc/<pid>/*` like any other ungranted path (`Permission
denied`) — closing both without any namespace doing the enforcing. This
is kernel-version-dependent: ABI V6's signal scoping is new enough that
older kernels this project still supports would plausibly reproduce the
original claim. `doctor`'s `kernel: Landlock V6` line is how to tell
which regime a given host is in — this decision entry was wrong to state
the gap unconditionally rather than naming that dependency.

### Cooperative network filtering

Domain-level allowlists on process-level backends require a proxy the
sandboxed process cooperates with. A process that deliberately bypasses the
proxy — raw sockets, direct IPs — is not stopped by this mechanism on all
platforms. Where an aspect cannot be enforced on the current host, `up`
says so once, and `doctor` lists it. Nothing is silently dropped.

**Corrected, verified live (`tests/process_tier_landlock_boundaries.rs`):**
"raw sockets, direct IPs — not stopped" doesn't hold on Linux as stated.
Tested directly: with `network.default = "deny"` (allowlist or not),
`policy --render` still shows `network.block: true`, and a raw socket
connecting to an IP with no relation to any allowed domain gets a
kernel-level `Permission denied` — nono's own Landlock network scoping,
not a proxy hint a raw socket simply never talks to. macOS Seatbelt is a
different backend and this correction doesn't extend to it — the original
claim stands there. Left genuinely open, not claimed as safe: whether the
*allowed* domain's own resolved-IP scope is wider than intended (a
different service on the same allowed IP, or DNS-rebinding-shaped
tricks) — untested, and a real candidate for the next thing to check
before trusting this further.

### No service sidecars (yet)

devcontainers compose with Docker Compose for databases and other services.
Planned mitigation: provider-native service support (flox `[services]`,
devenv) supervised by the keeper. Until then, run services on the host or
in containers alongside, and grant network access explicitly.

### Keeper is a single point of failure per sandbox

The keeper is the one resident process a sandbox's control socket, SSH
socket, and every live session route through (Decision 1). If it dies — an
unhandled panic, `SIGKILL`, an OOM kill — every session inside that sandbox
dies with it; there is no live failover to a standby process the way an
orchestrator might reroute traffic to a replacement container. `up`'s
health check (`lifecycle::state::health`) detects a dead keeper on the next
command and recovers by starting a fresh one (`UpOutcome::Recovered`), but
any process state the old keeper held — running builds, shells, servers —
is gone, not migrated. This is the same trade a devcontainer restart makes,
disclosed rather than silent: acceptable for a dev sandbox, not a model
for a fleet that needs live failover across many keepers without losing
in-flight work.
