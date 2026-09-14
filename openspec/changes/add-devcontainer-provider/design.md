## Context

devcroft's four providers share one substrate: a Nix store. Every
mechanism downstream of resolution assumes it loosely — `capture::store_grants`
finds a store root in `PATH`, `shell.rs` resolves a shell from a closure's
requisites, the mount view binds each grant at the same absolute path it
has on the host. `add-test-runtime-fixture` generalized the shell guard
to "inside a declared grant" precisely for a provider that is not
store-backed; this is that provider.

A dev container is two things: an **image** (a Linux rootfs, addressed
by digest) and a **contract around it** (`features`, lifecycle commands,
`remoteUser`, `mounts`, `forwardPorts`). `docs/decisions.md` §2–3 already
decided the contract, field by field. What was never decided is whether
the image can be a devcroft environment without Docker in the runtime
path. This design says it can, and where the two hard parts are.

Constraints that bind every decision below: the two-phase invariant
(provisioning host-side with pinned tooling, never project code;
runtime inside the boundary); "the keeper SHALL NOT be executed as a
child of a separate sandboxing binary"; "the process tier requires no
external backend binary"; the baseline denial of devcroft's own data
dir, which `check_no_deny_overlaps_allow` enforces against any nested
grant; and §1's rule that the guarantee tier is visible and honest.

## Goals / Non-Goals

**Goals:**

- A project with `.devcontainer/devcontainer.json` and a digest-pinned
  `image` runs under devcroft on Linux with the same isolation as a
  flox project: nono, mount view, netns, proxy, SSH.
- N sandboxes of one image share one materialized rootfs; a second
  project naming the same digest shares it too.
- No container runtime or daemon is needed on the host at all: the image
  is pulled and unpacked in-process, and nothing an agent can reach is a
  container.
- The tier is named, and named differently from `closure` and `artifact`.

**Non-Goals:**

- `build:` and `features` — refused by name with the prebuild route (D9-A);
  a confined builder is a separate, later decision (D9-B).
  (`postCreateCommand` is *not* deferred: it runs inside, as a hook — D8.)
- A container runtime anywhere: not at provisioning (D1 pulls and unpacks
  in-process) and not at runtime.
- macOS. A rootfs is Linux; on macOS the answer is a VM
  (`add-macos-service-vm`), and this change refuses with that pointer.
- Running the container. There is no `docker run` anywhere in this
  design, no Docker socket grant, no DinD (§2, unchanged).
- Rootless-Docker-shaped security claims. The tier is about
  reproducibility of the filesystem, not about a stronger boundary.

## Decisions

### D1 — Pull and unpack in-process; no container runtime, no daemon

Materialization is three operations, none of which is "run a
container": fetch the manifest, config and layers for a digest from the
registry; unpack the layers in order into a directory, honouring OCI
whiteouts (`.wh.<name>`, `.wh..wh..opq`); read `Config.Env` and
`Config.User` from the image config. All three are OCI *distribution*
and *image-spec* work, not OCI *runtime* work, and devcroft does them
itself:

- `oci-client` (the crate the `dev` devcontainer CLI uses for the same
  step; `docs/prior-art.md`) for the registry side — manifests, auth,
  blobs, by digest.
- `oci-spec` for the config types.
- Layer unpacking is devcroft's own: `tar` + `flate2`/`zstd`, applied
  layer by layer with whiteout handling, device nodes skipped (the mount
  view supplies `/dev`), setuid bits dropped (they would be setuid to the
  user anyway). Roughly a hundred lines, measured in phase 0 against
  `debian:stable-slim` and `alpine`, both of which use whiteouts.

The result lands in `<rootfs-store>/<digest>/`, read-only after a
completion marker is written; the digest is recorded in a lockfile
devcroft owns (`.devcontainer/devcroft.lock`, next to the file it reads).

**No Docker, no Podman, no daemon, no socket, no `docker` group, no
rootless setup.** The provider's only host requirement is network to
the registry at provisioning time — the same requirement every closure
provider has for its store — and `doctor` reports exactly that.

Alternatives considered:

- **`docker create` + `docker export`** — the first draft. Rejected: it
  makes a daemon and a group membership a prerequisite for a provider
  whose whole point is that Docker is not in the picture, and `doctor`
  would have had to explain "present but not usable by this user" for a
  tool devcroft does not otherwise need. What it offered — images that
  exist only in the local Docker cache, never pushed — is a `build:`
  concern, and `build:` is D9.
- **youki** — an OCI *runtime* (a `runc` in Rust). It runs a bundle that
  is already unpacked; it neither pulls nor unpacks. It is the part of
  the pipeline this design deliberately has none of, and would bring
  cgroups, seccomp and namespace code devcroft would not call. Not a fit
  for pull, and not a builder either (D9).
- **`skopeo`/`umoci`** — do the right operations, as external binaries.
  Rejected for the same reason as Docker, weaker: two tools to require
  for what two crates provide.

Why "never run the image" survives the change of mechanism unchanged:
nothing here executes an entrypoint, a `CMD`, or a lifecycle hook on
the host. The two-phase invariant holds at the provider's entry point
exactly as it does for `nix print-dev-env --json` — the route that hands
back an environment, not the one that runs a command.

### D2 — The rootfs store is a sibling of the data dir, not inside it

`~/.local/share/devcroft` is baseline-denied and the deny wins over any
nested grant by construction. The rootfs must be granted, so it lives at
`$XDG_CACHE_HOME/devcroft/rootfs/<digest>/` (default
`~/.cache/devcroft/rootfs/`). The store as a whole is not granted; each
sandbox is granted exactly the one digest it resolved, read-only, with
origin `provider:devcontainer`. Two sandboxes of two projects on the
same digest are granted the same directory, which is the sharing §1's
third criterion asks for.

### D3 — The rootfs is the *session's* root, not the keeper's

An image's binaries name absolute paths: `/lib64/ld-linux-x86-64.so.2`,
`/usr/bin/env`, `/etc/ld.so.cache`. Binding the rootfs at a prefix and
pointing `PATH` at it does not work — the interpreter path in every ELF
header is absolute. The rootfs has to be `/` for the processes that use
it.

It cannot be `/` for the keeper. The keeper is the host's `devcroft`
binary, linked against the host's libc; execing it into a view whose
`/lib` is the image's — a different glibc, or musl — fails at the
loader. Two options were weighed:

- **Static keeper** (musl, `-static-pie`). Clean, but changes how devcroft
  is built and shipped for one provider, and `cargo install devcroft`
  would need the target.
- **Two-level view.** The keeper keeps today's view (host-shaped, with the
  rootfs directory granted read-only as an ordinary provider grant).
  Each *session* is spawned into a child mount namespace whose root is
  the rootfs, with the project root, private `/tmp`, `/proc`, `/dev`
  and the proxy socket bound in — the same construction as
  `construct_view`, with a different root. Chosen.

What makes the second option correct rather than merely convenient:
Landlock rules are attached to **inodes** at rule time, not to paths.
The keeper grants `<store>/<digest>` by opening it (nono's `allow_path`
is an `O_PATH` open); when a session later sees the same inodes at `/`,
the rules follow the inodes. Nothing is granted twice and nothing is
granted by the session's own pivot. This is a **phase-0 measurement**,
not an assumption: task 0.3 proves it with a probe before any provider
code exists. `policy --render` shows the real path
(`~/.cache/devcroft/rootfs/<digest>  provider:devcontainer`), which is
honest — that *is* what was granted — and `status` says where a session
sees it.

### D4 — Environment is the image config, taken as data

`docker inspect <digest>` yields `Config.Env`, `Config.WorkingDir`,
`Config.User`. `Env` becomes the resolved environment (the diff against
`canonical_base_env`, as every provider does); `PATH` is the image's
own, which is what makes `shell.rs` resolve `/bin/sh` *inside the
rootfs* — the declared-grant guard passes because the rootfs is the
grant. `WorkingDir` is ignored (the project root is the cwd, as for every
provider). `User` is refused unless it is root or absent: devcroft runs
as the invoking user, and an image whose tooling only works as
`vscode:1000` is one whose files are owned by an unmapped uid — the
`remoteUser` rejection in §2, at the image level. Nothing is executed to
obtain any of this.

### D5 — A tier named `image`

Not `closure`: there is no dependency graph below the digest, and two
digests of "the same" Dockerfile differ. Not `artifact`: the runtime is
*not* host-linked — the image brings its own libc — which is precisely
the property `artifact` lacks. `image` is: identical filesystem for an
identical digest, on any Linux host; no guarantee about how the digest
was produced. Reported at `up` and in `status`, per §1. Refuses a tag
without a digest at resolution, with the remedy (`image@sha256:…`, or
`devcroft up` once with the tag to record the digest it resolved to —
after which the recorded digest is what counts, like a lockfile).

### D6 — Fields honored and refused, by name

Honored: `image` (digest-pinned or lock-recorded); `postCreateCommand`
and `postStartCommand` (D8); `name` (ignored with no error),
`forwardPorts` (ignored: SSH forwards; §3), `customizations.*` (ignored:
editor-side). Refused in this cut, each with its own message: `build`,
`features`, `postAttachCommand`, `initializeCommand`, `remoteUser`,
`containerUser`, `mounts`, `runArgs`, `privileged`, `capAdd`,
`securityOpt`, `dockerComposeFile`. `updateRemoteUserUID`,
`overrideCommand`, `shutdownAction`, `waitFor` are ignored as
meaningless without a container. Unknown keys are ignored — the file is
shared with other tools and devcroft must not fail on their extensions.

`dockerComposeFile` is refused in this cut, and the reason is scope,
not impossibility. A compose file is N containers, each with its own
image, entrypoint, network identity (the compose DNS name `db` the app
connects to) and volumes; this change materializes one image. The
mapping for the rest exists and reuses what devcroft already has: each
sibling service becomes a materialized rootfs of *its* image, its
entrypoint/command a process under process-compose inside the same
boundary (the `[services]` machinery, with `depends_on` →
`process_healthy` via `add-service-readiness`), compose service names
resolved to the sandbox's loopback (the network namespace already gives
each sandbox its own port table), named volumes as read-write grants
under the sandbox's artifact directory, bind-mount volumes refused as
`mounts` are. That is a change of its own — it needs a session view per
service rootfs, an environment per service, and measurements of
entrypoints that expect to be PID 1 or root — and it is the one that
would make "a project with a compose-based dev container" fully served.
It is deferred, and the refusal message says so and points at
`[services]` for the interim.

### D8 — Lifecycle commands run inside the boundary, as the hooks they are

`postCreateCommand` is project code that runs after the environment
exists. devcroft already has the rule for that category and applies it
to flox's `on-activate` and devenv's `enterShell`: never on the host at
provisioning, always inside, under the manifest's policy. So
`postCreateCommand` becomes the sandbox's `post_create` hook and
`postStartCommand` its `post_start` — same once-per-creation and
once-per-start semantics the format specifies and devcroft's hooks
already have. The string and array forms are both accepted; the object
form (parallel named commands) runs its entries in key order.

What differs from a container is not whether the command runs but what
it can touch, and that difference is the design, not a gap. The project
root is writable; the rootfs is read-only and shared. `npm install`,
`cargo fetch`, `pre-commit install`, `pip install --user` into a
project-local venv all work. `sudo apt-get install`, `pip install` into
the system prefix, anything writing under `/usr` or `/opt`, fail at
layer `keeper` — and the failure message says the rootfs is shared by
every sandbox on this digest, that mutating it would break the one
guarantee the tier makes, and that `features` (next cut) is where
`/usr`-level installs belong. A command that needs the network needs a
`network.allow` entry, exactly as every hook does; the two-phase
invariant's consequence — "a hook that needs the network needs an
allowlist entry" — applies unchanged.

Rejected alternative: run `postCreateCommand` at materialization, in
the build container, and commit the result into the rootfs. That is
what `features` are for; doing it for `postCreateCommand` would run
project code with the build container's authority (root, network) on
the host's Docker daemon, at provisioning — the thing the two-phase
invariant exists to forbid — and would bake one project's `npm install`
into a rootfs another project on the same digest shares.

### D7 — Staleness is the recorded digest, plus the file

`status` reports stale when `devcontainer.json`'s hash or the lock's
digest differ from what `Meta` recorded at `up`; `up --recreate`
re-resolves and, if the digest changed, materializes the new one. Old
rootfs directories are not garbage-collected by this change — `rm`
of the last sandbox naming a digest does not remove it, and a `devcroft
gc` is post-MVP surface, not added here.

### D9 — `build:` and `features` are prebuilds first; a confined builder is a later, separate decision

A Dockerfile in the repository is project code, and a `RUN` step in it
executes with root-in-container and the network. Building an image is
therefore the one thing in this provider that would run project code at
provisioning — the exact act the two-phase invariant forbids — and it
needs a container runtime to do it. That is why `build:` and `features`
are not "pull with one more step"; they are a different question, and
this change answers it in two parts.

**A, this change: devcroft never builds.** `build:` and `features` are
refused with a message that names the route: build the image where
images are built — in CI, on a runner that has Docker anyway — push it,
and pin `image@sha256:…`. The reference `devcontainer build --push`
produces exactly that image, features applied, in one CI step; GitHub
Codespaces' prebuilds are the same model. For a fleet this is not a
workaround but the right shape: N agents pull one digest instead of
each running one build. `docs/decisions.md` §3 already maps
"prebuilds → binary caches" for the closure tier; this is the same
mapping for the image tier.

**B, a later change, and a decision rather than a task:** a rootless,
daemonless builder at provisioning — `buildah`/`podman build` in its own
user namespace, output an OCI layout devcroft unpacks with D1's code.
`RUN` would run as root-mapped-to-the-user with no host access beyond
the build context, which makes the build container the boundary for
build-time project code the way the sandbox is the boundary for hooks —
*but with the build's own, unfiltered network*, which is precisely the
authority `fix-provisioning-hooks` refused to hand a flox hook. Whether
that exception is acceptable is the owner's call, written into that
change's proposal, not inherited from this one. Rejected outright for B:
Docker/BuildKit through a daemon (`bollard`, the `dev` CLI's route) —
`RUN` with the daemon's authority; and a devcroft-written builder on
youki — BuildKit reimplemented to avoid one binary.

## Risks / Trade-offs

- [The image's libc predates the host kernel's expectations, or the
  rootfs is musl and a session tool wants glibc] → D3 keeps the keeper
  on the host's libc; sessions run entirely on the image's, which is
  exactly what the image promises. Phase 0 runs `debian:stable-slim`
  and `alpine` both.
- [A 1–2 GB rootfs makes the mount view or Landlock slow] → one bind
  and one Landlock rule for the whole tree, same as `/nix/store`; phase
  0 measures `up` latency and records it.
- [`oci-client` brings a dependency tail — `reqwest`, TLS, async runtime]
  → measured in phase 0 the way nono's tail was, as a number; devcroft
  already links `reqwest`-shaped crates through `russh`/`sigstore`, so
  the marginal count is what matters. A registry that needs a credential
  helper (`~/.docker/config.json` `credHelpers`) is out of scope for the
  first cut; `doctor` says so when it sees one.
- [Unpacking gets whiteouts subtly wrong and a file from a lower layer
  survives] → phase 0 diffs devcroft's unpack of `debian:stable-slim`
  against `docker export` of the same digest on a host that has Docker;
  identical trees or the difference explained.
- [An image exists only in a local Docker cache, never pushed] → not
  served; the refusal says "push it and pin the digest", which is D9-A's
  answer for `build:` as well. A local-cache fallback via `docker export`
  is a two-line addition if it is ever wanted, and it is not wanted by
  default because it reintroduces the daemon as a dependency.
- [A digest-pinned `image` is rare in the wild; most files say
  `image: mcr.microsoft.com/devcontainers/rust:1`] → the lockfile
  (D5) records the digest the tag resolved to at first `up`; the file
  is not edited; reviewers see the lock.
- [Someone reads "devcontainer support" as "runs my devcontainer"] →
  `up` prints the tier and the refusal messages name what is not run;
  `docs/decisions.md` §2 gets a paragraph saying which half of the
  format this is.

## Migration Plan

Additive. No existing provider, manifest, or state file changes shape.
`Meta` gains a recorded digest with `#[serde(default)]`.

## Open Questions

1. For D9-B, if it is ever taken: `buildah` versus `podman build` as
   the rootless builder, and whether `features` are applied by the
   reference CLI's `devcontainer build` (Node) or by devcroft generating
   the Dockerfile the Features spec describes. Not this change's to
   settle; phase 0 records what the reference CLI does with
   `--skip-post-create` so that change starts from a measurement.
2. Whether the rootfs store should be content-addressed after
   extraction (hash the extracted tree) so two digests with identical
   contents share space. Cheap to add later; `image` tier's promise
   does not depend on it.
