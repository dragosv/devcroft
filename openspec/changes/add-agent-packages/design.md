## Scope and release evidence

Start with OpenCode for one explicitly named model provider, qualifying `linux-aarch64` and `macos-arm64` independently. On each platform record the exact executable, checksum, library dependencies, relative `state_paths` under `HOME`, self-update settings, required commands, Linux `procfs` need, macOS `HTTPS_PROXY` userinfo (`http://nono:<token>@127.0.0.1:<port>`) compatibility across startup and model requests (since Bun previously misparsed `<token>@127.0.0.1` in `docs/agents.md`), and a successful request through devcroft’s broker. Qualify Goose by the same criteria on any platform where OpenCode fails, or publish only the passing platform(s). Measure `fx` (Vercel Labs) by the same criteria as a comparator, without promoting it ahead of OpenCode before its broker, shell and lifecycle probes pass. Advertise only combinations that pass; a catalog entry or a successful download alone cannot establish support.

The qualification record is the source of truth. It has one row per artifact digest, devcroft version, platform, launch path and auth mode, and it holds what the recipe declares next to what each probe observed (`pass`, `fail`, `drift`, `skipped`, `unknown`). `agent list`, `doctor`, the CI release gate and `docs/agents.md` read that one record instead of keeping separate tables. Any mandatory probe that did not run leaves its row unqualified. The rule exists because every e2e test here skips itself on a host that cannot run it, and a skip looks like a pass.

Evidence has five levels, and none substitutes for another:

1. **Deterministic fixtures.** `devcroft-test-agent` is a Rust multicall fixture under `tests/fixtures/`, kept out of `src/bin` and out of the published crate. It covers CLI and PTY behaviour, process trees, sentinels and credentials, and reports what it saw out of band. It has no model and stays dependency-light.
2. **OS-level probes** on a real backend.
3. **devcroft’s own model–tool loop.** `model-loop-agent` is a small agent built on goose’s oi-kit crates (`oi-kit-agent`, `oi-kit-provider-types`, `oi-kit-providers`). It runs inside the sandbox and edits a file, runs the project test and reads the result through tools devcroft defines in-process. oi-kit ships no shell or other implicit tool, so the fixture exposes exactly the operations under test. It has two providers:
   - a scripted one, devcroft’s own implementation of the `Provider` trait, which replays a fixed sequence of tool calls, in-process;
   - the OpenAI-compatible provider pointed at the fake LLM server of level 4, which exercises the egress proxy and broker path with no vendor harness at all.

   It is its own crate, with a `[workspace]` table of its own (as the samples have) and the oi-kit version pinned exactly. oi-kit is alpha, and its dependencies (`reqwest`, `rmcp` and their tails) stay out of devcroft’s `Cargo.lock`, `THIRD-PARTY-LICENSES.md` and published package. The reviews had proposed an embedded `libfx` for this role: a Zig engine behind a WASM boundary, whose only probe could not be reproduced. oi-kit serves no ACP, so it does not replace Testy.
4. **The real harness against a fake LLM server**, deterministic and run in CI.

   `fake-llm` lives in the same fixtures workspace. It speaks the named provider’s wire protocol: OpenAI chat completions with SSE streaming first, and Anthropic Messages when `add-claude-agent-recipe` needs it. For each scenario it answers with scripted tool calls, matching each request on its content (the last tool result) rather than its position. An unexpected request fails the run loudly. It also serves, or records and fails on, any request the harness makes on its own, such as a model list at startup.

   It sits upstream of the broker, on the host, so the run follows the real route: harness → devcroft’s proxy endpoint → broker → fake server. It records what reached it and asserts:
   - the synthetic key arrived and the placeholder never did;
   - no secret appears in a prompt;
   - the tool schemas and the call order match the scenario;
   - streaming and cancellation behave.

   Pointing the broker at it takes an upstream override and a test CA. Both exist only under the non-default `test-support` feature, beside the seam `src/proxy/server.rs` already has. A release build has no flag, variable or file that can redirect a broker’s upstream or add a trust anchor for it, because such a switch in a shipped binary would be a way to exfiltrate the key.
5. **The real harness against the named provider** through the broker. This is the qualifying run.

A fake cannot show that a real model can drive the harness, and it cannot show real authentication or the vendor’s endpoint. Those remain level 5’s job. A run with a real local model is optional, a non-gating nightly job for behaviour only a real model triggers.

A fixture result never qualifies a vendor binary. That holds for `model-loop-agent` too, although goose uses the same engine, and for a passing run against `fake-llm`. Neither qualifies a recipe.

Prefer a statically linked or fully bundled payload. Record each Linux artifact’s libc (`gnu`, `musl` or `static`) and refuse a host whose loader does not match before downloading. A universal Mach-O stays refused until the header reader selects and checks the host’s slice. A checksum proves integrity, not that the executable can start on this host.

## Install and retain a package

Keep reviewed URLs and checksums in embedded versioned recipes. A committed seed pins upstream coordinates, tags and the revision of every registry the development-only generator reads, so regenerating from a clean checkout at the same commit produces identical recipes. The installed CLI fetches directly over HTTPS and verifies the payload without invoking a host package manager. Ship the `single-binary` and safe `archive` layouts here; refuse unimplemented layouts at layer `config` before download. Validate user recipes with the same rules and label their provenance. User recipes live in `agents/` beside the resolved user configuration file, so `XDG_CONFIG_HOME` moves both together.

Validate a recipe name as one lowercase component and every entrypoint’s `argv[0]` as a relative path of normal components that resolves inside the object, before download and again after extraction. Bound every fetch (connect, stall and total timeouts, maximum size, and a maximum number of redirects, each staying on HTTPS) and every extraction (entry count, expanded size). A limit failure publishes nothing, removes staging and releases the lifecycle lock within the stall timeout, so a server that accepts and then goes silent cannot hold `down` or `rm` of that sandbox.

The published crate must carry all of this. Add the embedded recipe files to `Cargo.toml`’s anchored `include`, declare every crate the fetcher and extractor call as a direct dependency (`hyper` and `flate2` are only transitive today, and `rustls` is a dev-dependency), and verify by building from the unpacked `cargo package` output and running `agent list`.

Use the shared artifact store with per-object locks, atomic publication and read-only sandbox grants. Every path that publishes an object (`up`, `agent install` and `agent run`) takes the agents store lease before publication and holds it until an install pin or a recorded `Meta` references the object, whatever the project’s provider. `rm` reads `Meta` and removes the sandbox’s state under its lifecycle lock, and sweeps agent objects under the store lease; today `rm` reads `Meta` before taking the lock and sweeps after releasing it (`src/lifecycle/terminate.rs`). Explicit installation (`agent install <recipe> [--version <v>]`) pins the object by `object_id`, including when a user recipe overrides the same name and version. Removal sweeps only objects referenced by neither a pin nor any recorded sandbox.

## Project commands and agent state

Start the agent by absolute path through the existing keeper: from `agent run` here, and from `agent acp` in the editor change. Nothing is added to `PATH`. Project tools stay first, then user tools, then the base runtime.

An earlier design put a per-sandbox launcher on `PATH`. It is dropped from this change for two reasons. `OwnedDir::write` creates files with mode `0644`, so the command would fail with `EACCES` in `devcroft shell`. A shebang naming `current_exe()` also breaks when devcroft is upgraded or moved. A later change can add a multicall link to the pinned runtime executable, with mode `0755` and a real execution test.

Strip `reserved_env` and `requires.runtime_env` names after the activation overlay and spawn environment. Refuse reserved manifest variables and keepers without this protocol support. Removing a runtime variable also removes it from commands the agent starts, so `up` names each one it removed. Check the agent’s executable headers against the agent’s own grants and the baseline before teardown, never against the provider’s grants, so the agent cannot come to depend on the project’s closure.

Store history and configuration in the sandbox HOME, and grant it read-write with origin `agent:<recipe>`. Today `.devcroft/<name>/home` is only an exception to the artifact directory’s write-deny (`src/policy/mod.rs`), not a grant, so without one an agent on a manifest that grants no `.` could not write its own state.

Keep one HOME per sandbox, shared by every recipe ever selected there. A HOME per recipe was considered and rejected for Mode 1. It would separate files, not authority: project processes and every agent share one policy, so each can read the other homes anyway, and a switch would have to move state while the project’s own `HOME` diverged from the agent’s. The retained-state line covers the switch honestly. A per-recipe home belongs to the isolated mode, where it is a boundary.

Traverse managed directories via `O_NOFOLLOW` directory descriptors and commit via exclusive temp file plus `renameat` (`OwnedDir`) for host-managed files in workspace-writable directories. Preserve unrelated settings across restarts and recreation. The agent shares this HOME, policy and credential route with project processes. Persist `mode1_recipes` (every recipe name and its relative `state_paths` ever run in shared mode on this sandbox) in `Meta` across `down`, `up`, `up --recreate`, `--no-agent` and recipe switches until `rm`, so that a later isolated mode can tell this sandbox’s HOME has held agent state. After a recipe switch or `--no-agent`, `up` names every recorded state path still present in `HOME`, because project processes can still read it.

## Selection and startup

Record the selected recipe, requested version, resolved recipe, auth mode, source, protocol version, `object_id`, `recipe_digest` and `mode1_recipes`.

`recipe_digest` is a SHA-256 over a versioned canonical serialization of the full resolved recipe for the current platform, plus the active `auth_mode`. It covers artifacts, entrypoints, `launch.env`, `reserved_env`, `requires`, `hosts`, `hooks` and `state_paths`. The serialization starts with its schema version, sorts keys, orders artifacts by path, lowercases and sorts hosts, and keeps `argv` order. Golden vectors in the tests fix it, and `object_id` the same way.

Recreation is not an upgrade. Plain `up` and `up --recreate` without a selection flag run the resolved recipe recorded in `Meta`, re-materializing its object if needed, so the agent’s bytes, hosts and auth never change as a side effect of recreating. `up --recreate --agent-upgrade` adopts what this release resolves for the recorded selection, after printing the changed artifacts, hosts and auth. Staleness compares `recipe_digest` against re-resolving the recorded selection (never `[agent]` default drift) and names that command. `object_id` identifies bytes and `recipe_digest` identifies authority, so a policy-only change in a recipe needs no new download. Preserve recorded selections and explicit opt-outs across plain starts and recreation. `--agent-from-config` with no `[agent]` fails rather than silently recording no agent. A sandbox with no recorded choice adopts the global default only on recreation. A change of selection on any existing sandbox requires recreation.

Resolve target recipe `R`’s version and auth deterministically. Use `--agent-auth` first. Otherwise use `Meta`’s mode, but only when `Meta` already records `R` and `--agent-from-config` was not passed. Otherwise use `[agent] auth` when `[agent]` names `R`, and otherwise `R`’s default. The mode is never inherited from a different recipe.

For `agent run [<recipe>]`, an explicit `--sandbox <name>` selects its recorded root or matching manifest. Otherwise compare the nearest manifest and the deepest recorded root containing the canonical working directory, and choose the deeper root. An equal-root ambiguity is reported with the candidate names and `--sandbox`. When a Git repository root (a `.git` directory or file, checked by name only) lies between the working directory and the chosen root, the command refuses and names the outer root and `--sandbox`. A nested repository without a manifest of its own, such as a vendored clone or a submodule, does not silently inherit its parent’s sandbox. One resolver serves `agent run`, `agent acp`, `status` and every remedy. When no `Meta` exists yet, `<recipe>` on the CLI passes `--agent <recipe>` to `up`, overriding an absent or different `[agent]` default. If neither `<recipe>` nor `[agent]` selects a recipe, fail at layer `config` before starting `up` or downloading. When `Meta` exists, materialize from the recipe it records, so a recorded recipe this release no longer ships still starts.

Every remedy is a command the user can copy. Prefix it with `cd <quoted project_root> && ` whenever canonical `cwd`’s nearest manifest root differs from `project_root`. Use `up --name <n>` whenever `sandbox_name != manifest.sandbox.name`.

Validate credentials, platform, launch path and selection, then materialize the package before teardown under the lifecycle lock. An already healthy unchanged sandbox uses the recorded choice. A failed replacement download leaves the old keeper and selection in place.

## Authentication and policy

The first maintained recipe uses a brokered API key from a non-empty host environment variable. Reject conflicting variable or host declarations. A user broker entry counts as identical to the recipe’s only when its host, variables, endpoint variable and placeholder all match. Remove the recipe’s credential variables from everything the agent inherits (the provider’s environment and the activation overlay; `[launch.env]` may not name them) and apply the selected mode’s values last.

Start the egress proxy from a cleared environment with an explicit allowlist. Today it inherits the user’s whole shell except the active brokers’ variables (`src/proxy/mod.rs`). On macOS, no process-visibility separation keeps a sandboxed process away from that environment until a measurement shows otherwise.

Check network prerequisites (brokers, the egress-proxy decision and the recipe’s hosts) against the manifest’s network policy plus the recipe’s hosts, before teardown and provider resolution. This is sound because provider grants and the tooling layer add only paths, never hosts. `policy --render`, `why` and compilation all use the one final policy that adds those grants, and every added grant is shown with its origin. Require an explicit manifest setting for any needed unrestricted loopback, and check private procfs only on applicable Linux targets.

Agent selection, recipes and credentials come only from the user configuration. They keep working unchanged when manifest approval makes repository manifests untrusted by default. A project with no environment provider is outside this change; [the file-only workflow](../add-file-only-workflow/proposal.md) decides how the agent runs there and which recipes qualify.

## Terminal acceptance

Measure a real edit and test using the project’s tools, first against `fake-llm` behind the broker and then with the named provider. Use distinct project and agent runtimes in a fixture to make toolchain mix-ups observable. On `agent run`, reuse the [terminal transport cases](../support-osc7501-terminal/design.md) `T01`–`T10` and `T12`. During the live project task, observe `hc ls` if the qualified harness natively emits OSC 7501. Otherwise verify that devcroft fabricates no reply, identity or state (`OSC-05`, defined there), and record `osc7501_native = false` in the recipe’s published capability profile in `docs/agents.md` (`S9`).
