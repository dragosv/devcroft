# Change: add-hardened-tier

> **Implemented, then removed — except for one piece.**
> `remove-gvisor-backend` deleted this tier and its dispatch, since its
> only concrete backend (`add-gvisor-backend`) could not be stacked with
> Landlock at all. What survives is the backend-generic `SessionBackend`
> seam this change introduced: it is what a second backend would attach
> to, and it costs nothing to keep. Everything below is the record of what
> was built and measured, not current state; the tier's removal, its three
> reasons, and the criteria for a future backend are in
> `openspec/changes/remove-gvisor-backend/`. Deliberately not archived —
> archiving would sync these delta specs into the main specs as truth.

Status: in progress (post-MVP). Depends on: add-mvp-core complete;
implemented alongside `add-gvisor-backend`, which concretizes this
tier's first backend via direct `runsc` integration — `dragosv/mxc`'s
`gvizor` branch was consulted as an implementation reference, not taken
as a dependency.

## Why

At the `process` tier devcroft cannot claim to stop a determined attacker:
the full host kernel syscall surface is reachable from inside, so a kernel
bug is an escape. That is acceptable for accident protection and it is the
right default, but it caps the product's addressable use cases — anyone
running genuinely untrusted code, or agents on a machine that also holds
production credentials, needs a real boundary.

gVisor closes that gap without giving up the density thesis. Sentry
implements syscalls in user space, so an escape requires a Sentry bug
rather than a host kernel bug. Landlock applied to the Sentry process is
additive (gVisor already seccomps Sentry): if Sentry is compromised, its
own filesystem reach stays bounded by the compiled policy. Unlike microVMs,
the per-sandbox cost stays in tens of MB, so fleets remain viable and the
shared read-only store still works.

Strategically this is consumption, not competition: the hardened tier is
delivered by an existing isolation backend, not by duplicated code in
devcroft.

## Backend candidates

Two candidates preserve the density thesis. MicroVM backends (Firecracker,
Cloud Hypervisor, Kata) are excluded here: they give a stronger boundary
but reintroduce per-sandbox kernel and rootfs cost, which is precisely what
devcroft exists to avoid. Other options were considered and rejected:
Hyperlight (no full Linux ABI), Occlum and Gramine (LibOS but SGX-oriented),
Sysbox (still the host kernel, weaker than gVisor with no compensating
gain), User-mode Linux (perfect compatibility, worse performance than
gVisor).

### Candidate A: gVisor (+ Landlock)

Mature, GKE-proven, well-understood failure modes. Sentry implements
syscalls in user space; Landlock on the Sentry process is additive defense
in depth. `runsc exec` provides an exec-into primitive.

Concretized in its own change, `add-gvisor-backend`, which answers this
proposal's rootfs/store-sharing/exec open questions for gVisor
specifically (and corrects the platform list below: systrap has been
runsc's default since mid-2023; ptrace is deprecated and not targeted).
This proposal stays backend-generic on purpose.

Cost: syscall overhead of roughly 2-10x, which lands on exactly the wrong
workload — builds are syscall-heavy. directfs mitigates the filesystem
path; the trap mechanism is the floor and cannot be optimized away.

### Candidate B: LiteBox (+ Landlock)

Rust library OS: OS services are linked into the workload rather than
mediated by a separate supervising process, so syscall traps are avoided in
many cases. This is the architectural answer to the build-performance
problem — it reduces the number of boundary crossings rather than the cost
of each one. Modular North (POSIX-like Rust API) / South (host platform)
split; SEV-SNP support is a bonus, not a requirement here.

Cost: experimental, no production ecosystem, APIs still changing. Its
being written in Rust is a memory-safety decision, not the source of its
performance advantage — the library OS architecture is.

### Selection principle

The tier is defined by its guarantee, not by its backend. If both
candidates ship, `isolation = "hardened"` resolves per host and `status`
names the concrete backend. Nothing in the manifest schema or session
semantics may depend on which one is in use.

## What Changes

- New concept: **isolation tier**, derived from the backend, surfaced in
  `status` and once at `up`, and required to qualify every security claim
  in docs and CLI output.
  - `process` — nono (Landlock / Seatbelt). Default. Unchanged behavior.
  - `hardened` — gVisor or LiteBox, plus Landlock. Linux only.
- Backend selection in the manifest gains an intent form:
  `isolation = "process" | "hardened"`, resolved to a concrete backend per
  host. `hardened` on a host that cannot provide it is a hard failure at
  layer `backend`, never a silent downgrade to `process`.
- `doctor` reports hardened-tier availability (runsc present, systrap or
  KVM platform, kernel support) with the actionable fix per failure.
- Policy compilation targets the hardened backend's policy model; the
  manifest schema does not change.
- Keeper simplification: where the backend provides an exec-into
  primitive (e.g. `runsc exec`), the spawn-server trick is not required.
  The keeper remains for the `process` tier and for any hardened backend
  without one. Session semantics (exec, shell, signals, SSH) MUST be
  identical across tiers and backends.

## Impact

- Affected specs: lifecycle (tier-conditional keeper), policy (tier
  annotation), cli (`status`, `doctor`), config (`isolation` key).
- `docs/decisions.md` §4 already states the security claim as
  tier-dependent; this change makes the tier real.

## Success Criteria

- `isolation = "hardened"` on a supported Linux host comes up, and
  `status` shows `isolation: hardened`.
- The same manifest, sessions, and SSH workflow work unchanged across both
  tiers; only performance and the stated guarantee differ.
- On macOS, `hardened` fails at `up` with layer `backend` and a message
  stating the tier is Linux-only.
- Benchmarks published for a syscall-heavy build (compile a mid-size Rust
  crate, plus a dependency-install workload) across `process` and every
  shipped `hardened` backend, so users can choose with real numbers
  instead of folklore.

## Open Questions

- **LiteBox: ABI coverage for arbitrary exec.** A dev environment execs
  unmodified binaries constantly — rustc, cc, ld, sh, test runners — and
  a library OS assumes linking. Runners and shims exist, but covering a
  full toolchain is a different bar than sandboxing a single application.
  This must be validated with a real `cargo build` and a real `npm ci`
  before LiteBox is considered viable, not with a hello-world.
- **LiteBox: process model.** fork/exec, process groups, signals, and pty
  behavior under a library OS need to be confirmed to match what the
  session layer (exec spec, ssh spec) already promises.
- **Rootfs shape.** `runsc` normally consumes an OCI bundle. If a hardened
  backend requires a rootfs, devcroft has partially reintroduced images,
  contradicting the pitch. Likely resolution: generate a minimal rootfs
  from the Nix closure at `up` (cheap, content-addressed, no registry).
  Must be confirmed before the tier is promised publicly.
- **Store sharing mechanism.** gofer vs directfs (gVisor), or the South
  interface equivalent (LiteBox), for exposing `/nix/store` read-only to
  many sandboxes; measure which preserves the density advantage.
- **Syscall overhead budget.** Builds are syscall-heavy. Decide whether
  `hardened` is ever a sensible default for any workload class, or always
  opt-in. Note that the lever is reducing boundary crossings, not
  optimizing per-crossing cost — which is the whole argument for
  evaluating a library OS alongside gVisor.
- **macOS.** No equivalent mechanism exists; the tier split is likely
  permanent across platforms, mirroring the guarantee-tier split.
