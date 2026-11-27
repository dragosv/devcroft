## Scope and release evidence

Start with OpenCode for one explicitly named model provider, qualifying `linux-aarch64` and `macos-arm64` independently. On each platform record the exact executable, checksum, library dependencies, relative `state_paths` under `HOME`, self-update settings, required commands, Linux `procfs` need, macOS `HTTPS_PROXY` userinfo (`http://nono:<token>@127.0.0.1:<port>`) compatibility across startup and model requests (since Bun previously misparsed `<token>@127.0.0.1` in `docs/agents.md`), and a successful request through devcroft’s broker. Qualify Goose by the same criteria on any platform where OpenCode fails, or publish only the passing platform(s). Advertise only combinations that pass; a catalog entry or a successful download alone cannot establish support.

Prefer a statically linked or fully bundled payload. Record each Linux artifact’s libc (`gnu`, `musl` or `static`) and refuse a host whose loader does not match before downloading. A universal Mach-O stays refused until the header reader selects and checks the host’s slice. A checksum proves integrity, not that the executable can start on this host.

## Install and retain a package

Keep reviewed URLs and checksums in embedded versioned recipes. A committed seed pins upstream coordinates, tags and the revision of every registry the development-only generator reads, so regenerating from a clean checkout at the same commit produces identical recipes. The installed CLI fetches directly over HTTPS and verifies the payload without invoking a host package manager. Ship the `single-binary` and safe `archive` layouts here; refuse unimplemented layouts at layer `config` before download. Validate user recipes with the same rules and label their provenance. User recipes live in `agents/` beside the resolved user configuration file, so `XDG_CONFIG_HOME` moves both together.

Validate a recipe name as one lowercase component and every entrypoint’s `argv[0]` as a relative path of normal components that resolves inside the object, before download and again after extraction. Bound every fetch (connect timeout, stall timeout, maximum size) and every extraction (entry count, expanded size). A limit failure publishes nothing, removes staging and releases the lifecycle lock within the stall timeout, so a server that accepts and then goes silent cannot hold `down` or `rm` of that sandbox.

The published crate must carry all of this. Add the embedded recipe files to `Cargo.toml`’s anchored `include`, declare every crate the fetcher and extractor call as a direct dependency (`hyper` and `flate2` are only transitive today, and `rustls` is a dev-dependency), and verify by building from the unpacked `cargo package` output and running `agent list`.

Use the shared artifact store with per-object locks, atomic publication and read-only sandbox grants. Every path that publishes an object (`up`, `agent install` and `agent run`) takes the agents store lease before publication and holds it until an install pin or a recorded `Meta` references the object, whatever the project’s provider. `rm` reads `Meta` and removes the sandbox’s state under its lifecycle lock, and sweeps agent objects under the store lease; today `rm` reads `Meta` before taking the lock and sweeps after releasing it (`src/lifecycle/terminate.rs`). Explicit installation (`agent install <recipe> [--version <v>]`) pins the object by `object_id`, including when a user recipe overrides the same name and version. Removal sweeps only objects referenced by neither a pin nor any recorded sandbox.

## Project commands and agent state

Start the agent by absolute path through the existing keeper: from `agent run` here, and from `agent acp` in the editor change. Nothing is added to `PATH`. Project tools stay first, then user tools, then the base runtime.

An earlier design put a per-sandbox launcher on `PATH`. It is dropped from this change for two reasons. `OwnedDir::write` creates files with mode `0644`, so the command would fail with `EACCES` in `devcroft shell`. A shebang naming `current_exe()` also breaks when devcroft is upgraded or moved. A later change can add a multicall link to the pinned runtime executable, with mode `0755` and a real execution test.

Strip `reserved_env` and `requires.runtime_env` names after the activation overlay and spawn environment. Refuse reserved manifest variables and keepers without this protocol support. Removing a runtime variable also removes it from commands the agent starts, so `up` names each one it removed. Check the agent’s executable headers against the agent’s own grants and the baseline before teardown, never against the provider’s grants, so the agent cannot come to depend on the project’s closure.

Store history and configuration in the sandbox HOME. Traverse managed directories via `O_NOFOLLOW` directory descriptors and commit via exclusive temp file plus `renameat` (`OwnedDir`) for host-managed files in workspace-writable directories. Preserve unrelated settings across restarts and recreation. The agent shares this HOME, policy and credential route with project processes. Persist `mode1_recipes` (every recipe name and its relative `state_paths` ever run in shared mode on this sandbox) in `Meta` across `down`, `up`, `up --recreate`, `--no-agent` and recipe switches until `rm`, so that a later isolated mode can tell this sandbox’s HOME has held agent state. After a recipe switch or `--no-agent`, `up` names every recorded state path still present in `HOME`, because project processes can still read it.

## Selection and startup

Record the selected recipe, requested version, resolved recipe, auth mode, source, protocol version, `object_id`, `recipe_digest` and `mode1_recipes`.

`recipe_digest` is a SHA-256 over a versioned canonical serialization of the full resolved recipe for the current platform, plus the active `auth_mode`. It covers artifacts, entrypoints, `launch.env`, `reserved_env`, `requires`, `hosts`, `hooks` and `state_paths`. The serialization starts with its schema version, sorts keys, orders artifacts by path, lowercases and sorts hosts, and keeps `argv` order. Golden vectors in the tests fix it, and `object_id` the same way.

Staleness compares `recipe_digest` against re-resolving the recorded selection (never `[agent]` default drift). Preserve recorded selections and explicit opt-outs across plain starts and recreation. A sandbox with no recorded choice adopts the global default only on recreation. A change of selection on any existing sandbox requires recreation.

Resolve target recipe `R`’s version and auth deterministically. Use `--agent-auth` first. Otherwise use `Meta`’s mode, but only when `Meta` already records `R` and `--agent-from-config` was not passed. Otherwise use `[agent] auth` when `[agent]` names `R`, and otherwise `R`’s default. The mode is never inherited from a different recipe.

For `agent run [<recipe>]`, an explicit `--sandbox <name>` selects its recorded root or matching manifest. Otherwise compare the nearest manifest and the deepest recorded root containing the canonical working directory, and choose the deeper root. An equal-root ambiguity is reported with the candidate names and `--sandbox`. When no `Meta` exists yet, `<recipe>` on the CLI passes `--agent <recipe>` to `up`, overriding an absent or different `[agent]` default. If neither `<recipe>` nor `[agent]` selects a recipe, fail at layer `config` before starting `up` or downloading. When `Meta` exists, materialize from the recipe it records, so a recorded recipe this release no longer ships still starts.

Every remedy is a command the user can copy. Prefix it with `cd <quoted project_root> && ` whenever canonical `cwd`’s nearest manifest root differs from `project_root`. Use `up --name <n>` whenever `sandbox_name != manifest.sandbox.name`.

Validate credentials, platform, launch path and selection, then materialize the package before teardown under the lifecycle lock. An already healthy unchanged sandbox uses the recorded choice. A failed replacement download leaves the old keeper and selection in place.

## Authentication and policy

The first maintained recipe uses a brokered API key from a non-empty host environment variable. Reject conflicting variable or host declarations. A user broker entry counts as identical to the recipe’s only when its host, variables, endpoint variable and placeholder all match. Remove the recipe’s credential variables from everything the agent inherits (the provider’s environment and the activation overlay; `[launch.env]` may not name them) and apply the selected mode’s values last.

Start the egress proxy from a cleared environment with an explicit allowlist. Today it inherits the user’s whole shell except the active brokers’ variables (`src/proxy/mod.rs`). On macOS, no process-visibility separation keeps a sandboxed process away from that environment until a measurement shows otherwise.

Check network prerequisites (brokers, the egress-proxy decision and the recipe’s hosts) against the manifest’s network policy plus the recipe’s hosts, before teardown and provider resolution. This is sound because provider grants and the tooling layer add only paths, never hosts. `policy --render`, `why` and compilation all use the one final policy that adds those grants, and every added grant is shown with its origin. Require an explicit manifest setting for any needed unrestricted loopback, and check private procfs only on applicable Linux targets.

Agent selection, recipes and credentials come only from the user configuration. They keep working unchanged when manifest approval makes repository manifests untrusted by default. A project with no environment provider is outside this change; [the file-only workflow](../add-file-only-workflow/proposal.md) decides how the agent runs there and which recipes qualify.

## Terminal acceptance

Measure a real edit and test using the project’s tools. Use distinct project and agent runtimes in a fixture to make toolchain mix-ups observable. On `agent run`, reuse the [terminal transport cases](../support-osc7501-terminal/design.md) `T01`–`T10` and `T12`. During the live project task, observe `hc ls` if the qualified harness natively emits OSC 7501. Otherwise verify that devcroft fabricates no reply, identity or state (`OSC-05`, defined there), and record `osc7501_native = false` in the recipe’s published capability profile in `docs/agents.md` (`S9`).
