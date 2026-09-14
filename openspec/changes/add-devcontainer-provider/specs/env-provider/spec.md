## ADDED Requirements

### Requirement: devcontainer provider resolution
The system SHALL resolve `env.provider = "devcontainer"` from the
project's `.devcontainer/devcontainer.json` (or `.devcontainer.json`)
by materializing its `image` into a read-only root filesystem on the
host, once per image digest, and treating that filesystem as the
provider's grant. Materialization SHALL be done in-process — the
manifest, config and layers fetched from the registry by digest and the
layers unpacked by devcroft with OCI whiteouts honoured — and SHALL
require no container runtime, daemon, or socket on the host. Nothing
from the image SHALL run on the host: no entrypoint, no command, no
lifecycle hook. The environment SHALL be taken from the image's
configuration as data (`Env`; `WorkingDir` ignored), never by executing
anything inside the image.

#### Scenario: A digest-pinned image resolves without running anything
- **WHEN** `devcontainer.json` declares `"image": "<ref>@sha256:<digest>"`
  and `up` runs on Linux with the registry reachable
- **THEN** the image's filesystem is materialized under the rootfs store
  keyed by that digest, read-only
- **AND** the resolution's grants name that directory with origin
  `provider:devcontainer`, and nothing else outside the project
- **AND** no process from the image ran on the host (asserted by an
  image whose entrypoint writes a marker: the marker is absent)

#### Scenario: A second sandbox on the same digest shares the rootfs
- **WHEN** two projects, or two worktrees, name the same image digest
- **THEN** the second `up` materializes nothing and is granted the same
  directory

#### Scenario: A tag without a digest is recorded, not trusted
- **WHEN** `image` names a tag with no digest and no lock exists
- **THEN** the first `up` resolves the tag to a digest, records it in
  `.devcontainer/devcroft.lock`, and materializes that digest
- **AND** every later `up` uses the recorded digest until `--recreate`
  re-resolves, so the tag moving does not change the sandbox silently

#### Scenario: Not on Linux
- **WHEN** `up` runs on macOS for a `devcontainer` project
- **THEN** it fails at layer `provider` with exit code 3, saying the
  provider needs a Linux host and naming the VM change as the macOS route

#### Scenario: The registry is unreachable
- **WHEN** the digest is not materialized and the registry cannot be
  reached
- **THEN** `up` fails at layer `provider` naming the registry and that
  the image was never pulled, and a digest already materialized needs no
  network at all

#### Scenario: Whiteouts are honoured
- **WHEN** an image's upper layer deletes a file or directory a lower
  layer created
- **THEN** the unpacked rootfs does not contain it, and the unpacked
  tree matches what a container runtime would produce for the same
  digest

### Requirement: The rootfs is the session's root and the keeper's grant
The system SHALL run each session of a `devcontainer` sandbox with the
materialized rootfs as its root filesystem, with the project root, a
private temporary directory, `/proc`, `/dev` and the sandbox's own
control sockets bound into it. The keeper SHALL keep the host-shaped
view it has for every other provider and SHALL be granted the rootfs
directory read-only as an ordinary provider grant; the session's view
SHALL add no grant the keeper does not hold. Sessions SHALL resolve their
shell out of the rootfs.

#### Scenario: An image binary runs, a host binary does not
- **WHEN** a session runs a binary present in the image at `/usr/bin`
- **THEN** it executes, linked against the image's own libraries
- **AND** the host's `/usr/bin` is not visible to the session

#### Scenario: The keeper is unaffected by the image's libc
- **WHEN** the image ships a libc the host's `devcroft` binary was not
  built against (musl, or an older glibc)
- **THEN** `up` succeeds, sessions run, and the keeper runs on the host's
  own libraries

#### Scenario: The policy is inspectable
- **WHEN** `policy --render` runs after `up`
- **THEN** the rootfs directory appears under `filesystem.read` with
  origin `provider:devcontainer`, by its real host path

### Requirement: The image tier
The system SHALL report `devcontainer` sandboxes as tier `image` — an
identical filesystem for an identical digest, on any Linux host, with no
guarantee about how the digest was produced — at `up` and in `status`,
and SHALL NOT describe it as `closure` or `artifact`.

#### Scenario: The tier is stated
- **WHEN** a `devcontainer` sandbox comes up
- **THEN** `up` prints the guarantee line naming `image` and its
  definition, and `status` names it thereafter

### Requirement: devcontainer fields honored and refused by name
The system SHALL honor `image`, `postCreateCommand` and
`postStartCommand`; ignore fields that have no meaning without a
container or that belong to the editor (`name`, `forwardPorts`,
`customizations`, `updateRemoteUserUID`, `overrideCommand`,
`shutdownAction`, `waitFor`); ignore unknown fields; and SHALL refuse at
layer `provider`, each with its own message naming the field: `build`,
`features`, `postAttachCommand`, `initializeCommand`, `remoteUser`,
`containerUser`, `mounts`, `runArgs`, `privileged`, `capAdd`,
`securityOpt`, `dockerComposeFile`. An image whose `Config.User` is
neither absent nor root SHALL be refused for the same reason
`remoteUser` is.

#### Scenario: `build` and `features` are refused with the prebuild route
- **WHEN** `devcontainer.json` has `build`, or `image` with `features`
- **THEN** `up` fails naming the field, that devcroft does not build
  images, and the route — build in CI, push, pin `image@sha256:…`
  (`devcontainer build --push` does it with features applied) — and
  that the image alone would resolve if the field were removed

#### Scenario: Unknown fields do not fail
- **WHEN** the file carries a key devcroft does not know
- **THEN** resolution proceeds and the key is not mentioned

#### Scenario: `postAttachCommand` and `initializeCommand` stay refused
- **WHEN** the file carries either
- **THEN** `up` fails naming the field and the reason `docs/decisions.md`
  §2 already records for it

#### Scenario: `dockerComposeFile` is refused in this version, with the route named
- **WHEN** the file carries `dockerComposeFile`
- **THEN** `up` fails naming the field, saying that compose-based dev
  containers are not supported in this version, that the `service:` the
  file points at would be the sandbox's image and its sibling services
  would run as supervised services in a later version, and naming the
  provider's `[services]` as the interim route for a database or cache

### Requirement: Lifecycle commands run inside the boundary as hooks
The system SHALL run `postCreateCommand` as the sandbox's `post_create`
hook and `postStartCommand` as its `post_start` hook — inside the
boundary, under the manifest's policy, once per creation and once per
start respectively — and SHALL NOT run either on the host or at
materialization. String, array and object forms SHALL be accepted; the
object form's entries run in key order. A command's failure SHALL fail
`up` at layer `keeper` as any hook's does, and a failure caused by
writing into the rootfs SHALL say that the rootfs is shared and
read-only and that `features` is where system-level installs belong.

#### Scenario: A project-local install works
- **WHEN** `postCreateCommand` is `npm install` and the manifest allows
  the registry's host
- **THEN** it runs once inside the sandbox after the first `up`, writes
  under the project root, and does not run again on a plain `up`

#### Scenario: A system-level install fails with the reason
- **WHEN** `postCreateCommand` is `sudo apt-get install -y jq` or
  anything writing under `/usr`
- **THEN** `up` fails at layer `keeper`, the message names the rootfs as
  shared and read-only and names `features`, and no other sandbox on
  the digest is affected

#### Scenario: Network without an allowlist
- **WHEN** `postCreateCommand` reaches a host the manifest does not allow
- **THEN** it fails as any hook does, and the message names
  `network.allow`

### Requirement: devcontainer staleness
The system SHALL report the sandbox stale when the hash of
`devcontainer.json` or the recorded digest differs from what was
recorded at the last `up`, and `up --recreate` SHALL re-resolve and
materialize a changed digest. Materialized rootfs directories SHALL NOT
be removed by `rm` or `--recreate`.

#### Scenario: The lock's digest changes
- **WHEN** the recorded digest is edited or re-resolved to a new one
- **THEN** `status` reports stale and suggests `up --recreate`

## MODIFIED Requirements

### Requirement: Only declarative providers
The system SHALL reject any provider value that does not name a supported
declarative environment provider. Supported values are `flox`, `nix`
(with `flake` and `flakes` accepted as aliases normalized to `nix`),
`devbox`, `devenv`, and `devcontainer` (Linux only). The rejection
message SHALL distinguish "not yet supported" (mise, pixi, hermit —
qualified but unscheduled) from "out of scope by design" (`host`, `none`
— devcroft has no non-reproducible mode) and from "fails the
qualification test" (version managers).

#### Scenario: Passthrough rejected
- **WHEN** the manifest declares `provider = "host"`
- **THEN** validation fails with exit code 2, layer `config`, and a message
  that devcroft has no non-reproducible mode

#### Scenario: Planned provider rejected
- **WHEN** the manifest declares `provider = "mise"`
- **THEN** validation fails with exit code 2 and a message that mise support
  is planned (artifact tier) but not yet implemented

#### Scenario: nix accepted
- **WHEN** the manifest declares `provider = "nix"`, `"flake"` or `"flakes"`
- **THEN** validation succeeds and the provider is `nix`

#### Scenario: devcontainer accepted
- **WHEN** the manifest declares `provider = "devcontainer"`
- **THEN** validation succeeds and the provider is `devcontainer`; the
  platform check is the provider's at `up`, not the parser's
