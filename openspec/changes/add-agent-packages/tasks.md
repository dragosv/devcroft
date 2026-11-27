## 0. Fixtures and the qualification record

- [ ] 0.1 Build `devcroft-test-agent`, a Rust multicall fixture under `tests/fixtures/`, with these personalities:
  - CLI and PTY;
  - OSC 7501;
  - process trees, including `setsid`, double fork and inherited stdio;
  - a sentinel;
  - a credential echo that reports what it received out of band.

  Confirm that `cargo package --list` does not contain it and that `cargo install` installs only `devcroft`.
- [ ] 0.2 Build `tests/fixtures/model-loop-agent`, a crate with its own `[workspace]` table, on oi-kit pinned exactly (`=0.1.0-alpha.16` or the version current when work starts, with `rustls-tls`).
  - Before writing it, confirm on docs.rs the `Provider` trait’s signature and that a `StateMachine` runs with no network under a scripted provider.
  - Implement a scripted `Provider` that replays a fixed sequence of tool calls, and wire the OpenAI-compatible provider to `fake-llm` (0.3).
  - Define in-process tools for reading and editing a file, running a command and reporting a result, and no others.
  - Build it on `linux-aarch64` and `macos-arm64`. Confirm that devcroft’s `Cargo.lock`, `THIRD-PARTY-LICENSES.md` and `cargo package --list` are unchanged.
  - Run it inside a sandbox to edit a file and run the project test, in scripted mode and against `fake-llm` through the broker, both in CI.
- [ ] 0.3 Build `tests/fixtures/fake-llm` in the same fixtures workspace.
  - It speaks OpenAI chat completions with SSE streaming, and it is structured so that Anthropic Messages can be added for `add-claude-agent-recipe`.
  - Each scenario is a script of answers matched on request content. An unexpected request fails the run.
  - It records what it received: headers, tool schemas and prompts.
- [ ] 0.4 Add the broker’s upstream override and test CA under `test-support` only, next to the existing seam in `src/proxy/server.rs`. Prove both directions:
  - a build without the feature, given every input the hook reads, still connects only to the declared host with the normal trust store;
  - a build with it delivers the synthetic key, never the placeholder, to `fake-llm`.
- [ ] 0.5 Define the versioned qualification record: rows per artifact digest, devcroft version, platform, launch path and auth mode, and declared versus observed results (`pass`, `fail`, `drift`, `skipped`, `unknown`). Make `agent list`, `doctor`, the CI release gate and `docs/agents.md` read it. Prove that a skipped or unknown mandatory probe leaves a row unqualified.

## 1. Qualify one native candidate

- [ ] 1.1 Qualify OpenCode per platform on `linux-aarch64` and `macos-arm64` for one named model provider. Record per platform:
  - URLs, digests, component versions, libc and loader;
  - dependencies, required commands and relative `state_paths`;
  - the Linux `procfs` need;
  - macOS `HTTPS_PROXY` userinfo (`http://nono:<token>@127.0.0.1:<port>`) behavior across startup and model requests;
  - any update or extra-download behavior;
  - every request the harness makes at startup and per turn, which `fake-llm` must serve.

  On any platform where OpenCode fails, qualify Goose by the same criteria or publish only the passing platform(s). Measure `fx` by the same criteria as a comparator, recording which probes it passes. Write every result into the qualification record.
- [ ] 1.2 Demonstrate a provider request through devcroft’s broker with an empty project allowlist. Confirm the agent runs from a read-only object, started by absolute path, with `PATH` unchanged. Record qualifying platforms and auth mode before advertising the recipe.
- [ ] 1.3 Measure on `macos-arm64` whether a sandboxed process can read a host process’s environment (`KERN_PROCARGS2`, `ps -E`). Record the result in `docs/known-gaps.md` whatever it is.

## 2. Install and select

- [ ] 2.1 Implement validated embedded and user recipes: names, relative `state_paths`, `argv[0]` inside the object before download and after extraction, Linux libc, and `agents/` beside the resolved user configuration (`XDG_CONFIG_HOME`). Add a committed seed pinning every registry revision, and a dev-only generator that reproduces it. Implement `single-binary` and `archive` layouts with checksums, sanitized entries, fetch limits (connect, stall and total timeouts, size, redirects staying on HTTPS), extraction limits and atomic publication. Run validation, checksum, digest, pin and archive-escape tests as unit tests that do not skip when no provider is available.
- [ ] 2.2 Add the embedded recipes to `Cargo.toml`’s anchored `include`, and make every crate the fetcher and extractor call a direct dependency. Build from the unpacked `cargo package` output and confirm `agent list` shows the recipes. Regenerate attribution if `Cargo.lock` changes.
- [ ] 2.3 Implement host `agent list`, `agent install <recipe> [--version <v>]` and `agent uninstall <recipe> [--version <v>]`, object-specific pins, and the lease-until-referenced rule on `up`, `agent install` and `agent run`. Move `rm`’s `Meta` read and state removal under the lifecycle lock, and its sweep under the store lease. Test:
  - mismatches and unsafe entries;
  - a stalled server, an off-HTTPS redirect and an expansion bomb;
  - install concurrent with `rm`, and `rm` concurrent with `up`;
  - two sandboxes sharing an object;
  - two different objects with the same recipe name and version.
- [ ] 2.4 Implement:
  - user selection, deterministic `auth` mode precedence and flags;
  - `--agent-upgrade`, with recreation running the recorded resolved recipe and the upgrade printing its changes before applying them;
  - `--agent-from-config` refusing an absent `[agent]`;
  - the canonical `recipe_digest` and `object_id` serializations with golden vectors;
  - staleness across non-binary recipe changes such as `hosts`, `launch.env` or `reserved_env`;
  - `mode1_recipes` retention across `--no-agent` and recipe switches, and the retained-state line;
  - `cd <project_root> && up [--name <n>]` remedy formatting.

  Test legacy, stopped and named sandboxes, opt-out across recreation, recreation after an upgrade of devcroft (no change without `--agent-upgrade`), switching recipe and version, and restart after the recorded recipe disappears from the release.
- [ ] 2.5 Insert recipe checks and downloads at the boundary from `validate-before-recreate`, and confirm failed preparation leaves the previous sandbox available. Before provider resolution, check brokers, the proxy decision and recipe hosts against the manifest’s network policy plus the recipe’s hosts, and check the agent’s executable against its own grants. Verify that `policy --render`, `why` and compilation use one final policy, with origin labels, baseline denials and an unchanged provider fingerprint and capture.

## 3. Run a task in the terminal

- [ ] 3.1 Implement the one target resolver shared with `agent acp` and `status`, including the refusal for a nested Git repository without its own manifest. Implement copyable recreate commands, explicit `<recipe>` propagation on a brand-new sandbox (refusing a missing selection before `up`), automatic materialization on interactive launch from the recorded recipe, and host/in-sandbox command dispatch. Check:
  - nested projects and nested repositories;
  - fresh projects with no `[agent]` in `config.toml`;
  - multiple named sandboxes;
  - `status` run outside `project_root`;
  - wrong-side calls.

  Report unsupported fleet use explicitly.
- [ ] 3.2 Launch by absolute path with per-launch variables and no `PATH` change, and grant the sandbox `HOME` read-write with origin `agent:<recipe>`. Check that the agent’s command is not found in `devcroft shell`, and that a manifest with no project-root grant still lets the agent write its state. Check old keeper refusal, reserved and runtime variables from the provider and activation, and project `PATH` shadowing. Confirm project Node 16 and a fixture agent’s private Node 24 remain distinct, and that an executable needing a provider-only path is refused before teardown.
- [ ] 3.3 Protect host-side configuration merges in sandbox HOME using `O_NOFOLLOW` descriptors and `renameat` (`OwnedDir`), and confirm unrelated settings survive restart. Confirm redirected and malformed targets are refused, never replaced. Confirm credential collisions are refused, including a provider-exported key and a broker entry that differs only in placeholder, and that the broker receives the real key only once per start. Start the egress proxy from a cleared environment and confirm an unrelated host secret is absent from it.
- [ ] 3.4 Run the `support-osc7501-terminal` cases `T01`–`T10` and `T12` through `agent run`. Run the qualified agent inside an `hc ls`-monitored terminal during a project task. If the harness emits OSC 7501 natively, record the `hc ls` state transition. If not, confirm devcroft fabricates no reply, identity or state (`OSC-05`) and record `osc7501_native = false` in `docs/agents.md` (`S9`).
- [ ] 3.5 From a fresh project, run the maintained agent, edit a file and execute its test with the project’s tools. Do it first against `fake-llm` behind the broker, in CI and deterministically, and then with the named provider through the broker, which is the qualifying run. A run with a real local model is optional and non-gating; if it is kept, record model, server and template digests, the number of runs and the failure types. Document in the CLI help, README and agent matrix:
  - the exact recipe/platform/auth combination;
  - OSC 7501 emission status;
  - shared-sandbox custody and retained state after opt-out;
  - the install, upgrade and recovery commands.

  Update the policy-origin documentation.
