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
- Docker/podman is used once, to produce bytes on disk, and is absent
  from every process an agent can reach.
- The tier is named, and named differently from `closure` and `artifact`.

**Non-Goals:**

- `build:`, `features`, `postCreateCommand` — next cut, same phase
  (materialization), explicitly refused by name in this one.
- macOS. A rootfs is Linux; on macOS the answer is a VM
  (`add-macos-service-vm`), and this change refuses with that pointer.
- Running the container. There is no `docker run` anywhere in this
  design, no Docker socket grant, no DinD (§2, unchanged).
- Rootless-Docker-shaped security claims. The tier is about
  reproducibility of the filesystem, not about a stronger boundary.

## Decisions

### D1 — Materialize to a rootfs on disk; never run the image

`docker create <image@digest>` + `docker export` (or `podman` with the
same verbs) yields a tar of the container's filesystem without starting
it. devcroft extracts it, as the invoking user, into
`<rootfs-store>/<digest>/`, read-only after extraction, and records
`digest` in a lockfile it owns (`.devcontainer/devcroft.lock`, next to
the file it reads). Device nodes in the tar are skipped — the mount
view supplies `/dev`; setuid bits are dropped on extraction — they
would be setuid to the user anyway, and a sandbox must not gain a
privilege the user lacks.

Alternative: use the OCI layer store directly (`skopeo`/`umoci`). Rejected
for the first cut: two more tools to require, for a result `docker
export` already gives; revisit if extraction cost measured in phase 0
says so.

Why `create`+`export` and not `run`: `run` executes the image's
entrypoint — project-controlled code — on the host. `create` executes
nothing. This is the two-phase invariant at the provider's own entry
point, the same choice made for every other provider (the entry point
that hands back an environment, not the one that runs a command).

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

Honored: `image` (digest-pinned or lock-recorded), `name` (ignored with
no error), `forwardPorts` (ignored: SSH forwards; §3),
`customizations.*` (ignored: editor-side). Refused in this cut, each
with its own message: `build`, `features`, `postCreateCommand`,
`postStartCommand`, `postAttachCommand`, `initializeCommand`,
`remoteUser`, `containerUser`, `mounts`, `runArgs`, `privileged`,
`capAdd`, `securityOpt`, `dockerComposeFile`. `updateRemoteUserUID`,
`overrideCommand`, `shutdownAction` are ignored as meaningless without a
container. Unknown keys are ignored — the file is shared with other
tools and devcroft must not fail on their extensions.

### D7 — Staleness is the recorded digest, plus the file

`status` reports stale when `devcontainer.json`'s hash or the lock's
digest differ from what `Meta` recorded at `up`; `up --recreate`
re-resolves and, if the digest changed, materializes the new one. Old
rootfs directories are not garbage-collected by this change — `rm`
of the last sandbox naming a digest does not remove it, and a `devcroft
gc` is post-MVP surface, not added here.

## Risks / Trade-offs

- [The image's libc predates the host kernel's expectations, or the
  rootfs is musl and a session tool wants glibc] → D3 keeps the keeper
  on the host's libc; sessions run entirely on the image's, which is
  exactly what the image promises. Phase 0 runs `debian:stable-slim`
  and `alpine` both.
- [A 1–2 GB rootfs makes the mount view or Landlock slow] → one bind
  and one Landlock rule for the whole tree, same as `/nix/store`; phase
  0 measures `up` latency and records it.
- [`docker export` needs the daemon, which needs the user in the `docker`
  group or rootless mode] → `doctor` names it; podman rootless is the
  alternative the same `doctor` entry offers. This is a provisioning-time
  requirement, and the invariant about runtime binaries is untouched.
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

1. Whether `features` in the next cut are run by `devcontainer build`
   (the reference CLI, Node) or by devcroft driving `docker build` with
   the feature install scripts — the former is one more external tool,
   the latter re-implements the Features spec. Not this change's to
   settle; phase 0 records what the reference CLI does when run with
   `--skip-post-create`.
2. Whether the rootfs store should be content-addressed after
   extraction (hash the extracted tree) so two digests with identical
   contents share space. Cheap to add later; `image` tier's promise
   does not depend on it.
