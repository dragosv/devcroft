# SSH validation matrix (task 6.5)

Task 6.5 asks for a validation matrix across OpenSSH client, rsync, VS Code
Remote-SSH, Zed, and Cursor. This document is that matrix as it stands: what
has actually been run against a real keeper, and what hasn't — and why, so
the gap is explicit rather than silently assumed away.

Everything below states plainly which claims are backed by an automated test
and which are backed only by protocol-level reasoning — the second kind is
not a substitute for the first and should not be read as one.

## Status

| Client | Status | Evidence |
|---|---|---|
| OpenSSH `ssh` (exec) | **Validated** | `tests/ssh_channels.rs::exec_channel_runs_commands_and_propagates_exit_code` |
| OpenSSH `ssh -tt` (pty/shell) | **Validated** | `tests/ssh_channels.rs::shell_channel_allocates_a_pty_and_runs_commands` |
| OpenSSH env forwarding (`SetEnv`) | **Validated** | `tests/ssh_channels.rs::env_allowlist_passes_term_and_lang_but_not_arbitrary_vars` |
| OpenSSH `sftp` | **Validated** | `tests/ssh_channels.rs::sftp_round_trips_a_file_and_lists_a_directory` |
| OpenSSH `scp` (SFTP-based, 9.0+) | **Validated** (data correctness only — see note) | `tests/ssh_channels.rs::scp_moves_correct_data_even_though_its_own_exit_code_is_unreliable_here` |
| OpenSSH `scp -O` (legacy) | **Spot-checked manually**, not in the automated suite | needs a real `scp` binary *inside* the sandbox, which depends on the project's own environment, not devcroft |
| `-L` local forwarding | **Validated** | `tests/ssh_channels.rs::direct_tcpip_forwarding_relays_a_real_connection` |
| `ProxyCommand`/`ssh-config` plumbing | **Validated** | `tests/proxy_up.rs`, `tests/ssh_up.rs`, `tests/ssh_config_cli.rs` |
| rsync | **Validated** (2026-08-14, macOS/aarch64-darwin, system `openrsync`) | `tests/ssh_channels.rs::rsync_transfers_a_file_through_devcroft_proxy_over_a_plain_exec_channel` |
| VS Code Remote-SSH | **Validated manually** (2026-08-14, macOS/aarch64-darwin, VS Code 1.130.0, remote-ssh 0.124.0) | real connection, publickey auth, remote server install, extension host launch against a live `up` sandbox — see below |
| Cursor (remote-ssh) | **Validated manually** (2026-08-14, macOS/aarch64-darwin, Cursor 3.15.19, anysphere.remote-ssh 1.1.14) | real connection, server download/install/start, multiplex + code server both listening — see below |
| Zed | **Not validated** | installed but has no CLI on `PATH` in this environment to drive it non-interactively; not attempted |

All "Validated" rows go through the real `ssh`/`scp`/`sftp` CLI binaries
talking to a real, `up`-started keeper via a real `devcroft proxy`
subprocess as `ProxyCommand` — not a russh test client standing in for a
real one. See `tests/ssh_channels.rs`'s module doc for why that distinction
matters.

## The `scp` exit-code note

`scp_moves_correct_data_even_though_its_own_exit_code_is_unreliable_here`
is named that way on purpose. Modern (non-`-O`) OpenSSH `scp` speaks SFTP
under the hood, through the same channel and the same `ssh::sftp::FsHandler`
every other SFTP client uses — and the file content transferred is always
byte-correct, checked directly by that test. What's specifically unreliable
is `scp`'s own process exit code: it depends on `scp`'s internal `ssh -s
sftp` child process receiving the channel-level exit-status request before
it stops listening, and a real `/usr/lib/openssh/sftp-server` subprocess
tends to win that race because it exits synchronously as the kernel reaps
it. `ssh::server::FsHandler` has no such subprocess to synchronize
against — `russh_sftp::server::run()` returns as soon as it *starts* the
session, not when it ends (see `ssh::server::subsystem_request`'s doc
comment) — so devcroft can detect completion correctly (via a stream-EOF
signal) but still can't reliably win that specific race. `sftp` itself
doesn't have this problem, which is exactly what the "Evidence" column
above shows: the `sftp` row transfers *and* reports success; the `scp` row
transfers correctly but isn't asserted on its own exit code for that
reason.

## rsync

Validated 2026-08-14 on macOS/aarch64-darwin, against a real `up`-started
keeper via a real `devcroft proxy` subprocess:
`tests/ssh_channels.rs::rsync_transfers_a_file_through_devcroft_proxy_over_a_plain_exec_channel`
uploads a file, downloads it back, and asserts the bytes round-trip
correctly on both legs.

`rsync -e ssh` runs `rsync --server ...` on the remote end over a plain SSH
**exec channel**, and devcroft's exec channel (`ssh::server::exec_request`)
already runs whatever command string it's given via `sh -c` — the same
path `exec_channel_runs_commands_and_propagates_exit_code` exercises, so
this needed no rsync-specific server support. What it *did* need was a real
`rsync` binary reachable on both ends: on this host, that's the system
`openrsync` at `/usr/bin/rsync` (macOS's built-in rsync, not GNU rsync —
the test doesn't depend on which implementation, only that a `--server`
invocation round-trips correctly), reachable *inside* the sandbox because
the flox-activated `PATH` the keeper inherits still contains the canonical
system bin dirs (`provider::flox::CANONICAL_PATH`) even though this
project's own flox environment doesn't install rsync itself. A project
whose flox environment's activation strips or replaces `PATH` entirely
would need its own `rsync` package instead — that's a property of what's
installed, same as any other tool this sandbox runs.

Getting this far surfaced a real bug, now fixed: `lifecycle::up`'s
`spawn_keeper` looked up `nono` by bare name (`Command::new("nono")`)
*after* the command's own environment had already been overridden to the
provider-resolved (flox-activated) env, whose `PATH` has no reason to
contain wherever the host actually installed `nono` — on this Mac,
Homebrew's `/opt/homebrew/bin`, which the fixed/activated `PATH` doesn't
include. `up` failed outright with `ENOENT` before this fix, on every
project, on this class of host. Fixed the same way `provider::flox`
already resolves `flox` itself: look `nono` up against *this process's*
own ambient `PATH` before the child's environment is replaced (now shared
as `paths::resolve_on_path`). Two macOS-only test bugs surfaced once `up`
started succeeding here — `tests/exec_up.rs` and
`tests/concurrency_and_suspend.rs` compared a project root built from
`std::env::temp_dir()` (`/var/...`) against `pwd` output from inside the
sandbox, which resolves macOS's `/var` → `/private/var` symlink; both now
canonicalize the expected path up front.

**To validate manually against a live sandbox:** with `rsync` installed on
the host and inside a project's flox environment, `rsync -e "ssh -F
<config>" <src> <name>.devcroft:<dst>` using the `ssh-config` block
`devcroft ssh-config --write` installs.

## VS Code Remote-SSH and Cursor: real, manually-run connections

Both validated 2026-08-14 on macOS/aarch64-darwin against a live `up`
sandbox, `devcroft ssh-config --write`'s real `~/.ssh/config` block, and
each editor's own installed remote-ssh extension (`ms-vscode-remote.remote-ssh`
for VS Code, `anysphere.remote-ssh` for Cursor — a fork of the same
extension, same underlying protocol) driven non-interactively via
`code --remote ssh-remote+<name>.devcroft <path>` / `cursor --remote ...`.
Both editors' own connection logs (`~/Library/Application Support/{Code,Cursor}/logs/.../Remote - SSH.log`)
confirm: SSH handshake, `Authenticated ... using "publickey"`, remote server
download+install+start, and (VS Code) an extension host launching against
the remote workspace with a workbench window open, or (Cursor) both its
multiplex and code servers reachable and pinged repeatedly over the
forwarded connection.

**The one real finding, and it's devcroft working as designed, not a bug:**
both editors default their remote server's install directory to `$HOME`
(`remote.SSH.serverInstallPath`'s default). devcroft's default filesystem
policy only grants write access to the project root, not `$HOME` — exactly
the intended behavior (CLAUDE.md's policy is deterministic and
inspectable; nothing widens without an explicit grant) — so the first
attempt against each editor failed with `mkdir: $HOME/.vscode-server:
Operation not permitted` / the Cursor equivalent. Setting
`"remote.SSH.serverInstallPath": {"<name>.devcroft": "<project
root>/.vscode-server"}` (or `.cursor-server`) in the editor's own
`settings.json`, redirecting the install to somewhere inside the granted
project root, fixed both on the next attempt with no changes to the
manifest or the sandbox's policy. Cursor's installer script separately hit
(and gracefully degraded around) two more denials from the same default
policy: `bash: ~/.profile: Operation not permitted` (outside the project
root, correctly denied) and `bash: /bin/ps: Operation not permitted` (its
own "is a server already running" liveness check) — neither stopped the
connection from completing, since Cursor's own script treats a failed `ps`
as "nothing running yet" rather than a fatal error.

**Protocol mechanisms both connections exercised**, matching what's also
covered by the automated suite independently:

- **Server bootstrap**: an exec channel running an arbitrary shell command
  (`ssh::server::exec_request`) — same path as
  `exec_channel_runs_commands_and_propagates_exit_code`.
- **File operations**: the workbench window (VS Code) opening the project
  folder and Cursor's forwarded code-server connection both depend on the
  same SFTP-equivalent file API surface `sftp_round_trips_a_file_and_lists_a_directory`
  exercises directly.
- **Long-lived forwarded connections**: both editors tunnel their own
  protocol over the SSH connection (VS Code via `-D` dynamic SOCKS
  forwarding, Cursor via its multiplex server reached the same way) and
  keep it alive with periodic pings — a sustained, bidirectional exec/relay
  session, not a one-shot command.

Not independently re-verified in this pass (already covered by the
automated suite, not by these two manual runs specifically): the
integrated terminal (pty/shell + resize) and `-L` local port forwarding for
previewing a dev server. Both use the same channel types already exercised
above and by `shell_channel_allocates_a_pty_and_runs_commands` /
`direct_tcpip_forwarding_relays_a_real_connection`, so there's no reason to
expect them to behave differently through a real editor than through the
real `ssh` CLI both already went through — but "no reason to expect
otherwise" is reasoning, not a re-run, so it's called out here rather than
folded silently into "Validated".

## Zed

Not attempted: `Zed.app` is installed on the host used for this pass, but
Zed has no CLI on `PATH` to drive a remote connection non-interactively
(unlike `code`/`cursor`, which both install a shell command). Zed's remote
development is also a materially different mechanism from the other two —
not a bundled VS Code-style remote-ssh extension, but Zed's own protocol
running a headless `zed` binary on the remote end — so the VS Code/Cursor
results above are not strong evidence either way for Zed specifically.

**To validate manually:** `devcroft ssh-config --write`, then add
`<name>.devcroft` as a remote project in Zed's own remote-development UI
for a sandbox that's already `up`. Expect the same `serverInstallPath`-style
friction if Zed's remote server also defaults to installing under `$HOME`;
redirect it inside the project root the same way. Record: did the remote
server install/start; do file open/save/browse work; does the integrated
terminal work including resize; does port forwarding for a locally-previewed
dev server work.

## What would close this out

1. Open Zed against a real `up` sandbox using the `ssh-config --write`
   block, following the same install-path pattern VS Code/Cursor needed,
   and record pass/fail per the checklist above.
2. Fold the result into the table at the top of this document, replacing
   "Not validated" with the actual result and a date.
3. Consider whether `serverInstallPath`-under-`$HOME`-by-default is common
   enough across remote-dev tooling that devcroft's own docs (not just this
   validation matrix) should call it out as a setup step for editor users —
   it will recur for any tool that defaults its remote install/cache
   directory to `$HOME` instead of the project root.
