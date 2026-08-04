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

**MVP implementation underway — 9/25 tasks.** The fd-passing keeper trick
(spike binary, task group 1) is proven on both Linux/Landlock and
macOS/Seatbelt; the config/policy compiler and the environment provider
layer (`flox` resolution, task group 3) are implemented and tested. There
is no `devcroft` CLI binary, keeper process, or SSH endpoint yet — those
are task groups 4 through 6. User-facing documentation is written at
release (task 7.4); until then the specs are the source of truth.

| | |
|---|---|
| [openspec/changes/add-mvp-core/](openspec/changes/add-mvp-core/) | The MVP — proposal, design, tasks, 7 capability specs |
| [docs/decisions.md](docs/decisions.md) | Every "why doesn't devcroft support X", answered falsifiably |
| [CLAUDE.md](CLAUDE.md) | Architecture invariants and repo conventions |

```sh
openspec list             # active changes and task progress
openspec validate --all   # validate delta specs
```
