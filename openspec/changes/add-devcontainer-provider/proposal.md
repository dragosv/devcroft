## Why

Most repositories that describe their environment at all describe it in
`.devcontainer/devcontainer.json`, not in a flox manifest or a flake. For
a fleet of agents that is the adoption wall: devcroft can give N agents
isolated, one-build-many-sandbox environments, but only for the projects
that already speak Nix. This change lets devcroft read the file those
projects already have — and keep every invariant it has: Docker builds
the image, once, at provisioning; devcroft owns the runtime, with nono,
the mount view, the network namespace and the proxy, exactly as on
`/nix/store`. Docker is never in an agent's execution path.

## What Changes

- A fifth `env.provider`, `devcontainer`, **Linux-only** (a container
  rootfs is a Linux filesystem; on macOS the answer stays
  `add-macos-service-vm`, not this).
- **Materialization, once, host-side:** the provider reads
  `devcontainer.json`, resolves `image` to a digest, and uses an OCI
  runtime (`docker` or `podman`, whichever `doctor` finds) to produce a
  **read-only rootfs on the host**, keyed by digest and shared by every
  sandbox of every project that names it. In the first cut only a
  digest-pinned `image:` is honored; `build:`, `features` and
  `postCreateCommand` are **refused by name** with the reason, and are
  the next cut — they too run inside the build container at
  materialization, never at `up`.
- **Runtime is devcroft's, unchanged:** the rootfs is a read-only
  provider grant (`provider:devcontainer` origin), the project root is
  read-write, the mount view binds both, the keeper self-restricts with
  nono, the shell resolves out of the rootfs through `src/shell.rs`'s
  declared-grant guard, `PATH`/`ENV` come from the image config as data.
  No container runs. No Docker socket is granted. Nothing from
  `docs/decisions.md` §2 (DinD, `runArgs`, `privileged`,
  `initializeCommand`) changes.
- **A third guarantee tier, `image`**, user-visible at `up` and in
  `status`: identical rootfs for an identical digest, host-independent
  *within Linux*, but digest-addressed rather than content-addressed and
  with no closure guarantee below the image (a `latest` tag is refused;
  only `image@sha256:…` or a resolved digest recorded in a lockfile
  devcroft writes). Neither `closure` nor `artifact` describes it, and
  §1 forbids selling two guarantees under one word.
- **Staleness** is the digest: `status` reports stale when
  `devcontainer.json` or the recorded digest changes; `up --recreate`
  re-materializes.
- Provisioning needs an OCI runtime binary on the host — an external
  dependency **at provisioning time only**. The invariant "the process
  tier requires no external backend binary" is kept; the `doctor` entry
  says the runtime is needed and for what.
- `devcroft init` learns to recognise `.devcontainer/devcontainer.json`,
  ranked below every closure provider and above `swift`.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `env-provider`: a `devcontainer` provider resolution requirement; the
  "Only declarative providers" list gains `devcontainer` with the fields
  it honors and refuses; a tier `image` beside `closure` and `artifact`;
  staleness by digest; the materialization/runtime split as a
  requirement, not a note.
- `config`: `provider = "devcontainer"` accepted on Linux, refused with
  the platform reason on macOS.
- `cli`: `init` detection and ranking; `doctor` reports the OCI runtime
  and the rootfs store.
- `policy`: the rootfs grant renders with origin `provider:devcontainer`;
  the exec set inside the sandbox is the rootfs's, not the host's (which
  `execute-scoping` already gives on Linux by construction).

## Impact

- New `src/provider/devcontainer.rs`; `ProviderKind::Devcontainer`;
  `validate.rs` list; `init`/`doctor` in `src/bin/devcroft.rs`; a tier
  value — `add-swift-provider` introduces the `Tier` type and this change
  adds the third variant, so it lands after that branch or carries the
  enum itself.
- A rootfs store under devcroft's data dir (`<data>/rootfs/<digest>/`),
  baseline-denied to sandboxes except for the one rootfs each is granted.
- `docs/decisions.md` §1 gets the sixth criterion's answer for this
  provider (preconditions: runtime present, digest resolvable) and the
  `image` tier's definition; §2's "Covered differently" gains the
  "`features` → run at build, in the container" entry.
- Tests: Linux e2e against a small pinned image (`debian:stable-slim@sha256:…`)
  — resolve, up, exec a rootfs binary, refuse a host binary, shell from
  the rootfs, `status` stale after digest edit; a refusal test for
  `latest`, `build`, `features`, `postCreateCommand`, `remoteUser`,
  `mounts`, `runArgs`; a macOS refusal test. CI gets an `e2e
  (devcontainer)` leg with docker available (it is, on `ubuntu-latest`).
- `samples/devcontainer-sample/` with the file most projects already
  have, unmodified.
