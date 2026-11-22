## Context

`docs/decisions.md` §1 rejected proto as "mise's class without mise's lockfile
maturity". That was true when written. The facts below are **documented**, read
on 2026-11-22. None of them is **measured**: that is group 0's job.

| Fact | Source |
|---|---|
| `.protolock` beside `.prototools`, enabled by `[settings] unstable-lockfile = true` | <https://moonrepo.dev/docs/proto/lockfile> |
| One record per tool, version, os/arch (and `libc` since 0.63, `variant` since 0.64): `spec`, `version`, `checksum` (`sha256:…`, or `gpg:<fpr>:sha256:…`), `backend`, `metadata` | same |
| No download URL in the record | same (not listed as a field) |
| "builds from source and native installations may not have a checksum" | same |
| `--immutable-lockfile` / `PROTO_IMMUTABLE_LOCKFILE`: resolves only from records; a missing record fails | same |
| A record for other platforms only now fails (0.63); the checksum-dropping bug #1128 is fixed in 0.63 | <https://moonrepo.dev/blog/proto-v0.63>, <https://github.com/moonrepo/proto/issues/1128> |
| `proto lock --platform …` records checksums without installing (0.64) and skips tools "that can only be verified by installing them" | <https://moonrepo.dev/blog/proto-v0.64> |
| Concurrent installs of one tool and version are serialized by a file lock (0.60) | <https://moonrepo.dev/blog/proto-v0.60> |
| `proto activate --json` prints an `env` object and a `paths` list | <https://moonrepo.dev/docs/proto/commands/activate> |
| `[env]` values are literals with `${VAR}` substitution; `file = ".env"` loads a dotenv file | <https://moonrepo.dev/docs/proto/config> |
| `[env]`, `[shell]`, `[plugins]`, `[backends.*]` and non-built-in pins apply only to trusted configs; CI trusts every config | same |
| Plugins are WASM, located by `https://`, `github://` or a registry, "downloaded and executed on your machine", cached 180 days by default | <https://moonrepo.dev/docs/proto/plugins> |
| Ruby falls back to building from source; `[settings.build] install-system-packages` defaults to `true` | <https://moonrepo.dev/blog/proto-v0.60>, config page |
| The lockfile is "currently unstable … the lockfile structure may change between releases" | lockfile page |

What the docs leave open, and the gate must answer: whether built-in plugins
are pinned by the proto release, whether any plugin calls a host-command
function during install, whether `activate --json` runs anything, whether the
Rust tool shells out to rustup, and whether installed tools are relocatable.

## Goals / Non-Goals

**Goals:**
- Re-decide the rejection by measurement against one pinned proto release.
- If admitted, the same contract as mise and pixi: locked, verified before any
  subprocess, nothing downloaded executed host-side, environment captured as
  data, immutable shared objects, attributed grants.

**Non-Goals:**
- moon (the build system) integration. `.moon/` is not read.
- asdf. Its rejection stands, and this change does not touch it.
- Tools proto installs by building from source or through a system package
  manager. They are refused, never degraded.
- Running `proto run` or proto shims inside the sandbox. proto itself is not
  in the sandbox, as with `mise run` and `pixi run`.

## Decisions

### P1. The gate can end in a rejection, and that is a valid outcome

proto overlaps mise. The change is only worth implementing if the narrow
profile covers what `.prototools` users actually pin (Node, Bun, Deno, Go,
Python) and demand exists. Otherwise the deliverable is a rewritten rejection
in `docs/decisions.md` naming the property that still fails.

### P2. Plugins are part of the lock, or there is no lock

The record has a checksum but no URL. The URL is derived at install time by a
WASM plugin, which proto itself downloads. A checksum check after download
bounds *what* is accepted, but not *where* proto connects or what the plugin
runs. Admission therefore requires one of:
- built-in plugins whose version is fixed by the pinned proto release, and
  verified as such (measured); or
- plugin locators pinned to an exact version with a checksum that devcroft
  checks before proto loads them.

Third-party plugins, registry locators and unpinned `github://` locators are
refused by locator. *Alternative considered:* trusting the checksum alone.
Rejected because a plugin is code, and the checksum covers the artifact, not
the code that chose it.

### P3. Derived configuration, allowlisted environment

As in mise M1/M2: devcroft writes a `.prototools` with `[tools]`, the admitted
`[plugins]` and `[settings] unstable-lockfile = true`, beside a byte copy of
`.protolock`. It runs `proto install --immutable-lockfile` with `env_clear`,
an empty `HOME`, `PROTO_HOME` inside the object and no inherited `PROTO_*`.
Trust is irrelevant, because the derived file holds nothing trust governs
except `[plugins]`, which devcroft has already pinned.

Every other `.prototools` key is classified: consumed (`[tools]`, `[env]`),
refused by key (`[plugins]` outside P2, `[backends.*]`, `[shell]`,
`[settings.build]`, `[env] file`), or not carried (`[settings]` keys that
affect only proto's own UX). Parent-directory and `~/.prototools` lookup is
switched off (variable or flag named by 0.1), or the subprocess runs where
none exists.

### P4. Capture as data, `[env]` interpreted by devcroft

`proto activate --json` returns `env` and `paths`. If 0.3 confirms it runs
nothing, devcroft takes `paths` and the tools' own variables from it. The
project's `[env]` literals are applied by devcroft over the provider's
environment, with `${VAR}` resolved against that environment. `file = ".env"`
is refused: a dotenv file is project data that devcroft's own `[env.vars]`
already covers.

### P5. Store, sharing and reclamation through `provider::artifacts`

Objects at `~/.local/share/devcroft-artifacts/proto/<id>/`, the id over the
derived configuration, the lock bytes, the plugin pins and the adapter
version. Lease, sweep and completion record are the existing shared code.
The write-grant overlap check already covers the artifact store root, so
nothing new is needed in `policy`.

### P6. Shell and host runtime as for mise

Artifact tier: the shell comes from the environment if it ships one, else from
the narrowest host grant (file grants plus loader on Linux, the shell's
directory on macOS, never `/usr/bin`). Host libraries come from the header
walk and carry origin `provider:proto`.

## Risks / Trade-offs

- [The lockfile format changes between releases] → one qualified minor
  release, a refusal below it, a warning above it, and lock parsing behind a
  version check.
- [Built-in plugins are themselves downloaded] → P2. If 0.2 finds they are not
  fixed by the proto release, the stop criterion applies.
- [Duplicates mise] → P1. Implementation waits for demand.
- [Tools are prefix-hardcoded under `PROTO_HOME`] → measured in 0.4. Objects
  are installed in place, never moved, as for mise.

## Open Questions

- Does a `github://` locator with a tag and a checksum exist upstream, or would
  devcroft have to vendor plugin WASM into the object?
- Is `PROTO_HOME` the only proto directory, or do cache and temp live
  elsewhere?
