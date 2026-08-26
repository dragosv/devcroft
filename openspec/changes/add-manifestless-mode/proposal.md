# Run Without a devcroft Manifest

**Depends on:** `sandbox-provisioning`. See "Risk" below — this mode is the most
exposed path in the tool and is the wrong thing to ship while provider
activation still runs unconfined on the host.

## Why

devcroft's cost of entry is not devcroft — it is Nix. Requiring a
`devcroft.toml` means a user must adopt an environment provider *and* write a
manifest before seeing anything work. Most people asked to do that on an
unfamiliar repository will not.

But most repositories that would benefit are already partway there: a
`flake.nix`, a `devbox.json`, a flox environment. Those are detectable. And a
repository with nothing at all can still be served if the user supplies the
toolchain on the command line.

This turns the first experience from "migrate your environment" into "try it on
this repository", and it is the same capability the fleet case needs for
external pull requests, dependency updates, and repositories an agent fetched
itself — none of which will contain a devcroft manifest.

## What Changes

- **MODIFIED** `env-provider`: environment configuration resolves through an
  explicit fallback order rather than requiring a manifest.
- Provider auto-detection from signature files, with fixed precedence and the
  choice reported.
- `--provider` to select explicitly, overriding detection.
- `--with` to supply packages where a repository has no provider file at all.
- An ad-hoc invocation that runs a command in a sandboxed environment without
  writing anything to the repository.
- A stricter default policy for this mode than for the manifest path.

## Impact

- Affected specs: `env-provider`
- Introduces the resolution order that error messages across the tool will cite.
- Gives the README a first example that does not begin with installing a
  provider.

## Risk

**This is the most exposed entry point in the tool.** It exists to be pointed at
repositories nobody has read, and evaluating a `flake.nix` or activating a flox
environment executes code from that repository. Until `sandbox-provisioning`
lands, that happens on the host, outside any boundary — so shipping this first
would take the tool's weakest path and aim it at its least trusted input.

Ordering is not a detail here. Provisioning first, then this.

## Non-Goals

- Guessing an environment when nothing is detected and nothing is supplied.
- Matching manifest-mode reproducibility without a lock. See `design.md`, M4.
