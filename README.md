# devcroft

Isolated, reproducible development environments built on OS-level sandboxing
— kernel primitives instead of containers or VMs — each reachable over SSH.

Many environments, including fleets of coding agents, run in parallel on one
host at near-zero marginal cost: they share a single content-addressed Nix
store while each stays behind its own kernel-enforced boundary. Because every
sandbox speaks SSH, existing editors work unchanged.

devcroft implements no isolation itself. It is a policy compiler, a
supervisor, and an SSH endpoint over existing sandbox backends.

## How this compares

devcroft isn't a cheaper Docker. It's a bet that most day-to-day
development work doesn't need a full container boundary — it needs a
reproducible environment, a boundary good enough to catch accidents, and
SSH access that works with existing editors, at a marginal cost low enough
to run many side by side on one host.

|  | devcroft | Dev Containers / Docker | flox alone | mise/asdf + manual sandboxing |
|---|---|---|---|---|
| Isolation | Kernel primitives (Landlock/Seatbelt); `process` tier only in MVP — accident protection, not a security boundary (see Limitations) | A container boundary, today | None | Whatever you build yourself |
| Editor/SSH access | Native — a real SSH server per sandbox | Native, through the container | No | No |
| Reproducibility | Mandatory — no `host`/`none` fallback (see [docs/decisions.md](docs/decisions.md)) | Optional | Yes | Partial, depends what's pinned |
| Marginal cost per environment | Low — a shared Nix/flox store, no separate rootfs or guest kernel | Higher — image layers, often a VM on macOS | None | None |

That trade makes sense for fleets of coding agents, many parallel projects
on one host, or local CI — not for running code you don't trust at all,
where a real container or VM boundary is still the right call.

## Status

**MVP implementation underway — 23/25 tasks.** A `..` path-traversal gap in
task 2.1's `filesystem.allow`/`read`/`deny` validation, and a task 3.2
reproducibility gap where flox activation inherited whoever's shell ran
`up` (personal `PATH`, ad hoc env vars) instead of a fixed environment,
were both found and closed along the way — see the git history for each
fix. The fd-passing keeper trick
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
cross-editor validation matrix (6.5, nearly done — OpenSSH and rsync are
validated by real end-to-end tests, VS Code Remote-SSH and Cursor are
validated by real manual connections against a live sandbox, only Zed
remains (no CLI to drive it here) — see
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
--recreate`) refuse to run non-interactively without `--yes` (7.2). Two
sandboxes now have end-to-end coverage
running side by side with disjoint state and independently-enforced
policy, and a keeper survives a freeze/resume cycle (`SIGSTOP`/`SIGCONT`
on the keeper pid, the realistic proxy for host suspend/resume available
in this environment) with the next command transparently confirming
health rather than assuming it (7.3).

## Limitations

devcroft's default (and only implemented) tier, `process`, is Landlock or
Seatbelt applied to a process tree. **This is accident protection, not a
security boundary** — the full host kernel syscall surface stays reachable
from inside, so a kernel bug is an escape. A real boundary is the planned
`hardened` tier (gVisor or LiteBox, plus Landlock; see
[openspec/changes/add-hardened-tier/](openspec/changes/add-hardened-tier/)),
not yet implemented. Every isolation claim in this README and in `devcroft`'s
own output is scoped to `process` unless said otherwise.

Known gaps, published rather than hidden:

- **No inter-sandbox process visibility separation.** MVP has no PID, mount,
  or network namespace separation between sandboxes
  ([design.md](openspec/changes/add-mvp-core/design.md) Decision 5):
  two sandboxes on the same host can see each other's processes, and — since
  they share the host's network namespace — two sandboxes each binding the
  same port (e.g. both running a dev server on 3000) would conflict with
  `EADDRINUSE`. There is no conflict detection; reach a sandbox's services
  through SSH's `-L` forwarding rather than assuming host ports are
  exclusive to it. **Note this is currently moot under the default
  policy** — see the listening-socket gap below, where neither sandbox can
  bind in the first place.
- **`network` blocking also blocks *listening* sockets, including
  loopback.** The default policy denies `bind`+`listen` outright, so a dev
  server inside a sandbox cannot come up at all — `python3 -c "…bind(('127.0.0.1', 0))…"`
  fails with `Operation not permitted`. The `[network]` section reads as
  outbound egress control, and nothing says it revokes the ability to
  serve; the port-conflict note above is written as though binding works.
  Setting `network.default = "allow"` restores it, but that also drops
  egress filtering, so there is currently no way to express "no outbound
  access, but I can still run my dev server". This is what stops VS Code
  Remote-SSH (its server needs a loopback listener) — see
  [docs/ssh-validation.md](docs/ssh-validation.md).
- **Network filtering is cooperative and platform-dependent.** Domain-level
  allowlisting needs a cooperative proxy; macOS Seatbelt cannot enforce it
  at all without one. `doctor` and `up` name this degradation once, rather
  than silently granting broader network access than the manifest asked for.
- **No cgroup resource limits.** A runaway build in one sandbox can affect
  the whole host — nothing today caps CPU or memory per sandbox. Planned:
  cgroup v2 scope units per keeper on Linux; no macOS equivalent exists.
- **A `filesystem.allow` grant for a path that doesn't exist yet is
  silently dropped**, while `policy --render` still shows it as granted
  with its `manifest:` origin. The backend ignores grants whose target is
  missing when the profile is applied, so the rendered policy is not the
  policy in force — the one gap here that contradicts an invariant
  ("deterministic and inspectable", "degraded capabilities are surfaced,
  never silent") rather than just missing a feature. Create the directory
  before `up` as a workaround. Found during task 6.5.
- **Zed's remote server connects and transfers but does not start.** Its
  forked daemon exits without logging; not yet attributed to devcroft.
  Zed also needs five separate `$HOME` grants, one of which is the local
  editor's own data directory. VS Code and Cursor are unaffected. See
  [docs/ssh-validation.md](docs/ssh-validation.md).
`docs/decisions.md` has the falsifiable "why not X" reasoning behind most of
these; the ones above are gaps in what's actually built, not design
decisions.

| | |
|---|---|
| [openspec/changes/add-mvp-core/](openspec/changes/add-mvp-core/) | The MVP — proposal, design, tasks, 7 capability specs |
| [docs/decisions.md](docs/decisions.md) | Every "why doesn't devcroft support X", answered falsifiably |
| [docs/ssh-validation.md](docs/ssh-validation.md) | SSH client/editor validation matrix (task 6.5) — OpenSSH, rsync, VS Code Remote-SSH and Cursor validated against a live sandbox; Zed connects and transfers but its server does not start |
| [CLAUDE.md](CLAUDE.md) | Architecture invariants and repo conventions |
| [samples/flox-rustup-sample/](samples/flox-rustup-sample/) | A real, verified flox + rustup + devcroft project — and the 4 real sandboxing/toolchain frictions found building it |
| [samples/flox-clap-sample/](samples/flox-clap-sample/) | A clap-derive CLI sandboxed the same way — plus what changes once a sample has real crates.io dependencies |

```sh
openspec list             # active changes and task progress
openspec validate --all   # validate delta specs
```
