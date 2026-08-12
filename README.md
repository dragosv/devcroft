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

**MVP implementation underway — 22/25 tasks.** A path-traversal gap in
task 2.1's filesystem validation was found and closed along the way: a
`..` segment in `filesystem.allow`/`read`/`deny` (e.g. `../../etc`) passed
validation silently and, since devcroft hands manifest path strings to
`nono` unresolved with the project root as its cwd, actually granted
access outside the project root — confirmed against a real `nono` profile
before the fix. Rejected now with `ConfigError::InvalidPath` regardless of
which root (project-relative, `~`, absolute) it appears under, since `..`
also breaks the containment model every other filesystem check in that
requirement depends on. The fd-passing keeper trick
(spike binary, task group 1) is proven on both Linux/Landlock and
macOS/Seatbelt; the config/policy compiler, the environment provider layer
(`flox` resolution, task group 3), the keeper's spawn protocol (control
socket, session registry, pty allocation — task 4.1), the supervisor
(`up`/`down`/`rm`, idempotent with crash recovery and `--recreate` — task
4.2), read-only sandbox introspection (`status`/`logs`/`ps` — task 4.3), and
sessions — task group 5 in full: one-shot `exec` with exit-code
propagation, cwd mapping, and signal forwarding (5.1); interactive `shell`
with a real pty, resize propagation, and a `$SHELL`-then-`/bin/sh` fallback
(5.2); and auto-up (`exec`/`shell` bring a cold sandbox up themselves unless
`--no-up`, 5.3) — are implemented and tested end to end against real `nono`
and `flox`. Task group 6 (SSH endpoint) is now complete except for its
cross-editor validation matrix (6.5, partially done — see
[docs/ssh-validation.md](docs/ssh-validation.md)): an SSH server (russh)
embedded in the
keeper on a second unix socket, mode 0600 in the state dir's mode 0700, bound
host-side and fd-passed the same way the control socket is, with publickey
auth against the devcroft client keypair and a fresh ephemeral host key per
`up` (6.1); `devcroft proxy <name>.devcroft` and `devcroft ssh-config
[--write]` (6.2); and full channel support (6.3/6.4) — exec, pty/shell with
resize and an env allowlist (`TERM`/`LANG`/`LC_*`), exit status, the `sftp`
subsystem (also what modern `scp` speaks by default), and `-L` direct-tcpip
forwarding gated by nothing devcroft-specific — it just lets the sandbox's
own network restriction accept or reject the target, same as every other
syscall the keeper makes. All of it is tested against the real `ssh`/`scp`/
`sftp` CLIs through a real `devcroft proxy` subprocess, not just a russh test
client. Task group 7 (CLI polish & release) is well underway: `devcroft
init` detects an existing flox environment or a bare single-ecosystem
toolchain pin (`rust-toolchain.toml`/`.nvmrc`/`.python-version`) and
generates a minimal manifest without ever overwriting one without
`--force`; its default sandbox name (the directory slug) is disambiguated
with a short path-derived suffix only on a real collision against another
project's already-existing state — e.g. two unrelated projects both named
`api` — so the common case keeps the plain slug and only a genuine clash
gets a suffix; `devcroft doctor` reports backend presence/version-range, kernel
sandboxing capability, the provider binary, `ssh-config` managed-section
state, and (when a manifest is discoverable) which of its aspects would be
degraded on this host, with every `FAIL` naming its fix (7.1). The rest of
the command surface is wired up too, each with the stable 0–5 exit codes
and layer-named errors the cli spec's error contract requires: `up`
(idempotent, `--recreate`), `down`, `rm`, `status`, `logs`, `ps`, `policy
--render`, `why --path`/`--host`, and `ssh` (execs a real system `ssh` with
the right options pre-filled). Destructive operations (`rm`, `up
--recreate`) refuse to run non-interactively without `--yes` (7.2). One gap
surfaced along the way: the lifecycle spec's `hooks.post_create`/
`hooks.post_start` execution isn't implemented — the manifest parses them,
but nothing runs them yet. Two sandboxes now have end-to-end coverage
running side by side with disjoint state and independently-enforced
policy, and a keeper survives a freeze/resume cycle (`SIGSTOP`/`SIGCONT`
on the keeper pid, the realistic proxy for host suspend/resume available
in this environment) with the next command transparently confirming
health rather than assuming it (7.3). User-facing documentation is
written at release (task 7.4); until then the specs are the source of
truth.

| | |
|---|---|
| [openspec/changes/add-mvp-core/](openspec/changes/add-mvp-core/) | The MVP — proposal, design, tasks, 7 capability specs |
| [docs/decisions.md](docs/decisions.md) | Every "why doesn't devcroft support X", answered falsifiably |
| [docs/ssh-validation.md](docs/ssh-validation.md) | SSH client/editor validation matrix (task 6.5) — what's actually been tested and what still needs a real editor or `rsync` |
| [CLAUDE.md](CLAUDE.md) | Architecture invariants and repo conventions |

```sh
openspec list             # active changes and task progress
openspec validate --all   # validate delta specs
```
