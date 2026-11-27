## Why

Most repositories have no reproducible environment, and devcroft today offers them nothing: `env.provider` is required, and the documented answer is `flox init`. An agent that reads, searches and edits files is still useful there, as long as nothing from the project or the host can run. The 1.0 scope accepts this second workflow (owner decision, 2026-11-27). Without it, devcroft can run an agent only on projects that have already adopted a provider.

## What Changes

- `devcroft up --file-only` and `devcroft agent run --file-only [<recipe>]` create a sandbox with no environment provider, rooted at the nearest directory holding `devcroft.toml` or `.git` (else the working directory). It reads no manifest, and its profile is recorded and fixed for the sandbox’s lifetime.
- Only the selected agent’s object can execute.
  - **Linux:** the workspace, `HOME` and every writable path are mounted `noexec`.
  - **macOS:** `process-exec` and executable mapping are allowed only inside the agent’s object.

  devcroft’s base runtime, the user’s tooling layer and any provider are absent. A host that cannot enforce this refuses.
- Only agent sessions run (`agent run`, `agent acp`). `shell`, `exec`, `ssh` and `proxy` are refused, and there are no hooks, services or ports.
- Egress is the recipe’s hosts alone, through devcroft’s proxy and broker. The user’s own `[credentials]` are withheld.
- The workspace is read-write. `.git` stays read-only, because Git configuration and hooks run on the host without review, and the manifest stays protected.
- A recipe is offered here only when it is qualified for file-only on that platform. Qualification shows that its shell fails closed, and that project files cannot run inside its own runtime (plugins, eval tools, interpreter switches).
- `docs/decisions.md`’s “no non-reproducible mode” is revised to “no non-reproducible execution”. File-only executes nothing from the project or the host. `host` and `none` remain rejected as providers.

## Capabilities

### New Capabilities

- `file-only`: selection and root, the fixed profile, execution confined to the agent’s object, allowed sessions, network and credentials, workspace and `.git` access, recipe qualification, and reporting.

### Modified Capabilities

- `cli`: `--file-only` on `up` and `agent run`, refusals of non-agent sessions, and the profile in `status`.

## Impact

- **Depends on:** [agent packages](../add-agent-packages/proposal.md) for recipes and `agent run`. The [editor endpoint](../add-agent-editor-acp/proposal.md) attaches unchanged. Manifest protection comes from [manifest approval](../add-manifest-approval/proposal.md).
- **Distinct from** [`add-manifestless-mode`](../add-manifestless-mode/proposal.md), which detects or builds an environment so that the project can run. Here the project never runs.
- **Code:** lifecycle start without a provider, the Linux mount view (`noexec`, no runtime directory), Seatbelt exec rules (instead of `scope_exec_to_read_grants` over the workspace), keeper session admission, the recipe schema (per-platform file-only qualification), `USAGE`, and `tests/cli_help_and_version.rs`.
- **Docs:** `docs/decisions.md`, `CLAUDE.md`’s framing rule, `docs/agents.md` (a file-only column), `docs/known-gaps.md` (`.git` writes in the managed workflow), and the README.
