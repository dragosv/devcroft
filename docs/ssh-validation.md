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
| SFTP relative-path base | **Known divergence** from OpenSSH: project root, not `$HOME` | see "Zed" finding 2 |
| `ProxyCommand`/`ssh-config` plumbing | **Validated** | `tests/proxy_up.rs`, `tests/ssh_up.rs`, `tests/ssh_config_cli.rs` |
| rsync | **Validated** (2026-08-14, macOS/aarch64-darwin, system `openrsync`) | `tests/ssh_channels.rs::rsync_transfers_a_file_through_devcroft_proxy_over_a_plain_exec_channel` |
| VS Code Remote-SSH | **Validated manually** (2026-08-14, macOS/aarch64-darwin, VS Code 1.130.0, remote-ssh 0.124.0) | real connection, publickey auth, remote server install, extension host launch against a live `up` sandbox — see below |
| Cursor (remote-ssh) | **Validated manually** (2026-08-14, macOS/aarch64-darwin, Cursor 3.15.19, anysphere.remote-ssh 1.1.14) | real connection, server download/install/start, multiplex + code server both listening — see below |
| Zed | **Attempted, blocked** (2026-08-15, macOS/aarch64-darwin, Zed 1.4.4) | real connection, publickey auth, remote server uploaded byte-correct — then blocked by devcroft's `scp` exit-status race; see below |

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

**Severity note, added 2026-08-15:** this was written off as cosmetic
because no test cared about the exit code. The Zed pass below shows it is
not — a real client that gates on `scp`'s exit code cannot connect, even
though every byte arrives. See "Zed" for the evidence.

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

## Zed: attempted against a live sandbox, blocked by devcroft

Attempted 2026-08-15 on macOS/aarch64-darwin, Zed 1.4.4, driven
non-interactively as `zed ssh://<name>.devcroft/<project path>` (Zed's CLI
*is* on `PATH` on this host — `/usr/local/bin/zed` — contrary to the earlier
note here, which is why this pass got further than "not attempted").

Zed's remote development is its own protocol running a headless
`zed-remote-server` binary on the remote end, not a VS Code-style remote-ssh
extension, and it has **no `serverInstallPath` equivalent**: the server
directory is hard-coded as `.zed_server` relative to `cd` with no argument,
i.e. `$HOME/.zed_server`. The VS Code/Cursor fix therefore does not
transfer — there is no setting to redirect it.

**How far it got, in order:** SSH handshake and publickey auth succeeded;
remote platform correctly discovered (`RemotePlatform { os: MacOs, arch:
Aarch64 }`); `curl` of the server tarball correctly **denied** by the
sandbox's `network.block` (working as designed), so Zed fell back to
downloading locally and uploading; the full 31,618,399-byte server binary
**was uploaded byte-correct into the sandbox**; and then the connection
failed anyway.

Three devcroft findings came out of this, in the order they blocked the
connection. The first two have workarounds; the third does not.

### 1. A grant for a path that does not exist yet is silently dropped

`mkdir -p .zed_server` failed with `Operation not permitted` even with
`filesystem.allow = [".", "~/.zed_server"]` in the manifest and
`policy --render` showing the grant with origin `manifest:filesystem.allow`.
The backend ignores it: nono drops `filesystem.allow` entries whose path
does not exist when the profile is applied, and says so only on stdout
(`'~/.zed_server' does not exist and will be ignored.`). Verified directly
against `nono why` with the same profile — granting an existing directory
is `ALLOWED`, granting a nonexistent one is `DENIED` with
`Reason: path_not_granted`, and a *child* of an existing granted directory
is `ALLOWED`. So the grant target itself must exist at `up` time; paths
underneath it need not.

This is the one finding here that contradicts a stated invariant rather
than merely being awkward: `policy --render` shows a rule that the backend
is not enforcing, so the rendered policy is not what is in force. That
crosses both "policy is deterministic and inspectable" and "degraded
capabilities are surfaced, never silent". **Workaround:** create the
directory on the host before `up`. **Real fix:** devcroft should detect a
grant whose path is missing and either fail, warn, or create it — not
render it as if it were live.

### 2. SFTP resolves relative paths against the project root, not `$HOME`

Zed creates the server directory over an **exec** channel (`cd; mkdir -p
.zed_server` → `$HOME/.zed_server`) and then uploads into it over **SFTP**
with a *relative* destination (`.zed_server/<binary>.gz`). Those two land in
different places:

| Channel | Base directory |
|---|---|
| exec, bare `cd` | `/Users/dragos` (`$HOME`) |
| SFTP, relative path | `…/samples/flox-clap-sample` (project root) |

`ssh::sftp::FsHandler` passes paths straight to `std::fs`, so relative
paths resolve against the keeper's own cwd, which `lifecycle::up::spawn_keeper`
sets to the project root. A real OpenSSH server starts *both* at the user's
home directory, so any client that creates a directory over exec and then
writes into it over SFTP by relative path breaks here. Zed is such a client;
`scp`/`sftp`/rsync as driven by the automated tests all use absolute or
explicitly-rooted paths and so never noticed.

Whether devcroft *should* match OpenSSH here is a real design question, not
an obvious bug — the project root is the granted directory and is the more
useful default for a human running `sftp <name>.devcroft`, whereas `$HOME`
is denied by the default policy, so matching OpenSSH would drop users into
a directory they cannot read. Left as-is and documented rather than changed
unilaterally.

**Workaround, and it is a good one:** point `$HOME/.zed_server` at a
directory inside the project root
(`ln -s <project root>/.zed_server ~/.zed_server`). Both channels then
converge on the same real directory, and the server binary lands inside
already-granted space instead of requiring a `$HOME` write grant. The
manifest still needs `~/.zed_server` in `filesystem.allow` — verified
by removing it, after which `mkdir` fails again: the sandbox must be able to
traverse the symlink itself, which lives in `$HOME`. With both in place,
`mkdir -p .zed_server`, `touch`, and a real `scp` upload of a 3 MB file all
succeed and land in the project root.

### 3. `scp`'s exit status is not delivered — and this one actually blocks

With 1 and 2 worked around, Zed uploaded the entire server binary correctly
and *still* failed:

```
Failed to open project: uploading server binary: failed to upload file via STFP/SCP
  …/1.4.4.gz -> .zed_server/zed-remote-server-…-download-9455.gz:
```

— note the empty message after the final colon. Zed gates its next step on
the `scp` process's exit code, and gets a non-zero one with no stderr.
Confirmed directly, uploading a 3,000,000-byte random file over the same
path: **exit code 1, empty stderr, and identical SHA-256 on both ends.**

This is the exact race already documented above under "The `scp` exit-code
note" — but that section judges it cosmetic ("data correctness only"), and
that judgement is now wrong. It is not cosmetic: it is the single thing
standing between Zed and a working connection, because a real client treats
`scp`'s exit code as the success signal. The severity should be read as
"blocks at least one real editor", not "an exit code you can ignore".
Closing this out means making `ssh::server::subsystem_request` deliver the
channel exit-status before the client stops listening — the same fix the
`scp` note describes as hard, now with a concrete reason to do it.

## What would close this out

1. Fix finding 3 (SFTP/`scp` exit-status delivery). Zed is otherwise fully
   functional up to that point — auth, policy enforcement, and a
   byte-correct 31 MB transfer all work — so this is plausibly the only
   thing between here and a Zed row that says "Validated".
2. Re-run `zed ssh://<name>.devcroft/<path>` with the symlink workaround in
   place and record: does the remote server start; do file open/save/browse
   work; does the integrated terminal work including resize; does port
   forwarding work.
3. Decide what devcroft does about finding 1 (missing-path grants rendered
   as live but silently dropped by the backend) — that one is a correctness
   bug in the policy surface independent of any editor.
4. Consider whether the remote-server-directory-under-`$HOME` pattern
   deserves a first-class answer in devcroft's own docs rather than a
   per-editor workaround, since all three editors tested hit some version
   of it — VS Code and Cursor via a redirectable setting, Zed via a
   hard-coded path that needs a symlink.
