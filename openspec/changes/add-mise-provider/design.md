## Context

Status: implemented 2026-11-22 for the qualified HTTP profile (planning
revised 2026-11-17). `tests/mise_provider_e2e.rs` passed on macOS ARM64
against the pinned mise 2026.11.9 (executable SHA-256 as recorded below):

- cold locked restore with the inputs byte-identical;
- an interrupted object removed and reinstalled;
- two sandboxes sharing one object, the first `rm` keeping it and the
  last removing it with its lock file;
- the install read-only, and host `cc`/`gcc`/`ld`/`git` refused;
- staleness on a newly consumed dotenv;
- an overlapping write grant refused at `config`.

The Linux ARM64 runtime rests on the group 0 evidence and was not re-run
for this commit.

`ProviderKind`, the `ProviderEntry` trait, `Resolution`, `Tier::Artifact` and
confined `__provision` already exist. mise is currently refused through
`NOT_YET_SUPPORTED` in `src/provider/validate.rs`. The removed sketch assumed
host libraries were baseline-readable; they are not (`own-policy-baseline`).

### Evidence, by how it was obtained

Each statement below is tagged, because the first draft mixed documentation
with assumption and the review could not tell them apart.

**Measured locally, 2026-11-17, mise 2026.7.5 macos-arm64** (older than the
2026.11.9 the docs describe; re-measure on the pinned version):

- `mise install --help`: tools install to
  `~/.local/share/mise/installs/<TOOL>/<VERSION>` by default. The install
  location follows mise's data directory, which the first draft's isolation
  list did not set.
- `--locked` is a global flag: "Require lockfile URLs to be present during
  installation"; also `MISE_LOCKED=1` or `settings.locked = true`.
- `mise registry`: `process-compose` resolves to
  `aqua:F1bonacc1/process-compose`. There is no entry named `bash`, `dash` or
  `busybox`, so a POSIX shell is not something a mise project can normally
  declare. `coreutils` resolves to `aqua:uutils/coreutils`.
- `mise daemons` does not exist in 2026.7.5.

**Read in the official documentation, 2026-11-17, not run:**

- [Lockfile](https://mise.jdx.dev/dev-tools/mise-lock.html): each config file
  pairs with the lockfile beside it (`mise.toml` and `mise.lock`,
  `mise.<env>.toml` and `mise.<env>.lock`, `mise.local.toml` and
  `mise.local.lock`). Locked mode fails when an entry is missing, requires a
  URL for the current platform on backends that record one, and requires a
  checksum for each URL unless `MISE_LOCKED_REQUIRE_CHECKSUM=0`. Backends that
  record URL and checksum per platform: aqua, github, gitlab, forgejo, http,
  s3, conda, and core tools that download archives. asdf and vfox backend
  plugins record a version only; cargo, gem, dotnet and `core:rust` record a
  top-level version; npm and pypi record a version plus a dependency graph in
  sidecar files under `.mise/locks/`. `mise install` updates `mise.lock`.
- [Environment output](https://mise.jdx.dev/cli/env.html),
  [config discovery](https://mise.jdx.dev/configuration),
  [trust](https://mise.jdx.dev/faq.html): JSON output is an interface, not
  proof of inert evaluation; configuration can execute code and inherits
  parent and global settings.

**From memory, unverified, each one a Group 0 item:** the full list of
project-level config locations mise loads; the variable that stops its
parent-directory walk; the per-language default-package files it reads from
`HOME`; which core backends execute the tool they just installed.

### Qualification run, 2026-11-17 — partial group 0

**Measured on macos-arm64, candidate mise 2026.11.9.** This pins a candidate
for further qualification, not a minimum supported release. The user's installed
2026.7.5 binary was not replaced. The candidate and its source were downloaded
to `/private/tmp` from the official `jdx/mise` release/tag URLs.

Recorded SHA-256 identities (not a signature/provenance attestation):

- Executable: `f3bb0e8cbe0217a0b52444ecc0042364563ddd1de0490313bb7e702ecab41a14`.
- Release archive `mise-v2026.11.9-macos-arm64.tar.gz`:
  `e4509f159e0492e89065edcf52f2918e2751e3574620586058468ba0563aed59`.
- Source archive for tag `v2026.11.9`:
  `0dd2fd42fddc636fae0ce8cb1344e0beaee1cbbd8c568cf35d6e0654335b4b7c`.

Reproduction: `python3 scripts/qualify-mise-install.py /absolute/path/to/mise
--output result.json` from the repository root. The script constructs its
subprocess environment from scratch, uses isolated temporary HOME/data/config/
state/cache/system directories, and serves synthetic executable archives over
loopback. No real language runtime is installed by the final probe. All seven
installations exit 0. See
[the recorded result](evidence/install-macos-arm64-2026.11.9.json).

- `core:node` executes `node -v` and `npm -v` from the downloaded archive.
- `core:go` executes `go version` from the downloaded archive.
- `core:python` executes `python --version` from the downloaded archive.
- `http:sentinel`, `github:sentinel/sentinel`, `aqua:junegunn/fzf` and
  `conda:sentinel` install without executing marker-writing payloads in these
  fixtures. The Conda package also contains a post-link sentinel; it is not run.
  These fixtures do not qualify every backend option or registry definition.
- The manifest and lock remain byte-identical in all seven cases. These are
  supplied synthetic locks, not proof of normal lock generation on four targets.

Node's GPG check/npm shim and Python's attestation verification are disabled
**only in the synthetic fixture**, as the archive is not an authentic upstream
release. This is not a proposed production verification policy. Go uses a local
download mirror matching the supplied lock; a preliminary probe showed that
core:go constructs its download URL from the mirror even in locked mode, rather
than treating an arbitrary supplied URL as its download location. That preliminary
attempt failed checksum validation and is not counted as a successful sentinel
probe. Python's fixture includes the minimum sysconfig metadata required to
reach the version check; a failed extraction is never counted as safe installation.

**Source-derived at the same tag:** `src/plugins/core/node.rs` calls `test_node`
and `test_npm` unconditionally after extraction; `src/plugins/core/go.rs` calls
`verify` → `test_go`; `src/plugins/core/python.rs` calls `test_python`. No guard
disabling these version checks was found in those install functions. Consequently
the three core backends cannot currently enter the admitted set under M2.
This does not rule out installing those runtimes through another backend.

Directory controls confirmed in `crates/mise-util/src/env.rs`:
`MISE_DATA_DIR`, `MISE_INSTALLS_DIR` (defaults to data/installs),
`MISE_CONFIG_DIR`, `MISE_STATE_DIR`, `MISE_CACHE_DIR`, `MISE_TMP_DIR`,
`MISE_SYSTEM_CONFIG_DIR` (legacy fallback `MISE_SYSTEM_DIR`),
`MISE_SYSTEM_DATA_DIR`, `MISE_SYSTEM_INSTALLS_DIR`;
`MISE_GLOBAL_CONFIG_FILE` (legacy fallback `MISE_CONFIG_FILE`) and
`MISE_SYSTEM_CONFIG_FILE`. `MISE_TRUSTED_CONFIG_PATHS` is declared in
`settings.toml`; `MISE_CEILING_PATHS` stops parent discovery before its named
directory. M2 must set overrides explicitly or ensure their inherited values
are absent. The probe uses the parent of the derived directory as the ceiling.
Per-language default-package files and all possible HOME reads have not yet
been exhaustively measured; task 0.1 remains open.

**Execution capabilities checked:** Docker's configured socket
`~/.orbstack/run/docker.sock` is absent; `docker info` cannot reach a daemon.
`limactl list --json` reports no instances. No Linux execution results are
claimed. A usable Linux environment, real-runtime restores, lock-generation coverage
and the narrow-policy real builds remain required by 0.1–0.5. Group 1 is not
started. Only the source-derived discovery inventory (0.3) is complete.

### Reading `new/`

### Qualification completed for the initial HTTP profile, 2026-11-20

The first admitted backend is **HTTP**, with binary archives, literal URLs,
standard extraction and no executable installation options. Core Node/Go/Python
are refused because of the measured host execution above. GitHub/aqua/conda have
successful synthetic controls, but are not admitted without real per-option
lock/runtime qualification. Task 0.2 is scoped to the admitted set accordingly;
this does not claim their synthetic input locks were generated by `mise lock`.

- [Linux sentinel results](evidence/install-linux-arm64-2026.11.9.json) reproduce
  all seven macOS cases. [Linux file-access tracing](evidence/install-linux-arm64-traced-2026.11.9.json)
  records unique paths beneath the isolated HOME during each install, including
  `.netrc`, skill discovery, mise config discovery, and the core language default
  package files. These are observed accesses under the specified environment,
  not an exhaustive prediction for unsupported settings or every macOS syscall.
- [macOS ARM64 runtime](evidence/runtime-macos-arm64-2026.11.9.json) and
  [Linux ARM64 runtime](evidence/runtime-linux-arm64-2026.11.9.json) each restore
  upstream Node 24.0.0 and Go 1.24.0 through HTTP, checking the generated lock
  digests against Node's published SHASUMS and Go's download metadata.
  `.mise.toml` pairs with `mise.lock`; the generated format is version 4.
  `install --locked` and environment capture leave input bytes unchanged.
- [Linux x64](evidence/runtime-linux-x64-2026.11.9.json) and
  [macOS x64](evidence/runtime-macos-x64-2026.11.9.json) exercise lock generation
  and upstream checksum comparison only. They are not qualified runtime rows.
  HTTP entries carry version/backend/specifiers and per-platform URL/checksum;
  no `.mise/locks` sidecar is created by these profiles.
- `examples/mise_qualification.rs` invokes the actual policy compiler and
  `nono::Sandbox::apply_auto`. Both native runs print `node-runtime-ok`,
  `mise-build-ok`, `policy-probe-ok`; all four absolute host commands
  (`cc`, `gcc`, `ld`, `git`) fail with a kernel denial and are absent from the
  filtered PATH. A write to the installed Node binary is denied.
- Linux grants the canonical shell **file** `/usr/bin/dash`; macOS grants `/bin`
  and `/var/select`. Linux Node additionally needs `/etc/ssl/openssl.cnf`.
  No host program directory is added on Linux and `/usr/bin` is never added.
  HTTP extraction defaults to installations, not its optional shared-extraction
  cache. The provider must refuse that option and discard ambient PATH tails.
- Initial probes under `/tmp` correctly exposed the baseline's scratch write
  grant and are **not** read-only-store evidence. Final probes put installations
  outside `/tmp`, in a sibling of the writable test project, and pass. The
  production artifact root also lies outside baseline scratch grants.
- Linux ran in the existing OrbStack devcontainer, kernel
  `7.2.12-orbstack-00394-gbecdd6130068`, glibc 2.36, aarch64. The helper linked
  existing artifacts from checkout `663eee4cf35b1b0d4e291e07297bc3ef5c9df26a`.
  Its `capability_set.rs` is byte-identical to this checkout; `policy/mod.rs`
  differs only by additional credential-broker metadata, not grants/enforcement.
  Helper SHA-256: `ef45e1fe8e1327bfd8c0c7c6718ded0ae4bf968ba8cde1aad6dca9e306cc320f`.
  macOS builds the helper from this checkout.
- Building the macOS helper exposed pre-existing unconditional references to
  Linux init in `lifecycle/up.rs`. They are now gated on Linux, including an
  explicit unsupported-platform error for a requested private procfs. Unused
  Linux-only reexports/identity renderers are gated as well; Linux behavior is
  unchanged. This is a prerequisite build repair, not mise provider behavior.

**Header-reader decision (0.4):** use `object` 0.37.3 with minimal read/ELF/Mach-O
features. It is already in Cargo.lock as a development dependency of brush-shell;
making it a normal dependency adds **one normal crate**, no new lock packages,
and uses the existing memchr. Avoid a new handwritten binary parser.

The stop criterion passes for the two native ARM64 profiles: two real runtimes,
one real compiler build and the narrow host-toolchain denial controls on each OS.
Support is not extrapolated to x64 or musl from these runs. Group 1 can proceed.

### Reading `new/` (continued)

Carried here so the plan stays readable while `new/` is untracked:

| Material | Consequence for this provider |
| --- | --- |
| `GOAL-NONO-LIBKRUN-WORKLOAD-COMPANION-2026-11-12.md` | Preparation, workload and companion are distinct roles. |
| `INVESTIGATIE-NIX-ASTERINAS-VM-WORKLOAD-2026-11-11.md` | Host Darwin artifacts cannot qualify a Linux guest. |
| `PARTAJARE-ARTEFACTE-NIX-LOCAL-FLEET-2026-11-11.md` | Share verified immutable objects; keep writes private. Recipe, bytes and instance identity are different things. |
| `REVIEW-ADVERSARIAL-PACHET-NEW-2026-11-13.md` | Claims need provenance; do not borrow unverified numbers. |
| `REVIEW-COMBINAT-…-2026-11-10.md`, `POZITIONARE-…-2026-11-12.md` | Qualify finite profiles; avoid universal ecosystem claims. |

Nothing in `new/` is imported as code or as an active instruction.

### What the 2026-11-17 review changed

| Finding | Resolution |
| --- | --- |
| `/usr/bin` granted on Linux, against the draft's own rules | M5: shell file plus loader and libraries on Linux, shell directory on macOS, no `/usr/bin` anywhere |
| Host-side install may execute downloaded artifacts | M2: measured per backend with a sentinel binary; a backend that does is refused |
| Store had no location, no references, no reclamation | M4 |
| `PackageManagerAuthority::WritableStore` is `/nix/store` only | M4: a provider-store overlap check is added beside it |
| Derived config moved `{{config_root}}` and relative paths | M3: mise never evaluates the project's `[env]` |
| Isolation list omitted the install directory, `HOME` and ambient `MISE_*` | M2: allowlisted environment |
| Discovery list incomplete; `[settings]`, `[hooks]`, `[tasks]` unclassified | M1 |
| Services | moved to `add-mise-daemons` |
| Wrong symbol names | corrected throughout: `NOT_YET_SUPPORTED`, `capture::canonical_base_env`, `ProviderKind::from_name`, `policy::capability_set` |

## Goals / Non-Goals

**Goals:** useful locked mise environments; explicit artifact guarantees; no
project code and no downloaded artifact executed on the host; inspectable,
narrow runtime grants; finite compatibility evidence.

**Non-Goals:** arbitrary backends or plugins; source builds; services (see
`add-mise-daemons`); making `mise run` work inside the sandbox; confined
materialization; cross-platform equivalence; automatic trust; VM, companion or
fleet claims.

## Decisions

### M1. Bounded inputs, and every table classified

The input is one root manifest, `mise.toml` or `.mise.toml`, with the lockfile
mise pairs with it and the sidecar locks under `.mise/locks/`. Two competing
root manifests are refused.

Every other project-level location the pinned mise would load is refused **by
path**, because the derived configuration (M2) hides it from mise and it would
otherwise vanish without a word. The exact source-derived order at the candidate
pin is recorded in [discovery-2026.11.9.json](evidence/discovery-2026.11.9.json):
14 shared patterns, 10 local-override patterns, environment-specific expansions,
and the early `.miserc.local.toml` / `.miserc.toml` / `.config/miserc.toml` walk.
Shared configuration is followed by environment configuration, local overrides,
then environment-local overrides. The inventory includes all 35 registry tool
entries declaring idiomatic version files. These are off by default and only
become discovery inputs when enabled per tool. Ordinary `package.json`, `go.mod`
or similar application files must not be refused merely because an upstream
registry entry could read them under a different setting. Custom filenames and
plugin discovery are excluded by refusing their project configuration and
isolating ambient/global/system settings. The JSON records source hashes; this
inventory is not a filesystem trace.

Top-level tables of the selected manifest:

| Table | Treatment |
| --- | --- |
| `[tools]` | Consumed. Each tool must name an admitted backend. Tool options are an allowlist per backend; any other option is refused by dotted key. |
| `[env]` | Consumed by devcroft, never by mise (M3). |
| `[settings]` | `experimental`, `lockfile` and `locked` are read. **Any other key present is refused by dotted key whatever its value**, since `false` is a declaration here (`aqua.cosign = false`, `node.compile = true`). |
| `min_version` | Checked against the mise in use; a failure is a provider error. |
| `[tasks]`, `[task_config]`, `[vars]` | Not carried and not refused. Tasks are commands for `mise run`, which is absent from the sandbox. Stated by `doctor` and the docs. `[vars]` has no consumer once templates are refused (M3). |
| `[hooks]`, `[watch_files]` | Refused by key. Install hooks cannot be moved; `enter`, `cd` and `leave` fire only under `mise activate`, which devcroft never runs. |
| `[plugins]`, `[alias]`, `[tool_alias]` | Refused by key: they change what a tool name means. |
| `[daemons]` | Refused by name until `add-mise-daemons` lands. Not refused under `up --skip-hooks`, matching the existing rule that the escape hatch does not fail over services that would not have started. |
| anything else | Refused by name. |

Qualification maps each admitted backend to its lock fields, download and
verification behavior, install side effects and runtime library needs. The six
criteria of `docs/decisions.md` §1 are measured on that subset:

| Criterion | Acceptance evidence |
| --- | --- |
| Declarative definition | All consumed config is parsed as data and fingerprinted. |
| Restorable lock | Cold install uses exact platform artifacts and checksums; manifest and locks stay byte-identical. |
| Immutable-capable sharing | Two sandboxes execute one object, neither can modify it, removing one keeps the other. |
| Capturable activation | Host-side sentinels untouched; confined exports reach sessions. |
| Completeness | A real build with a compiler and linker **declared in `[tools]`**, with `cc`, `gcc` and `ld` from the host unresolvable inside the sandbox. |
| Checkable preconditions | Platform, backend, runtime libraries and locks fail before a keeper starts. |

A failed probe gates that profile, not the change. The stop criterion is in
task 0.5.

### M2. Host-side mise sees tools only, under an allowlisted environment

devcroft parses the manifest and lock itself, verifies every selected tool has
an admitted backend and a lock entry with URL and checksum for the current
platform, and only then writes a derived directory holding:

- `mise.toml` containing `[tools]` with the admitted options, plus the settings
  devcroft itself sets (`locked`, `lockfile`);
- a byte copy of the lockfile under the name mise pairs with that file;
- a byte copy of the `.mise/locks/` sidecars the lock references.

Nothing else from the project reaches mise. There is no `[env]`, no template,
no hook and no task in that file, so there is nothing for mise to evaluate.

The subprocess environment is **built, not inherited**: `env_clear`, then

- `HOME` pointed at an empty directory devcroft owns. `capture::canonical_base_env`
  keeps the real `HOME` for the Nix-based providers; this provider must not,
  because mise reads per-language default-package files from it and installs
  what they list;
- `PATH` set to `capture::CANONICAL_PATH`;
- mise's data, config, state, cache and system directories and its global
  config file, each pointed inside the object or at an empty file. The data
  directory is the one that decides where installs land;
- the trusted-config setting naming the derived file and nothing else;
- the setting that stops mise's parent-directory walk at the derived directory;
- locked mode on;
- proxy and CA variables (`HTTPS_PROXY`, `HTTP_PROXY`, `NO_PROXY`,
  `SSL_CERT_FILE`) forwarded from the caller. They are the only ambient values
  that cross, and they are not part of object identity, since every download
  is checksum-verified.

No ambient `MISE_*` variable is inherited, so a caller's `MISE_ENV` or
`MISE_LOCKED_REQUIRE_CHECKSUM=0` cannot change the result. No token is
forwarded; a rate-limit failure is a provider error that says so.

`mise install` and `mise env --json` run only against that directory. `mise
trust` and `MISE_YES` are never used.

**A locked artifact is not automatically trusted to run on the host.** The URL
and checksum are in the repository, so the bytes are whatever the project
chose. A backend is admitted only if installing through it executes nothing it
downloaded. Task 0.1 measures this with a sentinel binary per candidate
backend: an archive whose executables write a marker when run. A backend that
runs one, including a post-install step such as creating a virtualenv or
enabling a package-manager shim, is refused until that step can be switched
off in the derived configuration, in which case the switch is part of the
admitted profile.

Alternative considered: run materialization confined, since mise needs no
daemon and so, unlike the Nix-based providers, could be given a policy.
Recorded as the way to lift a refusal later. Not taken here: it needs a
materialization policy, a network allowlist that survives download redirects
and a grant for the host's mise binary, which is a change of its own.

### M3. The project's `[env]` is devcroft's to interpret

mise resolves `{{config_root}}`, relative `_.path` entries, `_.file` and
`_.source` against the directory of the config file it was handed. Handing it a
derived file elsewhere would point all of them at the store. The pixi sibling
has the same problem with `PIXI_PROJECT_ROOT` and solves it by rewriting; here
the cleaner cut is that mise never sees `[env]` at all.

devcroft turns `[env]` into one activation script, in declaration order, and
runs it once in `__provision` with the working directory at the project root,
after the tool environment from `mise env --json` is applied:

| Directive | Rendering |
| --- | --- |
| literal string, number, boolean | `export NAME='value'`, single-quoted |
| `false` | `unset NAME` |
| `_.path` | prepended to `PATH`; a relative entry resolves against the manifest's directory |
| `_.file` | a dotenv file parsed by devcroft as data and emitted as exports; other formats refused |
| `_.source` | `. '<path>'`, path resolved against the manifest's directory and required to stay inside the project |
| `{{config_root}}` in a value | replaced by devcroft with the manifest's directory |
| any other `{{ }}` or `{% %}` | refused by key |
| table form `{ value = … }` | `value` honored; `tools = true` is a no-op since the tool environment is always applied first; any other field refused by dotted key |
| `_.python.venv` and other plugin directives | refused by key |

Declaration order is preserved because mise applies these in order and a
literal declared after a `_.source` overrides what the script exported.

The script runs under the runtime filesystem and `[provisioning.network]`
only, like flox's hook and devenv's `enterShell`. Its exports and unsets are
captured against `canonical_base_env`. A failure fails `up` at layer
`provider` with the existing provisioning attribution.

devcroft does not synthesize mise's own variables (`MISE_PROJECT_ROOT` and the
like) and does not call `mise activate`, `mise exec` or `mise run`, per session
or otherwise. `[env.vars]` in `devcroft.toml` and explicit forwarding keep
their existing order over the provider's environment.

### M4. Immutable objects, with the sandbox's own state as the reference

**Location.** `~/.local/share/devcroft-artifacts/mise/<object-id>/`. A sibling
of devcroft's data directory, not a child: `~/.local/share/devcroft` is
baseline-denied and baseline denials always win, so nothing under it could be
granted. Not a cache directory either, since deleting it breaks live sandboxes.

**Identity.** The object id is a hash of the derived `mise.toml`, the lock and
sidecar bytes, the target (OS, architecture, libc family), the mise version
and the adapter version. Objects derive only from checksum-verified upstream
artifacts, so two projects with the same recipe share one. Content digests of
what was installed are recorded in the completion record, separately.

**Publication.** mise's data directory is `<object>/data`, so tools install at
their final absolute path and nothing is relocated. Under an exclusive
per-object lock: if the directory exists without a completion record, remove
it entirely first, so mise never treats a half-extracted tool as installed;
install; verify the lock bytes are unchanged; write the completion record
atomically. Readers require the record, never the directory alone.

**Grants.** Sessions get `<object>/data/installs` read-only with origin
`provider:mise`. Runtime caches and anything mutable belong to the sandbox's
own `.devcroft/<name>/home`.

**A manifest write grant overlapping the store is refused** at layer `config`,
exit code 2. The existing `policy::capability_set::package_manager_authority`
does this for `/nix/store` alone, with a message about Nix. This change adds
the same check for provider artifact roots beside it, on resolved grants so a
symlink counts as the path it names, with a message naming the store.

**References and reclamation.** A sandbox's `Meta` records its object id. That
is the reference; there is no separate count to drift. `rm`, and `up
--recreate` when it replaces the id, then scan every recorded `Meta` the way
`state::recorded_allocations` already does for ports and remove each object of
this provider that none names, published or not. `up` holds a shared store
lock from acquiring its object until its `Meta` is written and the sweep takes
it exclusively, so a sweep cannot remove an object a concurrent `up` is about
to record. `down` removes nothing. `doctor` reports the store's size.

This mirrors `DERIVED_ENV_PREFIX`, which `rm` already removes only when
nothing else is left. It replaces the draft's "no GC", which on a closed
command surface meant no way to reclaim anything.

### M5. The shell and host runtime are the narrowest grant that works

devcroft needs a POSIX shell and no mise project can normally declare one.
Resolve it from the object first (a tool that ships `sh`). Otherwise:

- **Linux:** grant the host shell's **file**, canonicalized (`/usr/bin/dash`
  on a merged-`/usr` host), plus the loader and libraries it needs, each as a
  file with origin `provider:mise`. `shell::resolve` accepts a candidate inside
  a declared grant, and a file grant equal to the candidate satisfies that.
  `Identity` goes on writing `/bin/sh` into the view as it does today.
- **macOS:** grant `host_shell_dir()` (`/bin`) and `/var/select`, as
  `swift.rs` does, since `/bin/sh` is a selector that reads the latter.

**`/usr/bin` is not granted on either platform.** `swift.rs` grants it on
macOS by a recorded, measured decision for a provider that ships no userland
and fails the qualification test by its own admission. Copying that to Linux
would have been a different thing entirely: `host_shell_dir()` canonicalizes
to `/usr/bin` there, `CANONICAL_PATH` ends in it, and the host's `gcc`, `ld`,
`git` and `python3` would have resolved and run, which is criterion 5 failing
while the spec claimed it held. A project that needs `grep` or `sed` declares
a tool that provides them.

Task 0.4 measures whether that narrow sandbox is usable. If it is not on
macOS, widening is a decision to record with its reasons, as swift's was. It
is not available on Linux.

Because no directory grant covers `/bin/sh` or `/usr/bin/env` under this
design, the draft's change to `sandbox_identity` is dropped. Its `granted`
check does compare by equality and would miss a covering directory grant; that
is true today for any manifest, predates this change, and is noted in task 6.3
as something to file rather than fold in.

**Host runtime libraries.** Tools are prebuilt and link against the host's
libc. Each qualified profile carries a fixed inventory (loader, libc family),
rendered as `provider:mise`. Preconditions are checked by **reading** each
installed executable's headers for its interpreter and needed-library names,
and failing at layer `provider` when one resolves neither inside the object
nor in the inventory. Never by running it: `ldd` executes the binary's
interpreter. Whether the reader is hand-written or the `object` crate is
decided in 0.4 with the dependency count stated.

`process-compose` is not this change's concern; `add-mise-daemons` requires it
in `[tools]` when daemons are declared.

### M6. Full inputs, scoped diagnostics, and the seams that change

`fingerprint` covers the manifest, the lock and sidecars, every dotenv and
source file `[env]` names, the adapter version and the target. Files other
than the manifest go through `capture::optional_file_part`, so a consumed file
deleted after `up` yields a different hash and `status` reports `env: stale`,
rather than an I/O error that `status` shows as `unknown`.

Code that changes outside the new module:

- `src/provider/validate.rs`: `"mise"` leaves `NOT_YET_SUPPORTED` and joins
  `SUPPORTED`. The unit test asserting it is not yet supported is rewritten
  for `hermit`.
- `src/provider/mod.rs`: `ProviderKind::Mise`, with arms in `from_name`,
  `static_name`, `tier`, `resolution_reach` and `manifest_fingerprint`.
  `resolution_reach` states what runs host-side (`mise install` and `mise env`
  on a derived tools-only config) and that no daemon authority is involved.
- `src/bin/devcroft.rs`: the staleness line hardcodes "flake" or "flox
  manifest" and is already wrong for devbox, devenv and swift. It names the
  active provider's manifest instead.
- `src/lifecycle/state.rs`: `Meta` records the object id.

`init` detects a mise manifest by reading files and never runs mise; several
candidates follow the existing explicit-choice contract. `doctor` reports the
mise binary and version, the lock's coverage of this platform, the backends
that would be refused, the shell that would be resolved, and that `[tasks]` are
not runnable in the sandbox. It installs nothing.

Versions: a minimum mise version is enforced. A version newer than the last
one qualified draws one warning naming it, rather than a refusal, since mise
releases weekly and a hard ceiling would fail most users most of the time.

Errors use existing layers: syntax and value errors `config`/2, resolution
`provider`/3, enforcement `backend`/4, lifecycle `keeper`/5.

## Risks / Trade-offs

- The admitted backend list may be short → publish it exactly, refuse the rest
  by name; 0.5 stops the change if it is too short to be useful.
- No host userland → a mise sandbox has only what `[tools]` declares plus a
  shell. That is the completeness criterion working, and it will surprise
  users who expect `grep`. `doctor` says so.
- `[env]` is reimplemented rather than delegated → the admitted subset is
  small and everything outside it is refused by key. The alternative leaves
  the project root wrong.
- Host-linked runtime → every host grant is rendered, per qualified target.
- A newer mise changes lock or CLI behavior → fixtures pinned per qualified
  version; the lock-unchanged check fails loudly.

## Migration Plan

1. Group 0: measure, and stop if 0.5 says so.
2. Provider selection, input classification and refusals.
3. Store, materialization and capture.
4. Confined `[env]`, shell and host-runtime grants.
5. Tests on native targets; publish passing and refused rows.
6. Update decisions, README and context only when the tasks pass.

Rollback removes the provider selection. It does not delete objects; they are
removed by the next `rm` that finds them unreferenced.

## Open Questions

All tracked in Group 0, each with its required outcome:

- Which backends install without executing what they downloaded?
- Does `mise install` under locked mode leave the copied lock byte-identical on
  each target? If not for a backend, that backend is refused.
- What is the exact list of project-level config locations and of settings
  that isolate mise from `HOME`, parents and the ambient environment?
- Which host files does the shell need per Linux profile, and is the narrow
  sandbox usable?
