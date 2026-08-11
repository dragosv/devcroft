# SSH validation matrix (task 6.5)

Task 6.5 asks for a validation matrix across OpenSSH client, rsync, VS Code
Remote-SSH, Zed, and Cursor. This document is that matrix as it stands: what
has actually been run against a real keeper, and what hasn't — and why, so
the gap is explicit rather than silently assumed away.

The environment this was written in has no `rsync` binary, no passwordless
`sudo` to install one, and no way to run a GUI editor or its remote-SSH
extension. Those rows are **not validated** here. Everything below states
plainly which claims are backed by an automated test and which are backed
only by protocol-level reasoning — the second kind is not a substitute for
the first and should not be read as one.

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
| rsync | **Not validated** | no `rsync` binary available to test with (see below) |
| VS Code Remote-SSH | **Not validated** | GUI/remote-extension tool, cannot run here |
| Zed | **Not validated** | GUI tool, cannot run here |
| Cursor | **Not validated** | GUI/remote-extension tool, cannot run here |

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

Not installed in the environment this was validated in, and not
installable without a password this session doesn't have. The mechanism it
would use is already covered indirectly: `rsync -e ssh` runs `rsync
--server ...` on the remote end over a plain SSH **exec channel**, and
devcroft's exec channel (`ssh::server::exec_request`) already runs whatever
command string it's given via `sh -c` — the same path
`exec_channel_runs_commands_and_propagates_exit_code` exercises. Whether an
actual `rsync --server` invocation succeeds depends on an `rsync` binary
existing *inside* the sandbox, which comes from the project's own `flox`
environment, not from devcroft. That's a property of what's installed, not
of the SSH layer — and is exactly the kind of thing that needs a real
`rsync` binary on both ends to actually confirm, not just reasoned about.

**To validate manually:** with `rsync` installed on the host and inside a
project's flox environment, `rsync -e "ssh -F <config>" <src>
<name>.devcroft:<dst>` using the `ssh-config` block `devcroft ssh-config
--write` installs.

## VS Code Remote-SSH, Zed, Cursor

None of these can run in this environment (no display, no ability to
install/drive a GUI application or a remote-SSH extension programmatically).
What *is* validated is every protocol mechanism these editors are documented
to depend on for a remote-SSH connection:

- **Server bootstrap** (installing/running the editor's own remote server
  component): an exec channel running an arbitrary shell command — covered.
- **File operations**: SFTP — covered (`sftp_round_trips_a_file_and_lists_a_directory`
  exercises open/write/close, open/read/close, and opendir/readdir).
- **Integrated terminal**: a pty/shell channel with resize — covered
  (`shell_channel_allocates_a_pty_and_runs_commands`; window-change itself is
  additionally covered at the protocol level in `keeper::connection`'s own
  `resize_frame_mid_session_does_not_disrupt_the_session` test, since a real
  local pty to drive a genuine resize from this environment isn't available
  either).
- **Port forwarding** (e.g. previewing a dev server): `-L` direct-tcpip —
  covered (`direct_tcpip_forwarding_relays_a_real_connection`).
- **Agent forwarding off by default**: nothing in this implementation
  advertises or opens an agent channel unless asked, and no session's `env`
  ever sets `SSH_AUTH_SOCK` — there is no code path that could create one.

This is reasonable confidence that the underlying mechanisms editors rely on
work, not a substitute for actually opening each editor against a real
sandbox. That step needs a machine with the editor installed and is out of
reach here.

**To validate manually:** `devcroft ssh-config --write`, then point the
editor's Remote-SSH host picker (or Cursor/Zed's equivalent) at
`<name>.devcroft` for a sandbox that's already `up`. Record, per editor: did
the remote server install/start; do file open/save/browse work; does the
integrated terminal work including resize; does port forwarding for a
locally-previewed dev server work.

## What would close this out

1. Install `rsync` (host + a flox environment that includes it) and run an
   actual transfer through `devcroft proxy`.
2. Open each of VS Code Remote-SSH, Zed, and Cursor against a real `up`
   sandbox using the `ssh-config --write` block, and record pass/fail per
   the checklist above.
3. Fold both into the table at the top of this document, replacing "Not
   validated" with the actual result and a date.
