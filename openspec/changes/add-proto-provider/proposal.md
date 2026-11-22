# Change: add-proto-provider

## Why

`docs/decisions.md` §1 rejects proto ("Rejected: asdf, proto") on criterion 2,
and names its own revisit condition: "a lockfile with pre-resolved URLs,
checksums, and a strict mode that forbids resolution at install time". proto
has since moved most of the way there. As documented on 2026-11-22 (v0.64,
2026-11-13), `.protolock` holds a record per tool, version and platform with
an algorithm-prefixed checksum. `proto install --immutable-lockfile` resolves
only from those records and fails when one is missing, including, since v0.63,
when only other platforms are recorded.

The decision says it should be revisited, not defended, once its reason stops
being true. This change does that revisit. It is **not scheduled**: like mise
and pixi, an artifact-tier provider is built only under demonstrated demand
(`docs/decisions.md` §1, "re-qualifies under demonstrated demand"), and only
for the profile that a measurement gate admits.

## What Changes

- Group 0 re-measures the rejection against a pinned proto release. The
  outcome is one of: admit a narrow profile, or keep the rejection with an
  updated reason. Both are complete outcomes for this change.
- If admitted: accept `env.provider = "proto"` as an **artifact-tier**
  provider, moving `"proto"` out of `VERSION_MANAGERS` in
  `src/provider/validate.rs`. asdf stays rejected.
- Require a `.protolock` record with a checksum for every selected tool on the
  current platform, verified by devcroft before any proto subprocess. Install
  only under `--immutable-lockfile`. A record without a checksum (source builds
  and "native installations", which proto documents as possibly having none)
  is refused by tool.
- Refuse, by tool and reason, every tool whose installation builds from source,
  invokes a system package manager (`[settings.build]
  install-system-packages`, default `true` upstream), or executes what it
  downloaded. Which tools that leaves is decided by measurement, not here.
- Pin the plugin set. proto resolves download URLs inside WASM plugins that it
  downloads and caches, and the lock records no URL and no plugin checksum.
  devcroft therefore admits only plugins it can pin, or the change stops.
- Host-side proto sees a devcroft-written configuration holding `[tools]` and
  the admitted plugins, under an allowlisted environment with `PROTO_HOME`
  inside a devcroft-owned object. proto's trust model is not relied on.
- Capture the environment as data through `proto activate --json`, if
  measurement confirms it runs nothing. Interpret `[env]` in devcroft and refuse
  `file = ...` dotenv loading and `[shell.aliases]`.
- Materialize into immutable objects under
  `~/.local/share/devcroft-artifacts/proto/`, as the third consumer of
  `provider::artifacts`.

## Capabilities

### New Capabilities

None. This extends the existing provider contract.

### Modified Capabilities

- `config`: accepts `proto` and an optional `[env.proto]`.
- `env-provider`: locked resolution under immutable mode, plugin pinning,
  hook-free capture, and refusal of source builds and system package
  installation.
- `policy`: attributed artifact and host-runtime grants, the narrow shell
  grant, and the artifact-store write-grant overlap rule extended to proto.

The main spec tree is empty, so these are provider-scoped **ADDED**
requirements, as in `add-mise-provider` and `add-pixi-provider`.

## Impact

- If admitted: `src/provider/{mod,validate,proto}.rs`, `src/provider/artifacts.rs`
  (a third consumer, no new contract), `src/bin/devcroft.rs` (`init`,
  `doctor`, `status`), `tests/proto_provider_e2e.rs`, `samples/proto-sample`.
- `docs/decisions.md` §1: "Rejected: asdf, proto" is rewritten either way.
  It becomes "Built: proto" with the admitted profile, or the rejection is
  kept with the property that still fails. The latter is no longer a missing
  lockfile; it is whichever of plugin pinning, missing checksums or host-side
  execution the gate finds.
- proto overlaps mise almost entirely. The case for it is projects already on
  moonrepo (`.prototools` committed). Without that demand the right outcome is
  the updated rejection, not a second provider for the same tier.
- proto's lockfile is marked `unstable-lockfile` upstream ("behavior and the
  lockfile structure may change between releases"). The qualified range is
  therefore one minor release, and a newer proto warns, as for pixi.

See [design.md](design.md) for what is documented, what must be measured, and
the stop criteria.
