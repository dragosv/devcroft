# devcroft

Isolated, reproducible development environments built on OS-level sandboxing
— kernel primitives instead of containers or VMs — each reachable over SSH.

Many environments, including fleets of coding agents, run in parallel on one
host at near-zero marginal cost: they share a single content-addressed Nix
store while each stays behind its own kernel-enforced boundary. Because every
sandbox speaks SSH, existing editors work unchanged.

devcroft implements no isolation itself. It is a policy compiler, a
supervisor, and an SSH endpoint over existing sandbox backends.

## Status

**MVP implementation underway — 13/25 tasks.** The fd-passing keeper trick
(spike binary, task group 1) is proven on both Linux/Landlock and
macOS/Seatbelt; the config/policy compiler, the environment provider layer
(`flox` resolution, task group 3), the keeper's spawn protocol (control
socket, session registry, pty allocation — task 4.1), the supervisor
(`up`/`down`/`rm`, idempotent with crash recovery and `--recreate` — task
4.2), read-only sandbox introspection (`status`/`logs`/`ps` — task 4.3), and
one-shot `exec` with exit-code propagation, cwd mapping, and signal
forwarding (task 5.1) are implemented and tested end to end against real
`nono` and `flox`. There is still no `shell`/pty sessions, no auto-up, and no
SSH endpoint — those are the rest of task group 5 and task group 6.
User-facing documentation is written at release (task 7.4); until then the
specs are the source of truth.

| | |
|---|---|
| [openspec/changes/add-mvp-core/](openspec/changes/add-mvp-core/) | The MVP — proposal, design, tasks, 7 capability specs |
| [docs/decisions.md](docs/decisions.md) | Every "why doesn't devcroft support X", answered falsifiably |
| [CLAUDE.md](CLAUDE.md) | Architecture invariants and repo conventions |

```sh
openspec list             # active changes and task progress
openspec validate --all   # validate delta specs
```
