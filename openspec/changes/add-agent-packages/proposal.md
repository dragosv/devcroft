# Change: add-agent-packages

## Why

Getting a working coding agent into a sandbox is an adoption condition, not a
packaging detail (owner, 2026-11-19). Today the user becomes the integrator.
They write a closure-tier environment for the agent (the `[tools]` layer of
`add-agent-workload`, or the project's own manifest as the
`devenv-<agent>-sample`s do), find a compatible ACP adapter, install its
runtime and repair `PATH`. They also hand-write a five-field
`[[credentials.broker]]` entry and add the agent's host to every project's
`network.allow`. The agent ends up depending on the project's provider, or on
a provider the user had to learn just for the agent.

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
  It also pins the entrypoints, the absolute paths for every executable the
  agent chain starts, its network destinations, its broker template and its
  sandbox requirements. Recipes ship inside the devcroft release, several
  versions each. A user may add or override a recipe in
  `~/.config/devcroft/agents/`, under the same validator, with checksums
  mandatory. The user never writes Nix, MCP JSON or a `PATH` fix.
- **One line of configuration.** `[agent] name = "<recipe>"` in the user
  configuration, or `devcroft up --agent <recipe>`; `--no-agent` turns it off
  for one `up`. The recipe's hosts and its broker apply only to sandboxes that
  select it, so a user who exports `ANTHROPIC_API_KEY` writes nothing else.
  The project's `devcroft.toml` and its provider are unchanged and
  unconsulted. Selecting an agent is the one user-configuration input that
  widens a sandbox's egress, and `up` says so every time.
- **Materialized host-side, executing nothing downloaded.** `devcroft agent
  install <recipe>` (or the first `up`) verifies and extracts artifacts into
  immutable objects under `~/.local/share/devcroft-artifacts/agents/`, shared
  between sandboxes, leased while in use, swept when unreferenced.
  npm-distributed adapters are laid out from their lockfile without scripts.
  A recipe that cannot be installed this way runs its install in confined
  provisioning, or is refused.
- **Private runtime, same sandbox.** The agent starts inside the project's
  sandbox, through absolute paths into its object. Its runtime never goes on
  `PATH`, so a command the agent runs through its tools resolves the project's
  `node`. A launcher for the agent's own command name (`claude`, not `node`)
  lives in the object and goes on `PATH` after the project's entries. The
  agent keeps its config and login in the sandbox's own `HOME`.
- **Commands.** `devcroft agent acp` (an editor's ACP client configuration is
  one global line), `devcroft agent run` (interactive, on a PTY), `devcroft
  agent install` and `devcroft agent list`. They run on the host, beside the
  existing in-sandbox `agent` verbs.
- **Visible and attributed.** Agent rules carry origin `agent:<recipe>` and
  appear in `policy --render`. `status` shows the agent, its version, its tier
  (artifact) and whether it is stale. A recipe's requirements the host cannot
  meet (procfs, tools on `PATH`) fail or warn at `up`, by name.
- **What this does not guarantee, stated:** agent and project share one
  policy and one environment. Separate policies, routed execution and a
  separate credential route are `add-agent-executor`, which builds on this.

## Capabilities

### New Capabilities

- `agent-packages`: recipes, materialization, the private runtime, the
  launcher, the broker template and the ACP endpoint.

### Modified Capabilities

- `config`: `[agent]` in the user configuration, user recipes, `up --agent`
  and `--no-agent`.
- `policy`: rules with origin `agent:<recipe>`, composed into the policy every
  `up` check reads.
- `cli`: `agent acp|run|install|list`, and the agent in `status` and `doctor`.

## Impact

- New `src/agent/` (recipes, materialization, launch); `src/provider/artifacts.rs`
  as a fourth consumer; `src/config/user.rs`; `src/lifecycle/{up,state,status}.rs`
  (`check_brokers`, `ensure_egress_proxy` and the artifact lease read the
  composed policy, not the manifest alone); `src/bin/devcroft.rs` (`cli_agent`
  splits host verbs from in-sandbox verbs; `maybe_auto_up` loads the user
  configuration; `USAGE`, which `tests/cli_help_and_version.rs` enforces);
  `tests/agent_packages_e2e.rs`.
- `src/policy`: a new rule origin. CLAUDE.md's list of origins gains
  `agent:<recipe>`.
- `add-agent-workload`'s credentials spec says user configuration does not
  widen `network.allow`. This change amends that for a selected recipe's own
  hosts, and only those.
- `[tools]` stays as it is, for tools that are not first-class agents. The
  `devenv-<agent>-sample`s stay valid as the team-declared alternative.
- `init --agent claude-code` and `agent hook claude-code` keep their names;
  the recipe is named `claude-code` to match.
- `docs/decisions.md` §1: the "host's binary" rejection gains a pointer to
  the built alternative. `docs/agents.md` gains a recipe column.
- Relation to active changes: `add-sandbox-identity` (a separate sandbox UID
  must traverse the artifact store, as it must for mise and pixi). Fleet
  consumes recipes in a later group, after the `up` path is qualified.

See [design.md](design.md) for decisions and the measurement gate.
