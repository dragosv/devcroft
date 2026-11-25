# Change: add-agent-packages

## Why

Getting a working coding agent into a sandbox is an adoption condition, not a
packaging detail. Today the user becomes the integrator. They write a
closure-tier environment for the agent (the `[tools]` layer of
`add-agent-workload`, or the project's own manifest as the
`devenv-<agent>-sample`s do), find a compatible ACP adapter, install its
runtime and repair `PATH`. They also hand-write a five-field
`[[credentials.broker]]` entry and add the agent's host to every project's
`network.allow`. The agent ends up depending on the project's provider, or on
a provider the user had to learn just for the agent.

The requirement is the opposite: adding an agent — whether interactive in a
terminal or over ACP from a GUI-launched editor — must be effortless, and the
agent is something **devcroft controls**, independent of the project's provider
and without requiring `mise`, `npm` or `nix` on the host. A project on Node 16
must keep running its tests on Node 16 while an agent that needs Node 24 runs
on Node 24. And when the user runs an agent interactively in an OSC
7501-capable terminal (`herdr-core` / `hc ls`), bidirectional OSC 7501
negotiation and live state reporting must work transparently end to end across
every supported PTY path.

The repository already has the core pieces. `provider::artifacts` (built for
mise and pixi) is a devcroft-owned, content-addressed, read-only store with
leases and sweep, and `runtime::owned_dir` already performs `O_NOFOLLOW`
descriptor-anchored writes into sandbox-writable directories. `docs/decisions.md`
§1 ("Rejected: the host's binary, 'just for agents'") names this exact revisit
condition: an agent "with a published checksum, pinned somewhere devcroft can
verify. That would be a different artifact, not this shortcut."

## What Changes

- **Agent recipes owned by devcroft, seeded from a committed catalog without
  runtime manager dependencies.** A recipe pins, per platform, the harness, the
  ACP adapter if the harness does not speak ACP natively (or `acp = "none"` for
  CLI-only agents), and the private runtime if the distribution needs one, each
  with URL and checksum. It also pins entrypoints, `reserved_env` names
  stripped at agent launch so inherited project variables (such as
  `CLAUDE_CODE_EXECUTABLE`) cannot hijack the adapter or trigger `argv` secret
  leaks, network destinations, supported authentication modes (`api-key`
  broker, `login` or `env`) and platform-scoped sandbox requirements. Recipes
  ship as TOML inside `src/agent/recipes/`, seeded from a committed
  catalog in `scripts/` using `aqua-registry` in dev-only tooling
  (zero reliance on untracked notes and zero runtime dependency on `mise`). A
  user may add or override a recipe in `~/.config/devcroft/agents/` under the
  same validator, with checksums mandatory.
- **One line of configuration, one selection rule, one credential source per
  variable.** `[agent] name = "<recipe>"` (with optional `version` and
  `auth = "api-key" | "login" | "env"`) in the user configuration sets the
  default for brand-new sandboxes (and `--recreate` of sandboxes that never had
  a choice); `up --agent <recipe>` (`--agent-auth <mode>`), `--no-agent` and
  `--agent-from-config` choose explicitly, and changing any existing sandbox's
  selection (running or stopped, including from `Unset`) requires `--recreate`
  so egress never widens silently. Switching to another recipe never carries
  over the previous recipe's `version` or `auth` mode. A key comes from the
  host variable or from a user-configured command, so an editor started by the
  OS can bring a sandbox up. The recipe's credential names and hosts are checked
  against the manifest and the user's own credentials in every mode, so an API
  key cannot silently replace a subscription login. Selecting an agent is the
  one user-configuration input that widens a sandbox's egress, and `up` says so
  every time.
- **Checks before teardown.** Everything that can be checked without the
  project's environment, including the credential checks already shipped, runs
  before `up --recreate` stops a working sandbox.
- **Materialized in pure Rust, executing nothing downloaded, with install
  retention pins.** `devcroft agent install <recipe>` — and interactive `up` or
  `agent run` when the object is absent — downloads, verifies checksums and
  extracts `single-binary`, `archive`, `node-tarball` and script-free
  `npm-lock` layouts into immutable objects under
  `~/.local/share/devcroft-artifacts/agents/`. Explicit `agent install` writes
  a retention pin, by object id, so unrelated sandbox removals never sweep a
  pre-installed agent; `agent uninstall` drops the pin and sweeps unreferenced objects.
- **Private runtime, same sandbox, symlink-safe `HOME` updates.** The agent
  starts inside the project's sandbox through absolute paths into its object,
  with `reserved_env` unset on spawn. Its runtime never goes on `PATH`. A
  non-shell launcher for the agent's own command name, in the sandbox's
  read-only artifact directory and granted read-only with origin `agent:<recipe>`,
  goes on `PATH` after the project's entries and the tooling layer. Host-side reads and merges into the
  sandbox-writable `HOME` traverse components via `O_NOFOLLOW` directory
  descriptors and replace files atomically (`OwnedDir`), refusing symlinked
  paths so a sandboxed process cannot redirect host writes while preserving
  user settings and logins across `down` and `up`.
- **Host commands, nested-project discovery and bounded ACP readiness.**
  `devcroft agent acp`, `agent run`, `agent install`, `agent uninstall` and
  `agent list` run on the host beside the in-sandbox `agent` verbs. Sandbox
  discovery compares both recorded `Meta` roots and upward `devcroft.toml`
  roots and picks the **innermost** root, so a nested subproject with its own
  manifest never attaches to an outer parent sandbox. `agent acp` performs fast
  preflight and waits a bounded time for a background `up` that finishes on its
  own, so cold provisioning neither hangs an editor's `initialize` nor starts
  over on each retry; `agent acp --print-config` and `doctor` support GUI-launched
  editor setup and handshake diagnostics.
- **Mandatory bidirectional OSC 7501 (`herdr-core`) terminal transparency.**
  Interactive PTY sessions (`agent run`, `devcroft shell` with a PTY, and
  `devcroft ssh` with a PTY) transport OSC 7501 capability queries,
  non-newline terminal replies and agent identity/state reports byte-for-byte
  in both directions, checking `tcsetattr` errors/`EINTR` and restoring terminal
  state on error or signal, with zero emulation, per-session isolation, and zero OSC output on `agent
  acp`'s JSON-RPC `stdout`.
- **Visible, attributed and fully fingerprinted.** Agent rules carry origin
  `agent:<recipe>` and appear in `policy --render`. `status` shows the agent,
  its version, source, auth mode, tier (`artifact`) and staleness against the
  full resolved recipe digest (a notice, not a refusal), printing `devcroft up [--name <n>] --recreate`.
  Platform-scoped requirements never apply Linux-only `procfs` checks on macOS.
- **What this does not guarantee, stated:** agent and project share one
  policy, environment and `HOME`. Separate policies, routed execution, a
  separate agent `HOME` and an isolated credential route are
  `add-agent-executor`, which builds on this.

## Capabilities

### New Capabilities

- `agent-packages`: recipes, pure-Rust materialization and install pins, the
  private runtime and `reserved_env` stripping, symlink-safe `HOME` merges,
  authentication modes, nested-safe `agent acp` / `agent run` discovery with
  bounded auto-up, and the ACP session lifecycle.

### Modified Capabilities

- `config`: `[agent]` (`name`, `version`, `auth`) in the user configuration,
  `[agent.credentials.<recipe>]`, user recipes, the `up` agent flags, and
  selection persistence across `down`, `up` and `--recreate`.
- `policy`: rules with origin `agent:<recipe>`, composed into the policy every
  `up` check reads.
- `lifecycle`: checks before teardown, and what `up` does with a selection on a
  running sandbox.
- `exec`, `ssh`: bidirectional OSC 7501 transparency for PTY sessions.
- `credentials`: what selecting a recipe asks for, and what it may widen.
- `cli`: `agent list|install|uninstall|acp|run` (including `--print-config`),
  exact `up [--name <n>]` remediation hints, and agent preflight/handshake
  checks in `status` and `doctor`.

## Impact

- New `src/agent/` (recipes, pure-Rust materialization, pins, launch,
  `O_NOFOLLOW` `HOME` merge via `runtime::owned_dir`); `src/provider/artifacts.rs`
  as a fourth consumer; `src/exec.rs` (signals blocked before raw mode,
  checked `tcsetattr` with `EINTR` retry, guaranteed restore);
  `src/keeper/{protocol,session}.rs` (`SpawnRequest.unset`); `src/config/user.rs`;
  `src/lifecycle/{up,state,status}.rs` (credential and broker checks
  moved before teardown and extended; composed policy read before all checks;
  `Meta` records the selection and the resolved recipe);
  `src/bin/devcroft.rs` (`cli_agent` host vs in-sandbox verbs, `__agent-launch`; innermost-root
  discovery and bounded auto-up in `agent acp`/`run`; `maybe_auto_up` loads
  user configuration; `USAGE`); `tests/agent_packages_e2e.rs` and
  `tests/osc7501_pty.rs`.
- Committed seed catalog (`scripts/agent-catalog.json`, pinned upstream
  coordinates and tags) and dev-only recipe generator (`scripts/` or `examples/` using
  `aqua-registry` outside the shipped `devcroft` crate) to reproduce
  `src/agent/recipes/*.toml` from a clean clone.
- Archive crates not yet in `Cargo.lock` (`tar`, plus any required archive
  decoder beyond `flate2`), followed by regenerating `THIRD-PARTY-LICENSES.md`.
- `src/policy`: new rule origin `agent:<recipe>` (added to `CLAUDE.md`).
- A `credentials` delta: `add-agent-workload`'s requirements stand, and a
  selected recipe's own active hosts become the single user-configuration input
  that widens egress.

See [design.md](design.md) for decisions and the measurement gate.
