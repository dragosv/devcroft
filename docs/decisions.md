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

### Decided: the guarantee is the closure tier; the seam stays generic

The question "should devcroft restrict itself to nix-based providers and
optimize for that?" hides two questions with different answers, and this
entry records both so the argument does not have to be re-derived.

**The `Provider` seam stays provider-agnostic — because restricting it
would delete almost nothing.** Verified against the code rather than
assumed: the only nix-aware line in shared provider machinery is
`capture::store_grants` looking for `/nix/store` in the activated
`PATH`. Everything downstream — `Resolution`, policy compilation, the
hardened tier's bind mounts, `meta.json` — consumes opaque path lists.
The seam's genericity has now been exercised by three providers with
three different activation mechanisms (flox, nix flakes, devbox) at
near-zero marginal cost. Hard-coding nix into it would be a
falsifiable-sounding decision that in fact changes nothing measurable.

**The product guarantee is the closure tier.** This is where the
commitment lives, and it is grounded in a measurement, not a
preference: a full build from a closure needs the project root, `/tmp`,
and the store — zero host library grants, host toolchain denied
(`own-policy-baseline`, verified live for both flox and nix closures).
An artifact-tier provider structurally cannot make that claim; its
runtime links against whichever libc each host happens to have, which
degrades reproducibility and widens the policy in the same stroke.

Two consequences, stated so they can be checked later:

- **nix-specific optimizations land as closure-tier features, not as a
  reason to close the door.** The concrete candidates — narrowing store
  grants from the store root to the resolved closure's exact paths (so
  two sandboxes stop seeing each other's toolchains), pinning resolved
  closures as GC roots so `nix-collect-garbage` cannot break a live
  sandbox, store integrity verification at `up` — all work without
  rejecting the artifact tier. Nearly the entire benefit of
  "optimizing for nix" is available without the restriction, which is
  why the restriction is not taken.
- **The artifact tier is not rejected; its bar is raised from
  "qualifies" to "re-qualifies under demonstrated demand".** mise still
  passes the six criteria on paper, and the removed
  `add-mise-provider` sketch (mandatory `mise.lock`, devcroft-owned
  append-only store, declared host grants rendered with a
  `provider:mise` origin) remains the shape an implementation would
  take. What changed is the burden of proof, informed by what
  qualifying devbox actually cost: every provider requires per-entry-
  point measurement (which commands run hooks, what leaks from global
  state, how self-contained the result is), and nix-based providers
  amortize criteria 3 and 5 across a shared store model while each
  artifact provider is a new store design to qualify from scratch.
  "The third provider is free" was true because it was the third *nix*
  provider.

**Revisit if** either input changes: real demand for an artifact
provider materializes (at which point the rendered-grants distinction
above gets its practical test), or the nix monoculture cost bites —
upstream churn in flakes, the daemon, or store semantics now hits every
provider devcroft ships simultaneously, and that concentration risk is
accepted here, not denied.

### Not yet built: devenv

**Property that fails:** none established. This is a sequencing entry, not
a rejection, and it exists because devenv was previously only *classified*
here (as Nix-based, in the tier list above) and never argued.

devenv is closure tier by construction: it builds on Nix, so criteria 3
and 5 are answered by the same shared store model flox, nix flakes and
devbox already use. That makes it the cheapest provider left to add —
the "third provider is free" amortization applies to it, because it would
be the fourth *nix* provider rather than the first of a new kind.

**The one open question is criterion 4**, and it must be measured rather
than assumed. `enterShell` is project code, so the entry point devcroft
uses has to hand back the environment without running it. devenv wrapping
Nix makes a clean answer *likely* — nix itself has one in
`print-dev-env --json` — but "likely" is exactly what the criterion-4
table above exists to refuse: two shipped providers violated this
unnoticed (`fix-provisioning-hooks`) precisely because the entry point was
assumed rather than checked.

**Scheduled at 0.5, with `sandbox-provisioning`, and the reason is that
measurement.** What the correct behaviour *is* for a provider that runs a
hook changes at exactly that release: today devcroft warns, because
provisioning is unconfined either way and refusing would block a user from
something their own shell does identically; under `sandbox-provisioning`
the promise becomes "activation is confined" and such a provider must fail
closed at layer `provider`. Qualifying devenv before 0.5 means measuring
against a promise about to change and then revisiting the decision;
qualifying it at 0.5 means deciding once, against the final one.

It is wanted before 0.6 rather than after: `add-manifestless-mode` exists
to be pointed at repositories nobody has read, and detecting a
`devenv.nix` only to report it unsupported is a poor version of that.

Two things to settle when it is built, neither affecting qualification:
devenv's `processes` are supervised by process-compose, the same
supervisor devcroft already generates its own config for (`src/services`),
so the overlap needs a decision rather than a collision; and its `services`
concept has to map onto `ServiceSupport` the way flox's does.

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

### Security guarantee: one tier, and what it does not cover

devcroft has **one** isolation tier: nono with Landlock on Linux, Seatbelt on
macOS. It protects against accidents, careless commands, and simple
exfiltration. It is **not** a defense against a determined attacker who
controls the code running inside — the entire host kernel syscall surface
remains reachable, so a kernel bug is an escape.

For a boundary stronger than that, run devcroft inside a VM. That is the
supported answer rather than a deflection: it is already how the macOS path
works. See `docs/threat-model.md` — use case B, unreviewed code in many
instances, is not served and must not be claimed.

Two rules follow, and they are not negotiable:

1. Claims are stated at the strength the one tier actually provides. devcroft
   never says "sandboxed" without saying what that protects against.
2. Backends document their own limits; devcroft inherits those limits rather
   than optimistic summaries of them.

### Removed: the gVisor hardened tier

**Property that fails: it cannot compose with the sandboxing core, and it
cannot support fleet.**

There was a second tier, backed by gVisor. It was built, and it worked —
`up`, `exec`, SSH and services all verified end to end against a real rootless
`runsc`. It was removed anyway (`remove-gvisor-backend`), and the code is
recoverable at the tag **`gvisor-backend-last`**.

Three reasons, in the order they carry weight:

1. **Landlock cannot confine anything that builds its own filesystem view.**
   `runsc` needs `mount()`; Landlock has no hook for `mount()` at any ABI
   version, so the two fail together with `EPERM` under any ruleset, however
   permissive — confirmed by elimination, including a ruleset granting `/` full
   read-write. Stacking the tiers was structurally impossible, not fiddly.
   Composing the other way would need gVisor to implement the Landlock
   syscalls, and would only restrict paths inside a root filesystem devcroft
   already constructs entirely.
2. **The middle was squeezed.** Below it, the process tier is cheaper and
   matches what devcroft is for. Above it, a VM is stronger and already
   required on macOS. A tier more complicated than the first and weaker than
   the second has to earn its place, and every new capability would have been
   designed twice.
3. **Rootless operation cost it the property it was chosen for**, though this
   reason is narrower than it first looks and the narrowing matters. `runsc`
   rejects its sandboxed-network mode under `--rootless`, so the tier could
   never have the per-instance netstack fleet wants. But port separation at
   that tier never came from `runsc` — it came from the network namespace
   devcroft requests in its own OCI spec, so a *deny-default* hardened sandbox
   did get its own loopback and did not collide. The host's port space was
   shared only once egress was granted. A tier whose port isolation vanishes
   the moment you allow network access still cannot carry fleet; but "it
   structurally lacks it" would be the overstatement `add-port-allocation`
   already caught once.

**Not a reason, recorded so the removal is not defended by more than the facts
support:** "a backend outside the sandbox library gets none of its
capabilities". Most of that library's modules — host filtering, supervisor IPC,
attestation, audit, snapshots — run in the supervisor, outside any sandbox, and
work regardless of backend. What genuinely could not transfer was the
capability-set-to-Landlock path and its ABI-level scoping. Real, but narrow.

**Kept:** the `SessionBackend` trait, so a future backend is an addition rather
than a re-architecture, and a written set of criteria any candidate must meet
(`remove-gvisor-backend`'s design.md). Also kept: three integration defects that
only appeared when real toolchains ran — mount destinations needing to exist in
the bundle beforehand, `root.path` needing to be absolute for gVisor's
symlink-escape guard, and `runsc exec` rejecting the `--` separator its
Docker-shaped equivalent accepts. None were in any documentation. Budget for
that class of defect in any sandbox integration.

**Revisit if** a candidate meets the recorded criteria — composes with or runs
beneath the sandboxing core, supports what fleet needs, runs real toolchains,
is called a security boundary by its own authors, and is reachable from Rust as
a library rather than only as a subprocess.

Under-promising is cheaper than retracting a claim after someone
demonstrates a bypass.

### Rejected (for now): non-rootless gVisor for netstack

> **Superseded by the removal above.** The tier this entry is about no longer
> exists, so its conclusion is no longer load-bearing. Kept because the
> measurements are: they are why `remove-gvisor-backend`'s third reason is
> narrower than it looks, and the cost table at the end is the record of what
> non-rootless operation would actually have required. Read it as evidence,
> not as a live decision.

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

**Not reopened by `use-nono-library`.** That rewrite landed after this
removal, so the question is fair on timing — but it does not bear on this,
for two independent reasons. It moved the *process* tier from exec'ing
`nono wrap` to calling `nono::Sandbox::apply_auto` in-process, whereas the
removed layer never went through nono in either form: it used the
`landlock` crate directly, which is why removing it dropped that
dependency from `Cargo.toml` outright (it survives in `Cargo.lock` only
transitively, through nono). And the constraint is the kernel's, not an
integration detail — whatever code path installs the ruleset, the result
is a process with an active Landlock ruleset, which is precisely what
`runsc`'s `mount()` gets `EPERM` from. No library API can grant what the
LSM does not mediate.

**Measured, 2026-08-23** (`runsc release-20260810.0`, arm64, this
devcontainer). The "Revisit if" below hypothesized that the privilege
needed might be narrow. It was measured rather than argued, and the
hypothesis did not survive — the requirement is *wider* than the entry's
framing assumed, not narrower. As uid 1000, by elimination:

| configuration | result |
|---|---|
| `--rootless --network=none` | works |
| `--rootless --network=host` | works |
| `--rootless --network=sandbox` | rejected: "sandbox network isn't supported with --rootless" |
| `--network=sandbox` | `cgroup.subtree_control: read-only file system` |
| `--network=sandbox --ignore-cgroups` | `newuidmap failed` |
| `--network=host --ignore-cgroups` | "unable to run a rootless container without userns" |

Two things follow. First, `newuidmap` is a requirement of *any*
non-rootless run by an unprivileged user, not of netstack — netstack is
only the reason to want non-rootless. Second, running `runsc` as root
clears the userns/`newuidmap` requirement entirely and then fails at
`can't run sandbox process in minimal chroot since we don't have
CAP_SYS_ADMIN`. So the grant is not a narrow setuid helper: it is root
*plus* `CAP_SYS_ADMIN` in the container's bounding set — materially
closer to the blanket privileged container this entry rejects than to the
nix-daemon precedent it invokes.

One caveat on the evidence: this devcontainer also cannot run the
unprivileged non-rootless path at all, because `newuidmap` fails here even
for a single-line self-map. Ruled out by measurement, not assumed —
`nosuid` (overlay is `rw,relatime`), any LSM (none active), `no_new_privs`
(0), seccomp (0, `unconfined`), nested userns (init userns, identity map),
missing subuid ranges (`vscode:100000:65536` present), and setuid being
broken (`passwd -S` reads `/etc/shadow`, so elevation works). That failure
is environmental. It does not affect the root-path finding above, which is
what determines the cost.

**Revisit if:** a future backend change is willing to trade rootless for
netstack behind an explicit privilege grant. The shape originally imagined
here — the NOPASSWD sudo rule this repo already gives flox's nix-daemon
for the one root action it actually needs — is now known *not* to be
sufficient; the measurement above is the cost to argue against. Recorded
as a real option in
`add-gvisor-backend`'s own Open Questions, not chosen there either.

### No resource limits (yet)

Landlock and Seatbelt constrain access, not consumption. A session can
exhaust host CPU or memory. With a fleet of agents this matters — one
runaway build can take down the host. Containers get cgroups for free.

Planned mitigation: cgroup v2 scope units per keeper on Linux. macOS has no
comparable mechanism, so the gap there is likely permanent.

**Revisit target moved.** This used to say "revisit at the hardened tier",
because `runsc` integrates with cgroups directly and the process tier has no
equivalent. That tier is gone (see the removal entry above), so the door it
opened is closed with it. Resource limits are now `add-linux-agent-fleet`'s,
where they are foundational rather than incidental: one delegated cgroup v2
scope per agent, which yields limits, atomic teardown via `cgroup.kill`, and
the metrics `ps`/`status` would report — three things from one mechanism.

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

**Superseded on Linux by `add-egress-proxy`, which closes what this entry
called genuinely open at the top level.** The section title itself is now
only half true: filtering by *hostname* still requires the proxy's
cooperation (the kernel gate has no concept of a domain name), but "a
process that deliberately bypasses the proxy is not stopped" no longer
holds for anything, allowed domain or not — `NetworkMode::ProxyOnly`
denies every direct `connect()` except to the proxy's own port, so there
is no raw-socket path around it to any destination, allowlisted or not.
The resolved-IP-scope gap this entry left open is unchanged and is
`nono::HostFilter`'s own stated limit (link-local addresses are denied
regardless; a same-IP different-service risk on an *allowed* domain is
not). macOS status is unchanged from the correction above — genuinely
unverified, not assumed either way; see the README's Status section and
`policy::degraded`'s module doc for the same note.

### Service sidecars: delivered, with one gap named below

devcontainers compose with Docker Compose for databases and other
services. devcroft now has the provider-native equivalent
(`add-flox-services`): a flox environment's documented `[services]`
declarations are read host-side at `up`, started **inside** the sandbox
after restriction, supervised for the sandbox's lifetime, and reaped at
`down`. `status`/`ps`/`logs` report them; no new command was added.

**devcroft supervises, the provider declares — and that split has a
cost worth stating.** devcroft does not shell out to `flox services`:
doing so would need the flox binary and its internals executable inside
the compiled profile, which is exactly what the "environment resolves
once" invariant rejects for per-session activation, and for the same
reason. Nor does it consume flox's own generated `service-config.yaml`,
an undocumented artifact whose process-compose binary belongs to flox's
closure rather than the environment's. Instead devcroft generates a
process-compose config it owns, from the published schema.

The consequence: **`flox services status` run by hand shows nothing**,
because flox did not start these processes. `devcroft status`/`ps`/`logs`
are where they appear, and `devcroft doctor` says so explicitly in any
project that declares services, rather than leaving the empty list to be
misread as "my services never started". The other half of the cost is
that `process-compose` must be declared in the project's own
environment — a devcroft implementation choice leaking into the
project's manifest, accepted as the lesser evil against depending on a
binary the environment never declared.

### Two sandboxes declaring the same service port still collide

**Property that fails:** nothing separates the two sandboxes' loopback.

A committed `devcroft.toml` describes an *instance*, so every git
worktree of a repo declares the same port, and two sandboxes both asking
for 5432 collide with `EADDRINUSE`. At the `process` tier there is no
PID/mount/net namespace separation between sandboxes (`add-mvp-core`
design.md Decision 5), so both are binding the same host loopback.

**With one tier, the collision is unconditional**: at `process` it is real at
any N > 1, and there is no longer a second tier to qualify that with.

The tier-dependence this entry used to describe is worth keeping as history,
because getting it wrong cost a correction once. The hardened tier's port
separation never came from gVisor's netstack — it came from the network
namespace devcroft requested in its own OCI spec, so a deny-default hardened
sandbox did *not* collide, while one with egress granted did. That nuance died
with the tier; what survives is the lesson that the separation was devcroft's
doing rather than the backend's.

So the separation that does exist comes from the namespace devcroft asks
for in the OCI spec, not from gVisor's own netstack — which is
unavailable under `--rootless` for the reasons the netstack entry above
records.

**Revisit via:** `add-port-allocation`, which allocates a free loopback
port per sandbox where the collision exists and surfaces it through
`status`. It is scoped by resolved network mode rather than by tier, for
the reason above, and pairs with `add-agent-workload` — that change gives
N worktrees distinct sandbox *names*; without it they never get as far as
needing distinct ports.

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

**The blast radius now includes services** (`add-flox-services`). The
keeper owns the service supervisor's lifetime — it has to, since `up` is
a short-lived CLI process and a disconnected session is escalated after
two seconds — so a dead keeper takes every service in that sandbox with
it, database included. This widens an already-published gap rather than
introducing a new one, and it is the reason service state is *queried
live* from the supervisor rather than recorded at `up`: a keeper that
died must not leave `status` confidently reporting services that are no
longer running.
