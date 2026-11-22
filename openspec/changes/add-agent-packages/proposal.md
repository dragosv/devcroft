# Change: add-agent-packages

## Why

Getting a working coding agent into a sandbox is an adoption condition, not a
packaging detail (owner, 2026-11-19). Today the user becomes the integrator.
They write a closure-tier environment for the agent (the `[tools]` layer of
`add-agent-workload`, or the project's own manifest as the
`devenv-<agent>-sample`s do), find a compatible ACP adapter, install its
runtime and repair `PATH`. The agent also ends up depending on the project's
provider, or on a provider the user had to learn just for the agent.

The owner's requirement is the opposite: adding an agent together with its ACP
adapter is easy, and the agent is something **devcroft controls**,
independent of the project's provider. A project on Node 16 must keep running
its tests on Node 16 while an agent that needs Node 24 runs on Node 24.

The repository already has the pieces. `provider::artifacts` (built for mise
and pixi) is a devcroft-owned, content-addressed, read-only store with leases
and sweep. `docs/decisions.md` §1 ("Rejected: the host's binary, 'just for
agents'") names this exact revisit condition: an agent "with a published
checksum, pinned somewhere devcroft can verify. That would be a different
artifact, not this shortcut."

## What Changes

- **Agent recipes owned by devcroft.** A recipe pins, per platform, the
  harness, the ACP adapter if the harness does not speak ACP natively, the
  private runtime if the distribution needs one, each with URL and checksum.
  It also pins the entrypoint and argv, the absolute paths for every
  executable the agent chain starts, the agent's private config and state
  variables, its network destinations and its authentication method. Recipes
  ship inside the devcroft release. The user never writes Nix, MCP JSON or a
  `PATH` fix.
- **Selection by the user, not the project.** `[agent] name = "<recipe>"` in
  the user configuration (where `[tools]` and `[[credentials.broker]]`
  already live), or `devcroft up --agent <recipe>`. The project's
  `devcroft.toml` and its provider are unchanged and unconsulted.
- **Materialized at `up`, host-side, executing nothing downloaded.** Artifacts
  are verified and extracted into immutable objects under
  `~/.local/share/devcroft-artifacts/agents/`, shared between sandboxes and
  swept when nothing references them. npm-distributed adapters are laid out
  from their lockfile without install scripts. A recipe that cannot be
  installed this way runs its install in the existing confined provisioning
  phase, or is refused. The gate decides which.
- **Private runtime, same sandbox.** The agent starts inside the project's
  sandbox, with the project's environment, through absolute paths into its
  object. Its runtime never goes on `PATH`. A command the agent runs through
  its tools inherits the project's `PATH`, so `node` there is the project's.
  A launcher for the agent's own command name (`claude`, not `node`) is placed
  on `PATH` after the project's entries, for interactive use.
- **ACP endpoint.** `devcroft agent acp [name]` starts the agent's ACP
  entrypoint as a piped session and bridges stdio, so an editor's ACP client
  configuration is one line. `devcroft agent list` shows the recipes with
  their versions and platforms.
- **Visible and attributed.** The agent object and its host runtime are
  read-only grants, its state directory is a write grant, and its hosts join
  the allowlist. All carry origin `agent:<recipe>` and appear in
  `policy --render`. `status` shows the agent, its version and its tier
  (artifact). Credentials go through the existing broker.
- **What this does not guarantee, stated:** agent and project share one
  policy. The agent can still deliberately start its private runtime.
  Separate policies and routed execution are `add-agent-executor`, which
  builds on this change.

## Capabilities

### New Capabilities

- `agent-packages`: recipes, materialization, the private runtime, the
  launcher and the ACP endpoint.

### Modified Capabilities

- `config`: `[agent]` in the user configuration and `up --agent`.
- `policy`: grants with origin `agent:<recipe>`.
- `cli`: `agent acp`, `agent list`, and the agent in `status` and `doctor`.

## Impact

- New `src/agent/` (recipes, materialization, launch); `src/provider/artifacts.rs`
  as a fourth consumer; `src/config/user.rs`; `src/lifecycle/{up,state,status}.rs`;
  `src/bin/devcroft.rs` (`USAGE` must list the new subcommands, which
  `tests/cli_help_and_version.rs` enforces); `tests/agent_packages_e2e.rs`.
- `src/policy`: a new rule origin. CLAUDE.md's list of origins gains
  `agent:<recipe>`.
- `[tools]` stays as it is, for tools that are not first-class agents. The
  `devenv-<agent>-sample`s stay valid as the team-declared alternative.
- `docs/decisions.md` §1: the "host's binary" rejection gains a pointer to
  the built alternative. `docs/agents.md` gains a recipe column.
- Relation to active changes: credentials are `add-agent-workload`'s broker,
  unchanged. Fleet consumes recipes in a later group, declared separately and
  only after the `up` path is qualified.

See [design.md](design.md) for decisions and the measurement gate.
