## Context

Status: planning, revised 2026-11-17. Pixi is not installed on the machine
this was written on; **nothing below about pixi's behavior is measured.**
`Tier::Artifact` already ships for Swift. Pixi is refused today through
`NOT_YET_SUPPORTED` in `src/provider/validate.rs`.

### Evidence, by how it was obtained

**Read in the official documentation, 2026-11-17, not run:**

- [shell-hook](https://pixi.prefix.dev/latest/reference/cli/pixi/shell-hook/):
  `--json` emits "the environment variables set by running the activation".
  `--as-is` is shorthand for `--no-install` and `--frozen`. `--no-completions`
  and an experimental activation cache exist. The page does not say whether
  the non-JSON form executes activation scripts.
- [Configuration](https://pixi.prefix.dev/latest/reference/pixi_configuration/):
  `run-post-link-scripts` defaults to `false`; the opt-in value is
  `"insecure"`. The workspace's `.pixi/config.toml` outranks every user and
  system file and is **always merged**, including under `--no-config` /
  `PIXI_NO_CONFIG=1`, which skip only the user and system layers.
  `detached-environments` moves environments out of `.pixi/envs` to a
  directory named from the project name and a hash of its path. Cache root is
  `PIXI_CACHE_DIR`, then `RATTLER_CACHE_DIR`.

**Cited by the first draft, not re-read on 2026-11-17:** the
[install](https://pixi.prefix.dev/dev/reference/cli/pixi/install/) and
[manifest](https://pixi.prefix.dev/dev/reference/pixi_manifest/) references,
for `--locked`, `--environment` and activation composing across features and
targets. Both are moving development pages; task 0.1 verifies the options on
the pinned release.

**From memory, unverified, each a Group 0 item:** that the non-JSON hook only
emits `source` lines; that installing PyPI wheels queries the environment's
own interpreter by running it; that `pixi init --format pyproject` adds the
project as an editable path dependency; that `detached-environments` links
`.pixi/envs` in the project to the detached location.

The `new/` material supplies constraints, not evidence: recipe, content and
instance identity are distinct; a Darwin prefix is never a Linux guest
artifact; native tests claim nothing about VMs or fleet rebuilds.

### What the 2026-11-17 review changed

| Finding | Resolution |
| --- | --- |
| `/usr/bin` granted on Linux, against the draft's own rules | P5 |
| Host-side install may execute downloaded artifacts | P2: measured; PyPI admitted only if it does not |
| Store had no location, references or reclamation | P3 |
| `PackageManagerAuthority::WritableStore` is `/nix/store` only | P3: a provider-store overlap check beside it |
| Default pyproject layout refused, and the staging design could never admit it | P2: stated as a limitation with its real reason |
| Three-stage activation reimplemented pixi's composition | P4: pixi composes, devcroft rewrites one path prefix |
| "Tasks remain available if `pixi` is invoked manually" | P6: withdrawn |
| `ProviderEntry::from_name` does not exist | P7: the real seams |
| `run-post-link-scripts = true` | the value is `"insecure"` |
| Cross-provider refusal described as general | P6: it reads flox only today |

## Goals / Non-Goals

**Goals:** a locked Pixi workspace selected explicitly; useful binary
environments; confined activation; immutable shared prefixes; accurate
artifact-tier diagnostics on qualified native platforms.

**Non-Goals:** Windows; Pixi global environments; build backends, source or
editable installation; private channels and indexes that need credentials;
tasks as services; `pixi run` inside the sandbox; relocking; confined
materialization; VM, companion or fleet claims.

## Decisions

### P1. Explicit workspace and environment identity

```toml
[env]
provider = "pixi"

[env.pixi]
manifest = "pixi.toml"     # optional; relative file inside the project
environment = "default"    # optional; exact named environment
```

Without a selector, use the sole eligible root file: `pixi.toml`, or
`pyproject.toml` with `[tool.pixi]`. Both present and neither selected is a
refusal naming both. A `pyproject.toml` without `[tool.pixi]` is not Pixi. An
explicit selector must be a regular file inside the canonical project root;
traversal and symlink escape are refused. Unknown keys under `[env.pixi]` and
`[env.pixi]` under another provider are config errors. `default` means Pixi's
named default, never an ambient `PIXI_ENVIRONMENT_NAME`.

### P2. Strict, bounded materialization that runs nothing it downloaded

`pixi.lock` must exist before any pixi command and its format version must be
one the fixtures cover; an unknown version is a provider error naming it. The
selected environment needs a solution for the current platform. System
requirements recorded in the lock (`__glibc` and similar) are checked against
the host before installing. Installation uses `--locked`, not `--frozen`, so
a stale manifest fails. Manifest and lock are hashed before and after.

devcroft reads the lock itself and refuses, by package and reason, before any
installer runs: source distributions, editable, path and VCS dependencies,
and pixi build backends.

**A locked artifact is not automatically trusted to run on the host.**
Channels, URLs and hashes are in the repository, so the bytes are whatever
the project chose. Task 0.3 measures, with a sentinel interpreter, whether
`pixi install --locked` executes anything it downloaded:

- for a Conda-only environment, where the expectation is that it does not,
  post-link scripts being off by default;
- for an environment with PyPI wheels, where the expectation is that the
  environment's interpreter is run to learn its tags.

An environment shape that executes downloaded code on the host is **refused
by name** in this change. If that is every PyPI-bearing environment, then the
first slice is Conda-only and says so. Lifting it means running
materialization confined, which pixi permits in principle since it needs no
daemon; that is a change of its own (a materialization policy, a network
allowlist that survives redirects, a grant for the host's pixi binary) and is
not attempted here.

**The default `pyproject.toml` layout is refused for this reason, not for a
structural one.** Installing the project itself as an editable dependency
runs its build backend. The workspace is recognized as Pixi and the refusal
names the dependency and says why.

The subprocess environment is **built, not inherited**: `env_clear`, then an
isolated `HOME`, `capture::CANONICAL_PATH`, `PIXI_HOME` and `PIXI_CACHE_DIR`
inside devcroft's store, the switch that skips user and system configuration,
and the caller's proxy and CA variables, which are the only ambient values
that cross. No credential is forwarded, so a channel or index that needs one
fails at download with a provider error that says credentials are not
supported.

The project's `.pixi/config.toml` cannot be skipped, since pixi always merges
it. devcroft parses it and refuses any key outside a small measured allowlist
by dotted name. `run-post-link-scripts = "insecure"`, `tls-no-verify`,
`detached-environments` and an authentication override are refused outright.
What is admitted is copied into the staging workspace so pixi sees the same
thing.

**Staging.** `<object>/` holds a byte copy of the selected manifest, a byte
copy of `pixi.lock`, the admitted config, and, only if 0.4 finds pixi checks
that they exist, copies of the activation scripts. `pixi install --locked
--environment <env>` runs there, so the prefix is created at its final
absolute path, `<object>/.pixi/envs/<env>`, and nothing is relocated.

Alternative: install into the checkout's `.pixi/envs/<name>`. Rejected: two
sandboxes of one checkout would share a prefix, an ordinary `pixi install`
could replace it under a running sandbox, and a prefix inside the writable
project root cannot be made read-only, since
`check_no_deny_overlaps_allow` forbids a deny nested in an allow.

Alternative: `detached-environments`, with pixi run against the original
manifest so that paths are native. Not chosen, and recorded so it is not
re-proposed without measuring: from memory it links the project's
`.pixi/envs` to one detached location, which writes into the checkout and
gives two sandboxes one target.

### P3. Immutable objects, with the sandbox's own state as the reference

**Location.** `~/.local/share/devcroft-artifacts/pixi/<object-id>/`, with the
download cache at `~/.local/share/devcroft-artifacts/pixi/cache`. A sibling of
devcroft's data directory, which is baseline-denied and so could never be
granted from. Not a cache directory, since deleting it breaks live sandboxes.

**Identity.** A hash of the manifest bytes, the lock bytes, the admitted
config, the environment name, the target platform, the pixi version and the
adapter version. Objects derive only from hash-verified packages, so two
projects with the same recipe share one. Package digests are recorded in the
completion record, separately.

**Publication.** Under an exclusive per-object lock: if the directory exists
without a completion record, remove it entirely; install; verify manifest and
lock unchanged; write the record atomically. Readers require the record.

**Grants.** Sessions get `<object>/.pixi/envs/<env>` read-only, origin
`provider:pixi`. The cache and the rest of the staging directory are never
granted. Whether prefix files are hardlinks or clones of cache objects is
measured in 0.3 and does not change the guarantee: no sandbox has a writable
path to either name. Mutable state lives in the sandbox's own
`.devcroft/<name>/home`.

**A manifest write grant overlapping the store is refused** at layer
`config`, exit code 2. `policy::capability_set::package_manager_authority`
does this for `/nix/store` alone; the same check for provider artifact roots
is added beside it, on resolved grants.

**References and reclamation.** `Meta` records the object id; that is the
reference. `rm`, and `up --recreate` when it changes the id, scan every
recorded `Meta` as `state::recorded_allocations` does for ports and remove
each pixi object none names. `up` holds a shared store lock from acquiring
its object until its `Meta` is written; the sweep takes it exclusively.
`down` removes nothing. `doctor` reports the store's size. This follows
`DERIVED_ENV_PREFIX`, which `rm` already removes only when nothing else is
left, and replaces the draft's "no GC", under which multi-gigabyte prefixes
would have accumulated with no command to remove them.

### P4. Activation: pixi composes it, devcroft moves it, provisioning runs it

After installation, request the hook as text against the staging workspace
with the qualified equivalent of

```
pixi shell-hook --shell bash --as-is --no-completions \
    --environment <name> --manifest-path <object>/<manifest>
```

Task 0.1 must show this is inert: package `activate.d` sentinels and project
activation sentinels untouched, nothing installed, lock unchanged. `--json`
is never used; it works by running the activation. If the text form runs
anything at a candidate version, that version is not admitted and there is no
host-side fallback.

The project's `[activation]` stays in the staged manifest. The draft removed
the scripts and sourced them by hand afterwards, which meant reproducing
pixi's composition across features and targets, and getting it subtly wrong
would have been silent. Left in place, the emitted text already lists them in
pixi's own order.

What is wrong in that text is one prefix: paths that should name the project
name the staging directory. `<object>` is a path devcroft generated and
contains the object id, so the substitution is unambiguous: every occurrence
of `<object>` that is not followed by `/.pixi/` becomes the project root. That
moves `PIXI_PROJECT_ROOT`, `PIXI_PROJECT_MANIFEST` and every project script
path, and leaves `CONDA_PREFIX` and `PATH` on the prefix. Task 0.4 checks the
result against a fixture with feature-level and target-level scripts: order
preserved, no staging path left outside the prefix.

The activation script devcroft hands to `__provision` is then: `cd` to the
project root; the rewritten text; `unset PIXI_EXE`, since it names a host
binary the sandbox cannot reach. It runs once under the runtime filesystem
and `[provisioning.network]` only, and the resulting environment is captured
against `canonical_base_env`. A non-zero exit fails `up` at layer `provider`
with the existing provisioning attribution.

Activation scripts named by the manifest must be relative paths inside the
project; anything else is refused by key.

### P5. The shell and host runtime are the narrowest grant that works

Sessions need a POSIX shell and activation needs Bash. Resolve both from the
prefix first; conda-forge ships them and many environments pull one in.
Otherwise:

- **Linux:** grant the host's `sh` and `bash` as **files**, canonicalized,
  with the loader and libraries they need, origin `provider:pixi`.
  `shell::resolve` accepts a candidate inside a declared grant and a file
  grant equal to it qualifies. `Identity` writes `/bin/sh` into the view as
  today.
- **macOS:** grant `host_shell_dir()` (`/bin`, which holds both) and
  `/var/select`, as `swift.rs` does.

**`/usr/bin` is not granted on either platform.** On Linux
`host_shell_dir()` canonicalizes to `/usr/bin` and `CANONICAL_PATH` ends in
it, so the draft's grant would have put the host's compilers and tools on the
path of a provider that claims completeness. A project that needs `grep` adds
the conda package.

No directory grant covers `/bin/sh` or `/usr/bin/env` under this design, so
the draft's change to `sandbox_identity` is dropped. Its equality-only check
is a pre-existing observation, noted in task 5.3 to be filed separately.

**Host runtime.** Conda bundles most of a toolchain but binaries still use
the host's loader and libc. Each qualified profile carries a fixed inventory
rendered as `provider:pixi`. Requirements are checked by reading executables'
headers, never by running them; the reader is the one `add-mise-provider`
task 0.4 chooses, or is chosen here if this lands first. GPU drivers, devices
and system services are not admitted because a virtual-package requirement
mentions them.

The six criteria are measured on the supported subset: bounded inputs; cold
locked restore; shared objects no sandbox can write; inert host capture and
confined activation; a real Python and native-library workload and a compiler
build with the host's `cc` unresolvable; actionable preflight failures.

### P6. No services, and no `pixi run`

`PixiProvider` returns `ServiceSupport::Unsupported`. A task named `serve` or
`postgres` is not a service. `[network.services.<name>]` under pixi fails at
layer `config`, exit code 2, through the existing `check_declared_ports`.

The cross-provider refusal is the existing one and is not broadened here:
`ensure_no_services_declared_for_another_provider` fires because pixi reports
`Unsupported`, and it detects what it can read, which today is a flox
manifest's services (and mise daemons once `add-mise-daemons` lands). The
draft described it as covering any provider's manifest; it does not.

The draft also said tasks "remain available to the user inside the sandbox if
`pixi` is invoked manually". They do not: pixi is a host binary and is not
granted. If a project put pixi in its own environment, `pixi run` there would
try to build a second environment under the project's `.pixi/` with no
network. Tasks are therefore not runnable by name in the sandbox. `doctor`
and the sample say so; the user runs the task's command directly.

### P7. Complete fingerprints, and the seams that change

`fingerprint` covers the selected manifest, `pixi.lock`, the environment
name, the project's `.pixi/config.toml`, each project activation script, and
the adapter version. Files other than the manifest go through
`capture::optional_file_part`, so a deleted script changes the hash and
`status` reports `env: stale` rather than `unknown`.

Code that changes outside the new module:

- `src/provider/validate.rs`: `"pixi"` moves from `NOT_YET_SUPPORTED` to
  `SUPPORTED`.
- `src/config/{mod,validate}.rs`: the `[env.pixi]` sub-table and its
  provider-mismatch refusal.
- `src/provider/mod.rs`: `ProviderKind::Pixi` and its arms. `ProviderKind` is
  a `Copy` enum and `ProviderEntry` is a trait with no constructor; the
  selection therefore travels beside the kind. `Provider::resolve`,
  `manifest_fingerprint` and `is_stale` take only a name and a project root
  today and each gains the selection, so resolution and staleness cannot hash
  different environments.
- `src/lifecycle/state.rs`: `Meta` records the selection and the object id,
  which is what `status`, `policy --render` and `why` reconstruct from.
- `src/bin/devcroft.rs`: the staleness line names the active provider's
  manifest instead of hardcoding "flake" or "flox manifest".

`status` reports the environment, the tier and stale inputs. `doctor` checks
the pixi binary and version, the manifest and lock, the platform, the
packages that would be refused and the store's size, and installs nothing.
`init` reads TOML only. A minimum pixi version is enforced; a version newer
than the last qualified draws one warning. Existing error layers and exit
codes are kept.

## Risks / Trade-offs

- The first slice may be Conda-only → decided by 0.3, stated in the proposal,
  and reported to the owner before group 1 if so.
- No host userland → a pixi sandbox has what the environment declares plus a
  shell. `doctor` says so.
- The prefix is read-only → Python cannot write `__pycache__` beside
  installed modules and recompiles at import. A cost, not a failure.
- The path rewrite is textual → safe because the prefix rewritten is a
  devcroft-generated unique path; 0.4 checks for residue.
- Project config can change installation → parsed and allowlisted, since it
  cannot be skipped.

## Migration Plan

1. Group 0; stop or narrow as 0.6 says.
2. Config, selection and the provider seams.
3. Store and materialization.
4. Activation, shell and host-runtime grants, staleness.
5. Tests on native targets; publish supported and refused rows.
6. Update decisions and context only when the evidence exists.

Existing providers are unaffected. devcroft never reads, writes or deletes the
user's own `.pixi` directory. Rollback removes the provider selection; objects
are removed by the next `rm` that finds them unreferenced.

## Open Questions

All in Group 0:

- Is the text form of `shell-hook` inert at the pinned version?
- Does locked installation execute downloaded code, for Conda-only and for
  PyPI-bearing environments?
- Does the rewritten hook preserve order with no staging residue, and does
  pixi require activation scripts to exist when it emits it?
- Which lock fields distinguish binaries from everything refused?
- Which host files do `sh` and `bash` need per Linux profile?
