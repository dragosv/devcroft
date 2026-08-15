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
| OpenSSH `scp` (SFTP-based, 9.0+) | **Validated** (data *and* exit code, both directions) | `tests/ssh_channels.rs::scp_round_trips_correct_data_and_reports_success` |
| OpenSSH `scp -O` (legacy) | **Spot-checked manually**, not in the automated suite | needs a real `scp` binary *inside* the sandbox, which depends on the project's own environment, not devcroft |
| `-L` local forwarding | **Validated** | `tests/ssh_channels.rs::direct_tcpip_forwarding_relays_a_real_connection` |
| SFTP relative-path base | **Known divergence** from OpenSSH: project root, not `$HOME` | see "Zed" finding 2 |
| `ProxyCommand`/`ssh-config` plumbing | **Validated** | `tests/proxy_up.rs`, `tests/ssh_up.rs`, `tests/ssh_config_cli.rs` |
| rsync | **Validated** (2026-08-14, macOS/aarch64-darwin, system `openrsync`) | `tests/ssh_channels.rs::rsync_transfers_a_file_through_devcroft_proxy_over_a_plain_exec_channel` |
| VS Code Remote-SSH | **Validated under three named preconditions** (2026-08-15 second retest, macOS/aarch64-darwin, VS Code 1.130.0) | live remote window, extension host connected. Does **not** work under the default policy: needs `serverInstallPath`, `network.default = "allow"`, and a `$TMPDIR` short enough for a unix socket path — see below |
| Cursor (remote-ssh) | **Validated manually 2026-08-14; not retested, treat as suspect** (Cursor 3.15.19, anysphere.remote-ssh 1.1.14) | its servers listen on ports, so the listening-socket blocker found for VS Code on 2026-08-15 very likely applies — see below |
| Zed | **Partial — connects, transfers, does not start** (2026-08-15, macOS/aarch64-darwin, Zed 1.4.4) | auth, upload, decompress and chmod of the 31 MB server all succeed; its forked server daemon then exits silently. Needs 5 `$HOME` grants. See below |

All "Validated" rows go through the real `ssh`/`scp`/`sftp` CLI binaries
talking to a real, `up`-started keeper via a real `devcroft proxy`
subprocess as `ProxyCommand` — not a russh test client standing in for a
real one. See `tests/ssh_channels.rs`'s module doc for why that distinction
matters.

## The `scp` exit-code note — resolved 2026-08-15

**This section used to describe an unfixable race. It was fixed instead.**
`scp` now reports success, and
`tests/ssh_channels.rs::scp_round_trips_correct_data_and_reports_success`
asserts the exit code in both directions.

The original diagnosis was wrong in an instructive way, so it is kept here
rather than deleted. It held that `scp` derives its exit code from its
internal `ssh -s sftp` child, which must receive the channel-level
exit-status before it stops listening; that a real
`/usr/lib/openssh/sftp-server` wins that race by exiting synchronously as
the kernel reaps it; and that `FsHandler`, having no such subprocess, could
not reliably win it. Every step of that is about **timing**, and the
conclusion followed from treating timing as the only variable.

The actual mechanism was **ordering**, which the server controls
completely. `russh_sftp`'s request loop ends the moment it sees EOF and
drops the channel; dropping a russh channel sends `close`; and no client
will accept an `exit-status` that arrives after `close`. The exit-status
was being sent from a `tokio::spawn`, so it was racing that drop — and
losing. Captured directly from `scp -vvv` before the fix:

```
debug2: channel 0: rcvd close     <- no exit-status ahead of it
debug1: Exit status -1            <- so scp reports failure
```

`ssh::server::NotifyOnEof` now withholds the EOF from `russh_sftp` until
the exit-status future has resolved, which keeps the stream — and so the
channel — alive across the send. `close` then cannot overtake
`exit-status`, by construction rather than by timing. The same capture
after the fix ends `debug1: Exit status 0`, and a 3 MB upload, a download,
and an `sftp` put all report success with matching SHA-256 on both ends.

The lesson worth keeping: "a real subprocess wins this race" was a
plausible story that explained the symptom and pointed away from the fix.
It went unchallenged because no test asserted the exit code — the
assertion was omitted *because* of the belief it could not pass.

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

## VS Code Remote-SSH: three blockers, all now identified

Retested twice on 2026-08-15, on the same host and the **same VS Code
build** (1.130.0, commit `1b6a188`) that the 2026-08-14 section below
calls validated. The first retest found two blockers and left a third
misdiagnosed as an unexplained VS Code-side hang; the second retest found
what that third one actually was. With all three worked around, VS Code
Remote-SSH **works**: a live remote window (`Welcome — flox-clap-sample
[SSH: flox-clap-sample.devcroft]`), extension host agent listening,
management and extension-host connections established, and no errors left
in the connection log.

None of the three is visible under the default policy, and only one of
them was previously written down, which is why the 2026-08-14 "Validated"
claim should not be trusted as written.

**Blocker 1 — the install directory (known).** With no
`remote.SSH.serverInstallPath` set, VS Code reports `CreateInstallDirFailed`,
because it defaults the directory to `$HOME` and devcroft correctly denies
writing there. Redirecting it into the project root fixes this, exactly as
described below. Worth noting the setting was *not* present in this host's
`settings.json` at retest time, so whatever was configured for the original
pass did not persist — which is itself a reason the original result was not
reproducible.

**Blocker 2 — listening sockets (new, and the important one).** Past the
install directory, the server dies immediately:

```
info  No agent host supervisor running; starting one in the background
error error listening on port: Operation not permitted (os error 1)
```

devcroft's default `network.block` denies `bind`+`listen`, loopback
included. Confirmed independently of VS Code, straight through an exec
channel:

| Policy | `bind(("127.0.0.1", 0)); listen()` |
|---|---|
| default (`network.block: true`) | `[Errno 1] Operation not permitted` |
| `network.default = "allow"` | `LISTEN OK ('127.0.0.1', 52053)` |

This is not really an editor finding at all — **no dev server can bind a
port inside a sandbox under the default policy.** The README's own
port-conflict note is written as though two sandboxes race for port 3000
and one gets `EADDRINUSE`; under the default policy neither binds and both
get `EPERM`. The `[network]` section reads as outbound egress control and
says nothing about revoking the ability to serve, so this is a policy
*granularity* gap as much as a documentation one: there is currently no
way to express "no outbound access, but my dev server can still listen",
which is the single most common shape of local development.

**Blocker 3 — a unix socket path over macOS's 104-byte limit (new).**
Past both of those, VS Code authenticates, downloads and extracts the full
server (543 MB on disk), reaches `Starting server...`, and then dies every
time on:

```
[server] Error occurred in server
[server] Error: listen EINVAL: invalid argument
         /Users/dragos/Development/devcroft/samples/flox-clap-sample/.tmp/code-ab7bbd8c-724a-4cc8-8168-f9f6f2b1a866
```

`EINVAL`, not `EPERM` — so this is not a policy denial at all. `code-server
--socket-path=$TMPDIR/code-<uuid>` binds a unix domain socket, and macOS
caps `sun_path` at 104 bytes including the NUL. That path is 106. The
budget is unforgiving and entirely mechanical: `code-` plus a 36-character
UUID plus a separator is 42 bytes, so **`$TMPDIR` must be ≤ 61 characters**
or VS Code's remote server cannot start.

Confirmed straight through an exec channel, independently of VS Code:

| Socket path length | `bind()` in `$TMPDIR` |
|---|---|
| 106 chars (`code-<uuid>`) | `AF_UNIX path too long` |
| 88 chars | binds and listens |

**Where the long `$TMPDIR` came from, and it was ours.** This sample's own
flox `on-activate` exported `TMPDIR="$FLOX_ENV_PROJECT/.tmp"` — 64
characters, 3 over budget on its own. The comment justifying it reasoned
by analogy from `CARGO_HOME`: the host location is outside the project, so
the sandbox must deny it. That analogy is false, and probing the running
sandbox settles it:

| Path | Writable inside the sandbox |
|---|---|
| `/tmp` | yes |
| host `$TMPDIR` (`/var/folders/…/T`) | yes |
| `~/.cargo` | denied |
| `$HOME` | denied |

So the `CARGO_HOME` redirect is necessary and the `TMPDIR` redirect never
was — it bought nothing and cost 64 bytes of a 104-byte budget. Removed
from the sample's manifest; `devcroft exec -- cargo build` still works
without it, and with the host `$TMPDIR` restored the socket path drops to
91 bytes and VS Code connects.

**The lesson is the same shape as the `scp` one above.** The first retest
reported the symptom as "its agent host supervisor never comes up" and
declined to attribute it to devcroft. That was wrong in both directions:
the supervisor message was incidental (`agentHostProxy` is *expected* to
be unavailable — `no --agent-host-bridge-port set`), and the real cause
*was* ours, in a sample manifest, three lines from a comment explaining
why it was there. Reading the error text one line further down — `EINVAL`
rather than `EPERM` — was the whole diagnosis.

**Generalise it, because it is not a VS Code quirk.** Every devcroft
project is tempted to redirect `$TMPDIR` into the project root, and every
deep project root then breaks any tool that puts a socket in `$TMPDIR`.
devcroft already knows about this limit for its own sockets
(`lifecycle/state.rs` hashes state paths precisely to stay under it) but
says nothing about it to users. A `doctor` check — "`$TMPDIR` is N
characters; unix sockets created in it will fail above 103" — would have
turned two retests into one glance.

**On the 2026-08-14 section below.** It is left intact rather than
rewritten, because it records what was observed at the time and the
discrepancy is more useful than a tidy overwrite. But its "Validated"
claim omits preconditions that are load-bearing, and that omission is
exactly the kind of thing this document's own preamble warns about: a
manual result, unlike a test, does not re-run itself and cannot tell you
when it has stopped being true.

### Reproducing the working configuration

1. `"remote.SSH.serverInstallPath": {"<name>.devcroft": "<project
   root>/.vscode-server"}` in VS Code's own `settings.json`.
2. `network.default = "allow"` in `devcroft.toml` (blocker 2 — nothing
   less will do today, since there is no way to permit a loopback bind
   without permitting egress).
3. A `$TMPDIR` of ≤ 61 characters that the sandbox can write — the host
   default qualifies; a project-rooted one usually does not.

Then `code --remote ssh-remote+<name>.devcroft <project root>`.

**One lifecycle observation, noted but not chased.** The failed runs left
about ten `code-server` and `agent host` processes alive, and they
survived a full `devcroft down` + `up --recreate` — VS Code daemonises
them off the exec channel, so they outlive the keeper that spawned them.
Whether devcroft should reap a detached grandchild at teardown is a real
question and is not answered here.

## VS Code Remote-SSH and Cursor: the original 2026-08-14 pass

(Superseded for VS Code by the retest above; kept as the record of what
was observed then. Cursor has not been retested and its row still stands
as written — though note it shares VS Code's need for a listening socket,
so the same blocker 2 very likely applies to it too.)

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

## Zed: connects and transfers; its server does not come up

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
**was uploaded byte-correct into the sandbox**, decompressed and made
executable; and then its forked server daemon exited without ever coming
up.

Four findings came out of this, in the order they blocked the connection.
One was a real devcroft bug and is **fixed** (finding 3, the `scp`
exit-status ordering — the single most valuable thing this pass produced).
Two have workarounds and are still open. The fourth is where it stands
today and is not attributed to devcroft.

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

### 3. `scp`'s exit status was not delivered — fixed

With 1 and 2 worked around, Zed uploaded the entire server binary correctly
and *still* failed, because it gates its next step on `scp`'s exit code and
got a non-zero one with empty stderr (verified independently: exit 1, no
stderr, identical SHA-256 on a 3 MB file).

**This is now fixed** — see "The `scp` exit-code note" above for the
mechanism and the `scp -vvv` captures either side of it. After the fix Zed
logs `uploaded remote development server in 107.449ms`, and the binary
arrives decompressed (91,726,320 bytes from the 31 MB `.gz`) and
executable, which it never did before.

### 4. Where it actually stops now

Past the upload, Zed's remote server needs **five** separate `$HOME`
locations, each hard-coded with no redirect setting, discovered one at a
time because each failure only names the next path:

| Path | Why it is needed |
|---|---|
| `~/.zed_server` | the server binary itself |
| `~/Library/Application Support/Zed` | `server_state/<id>`, `extensions`, and more |
| `~/Library/Logs/Zed` | the server's log file |
| `~/Library/Caches/Zed` | cache dir |
| `~/.config/zed` | config dir |

Two of these are worth calling out beyond the inconvenience:

- **Granting them is a real widening.** `~/Library/Application Support/Zed`
  is the *local* Zed's entire data directory — settings, extensions,
  database. A sandbox granted it can write to the editor that is connecting
  to it. That is a materially different posture from VS Code/Cursor, which
  only needed a redirectable server install path.
- **Denying *read* makes existence checks lie.** Three of those paths exist
  already, and the failure was `create_dir` returning `EEXIST` — because
  with the path unreadable, Zed's `exists()` check returns false, so it
  takes the "create it" branch and the kernel rejects the create. The error
  text ("File exists") points at the opposite of the actual cause. Any
  sandbox that denies read will produce this class of confusing failure, so
  it is worth recognising on sight.

With all five granted, the server binary starts, initialises its
directories, and writes its `server.pid` — and then the daemon that
`proxy` forks exits without writing a single byte to the log file it just
created, so `proxy` times out with `failed to spawn server` and Zed reports
`Client exited with exit_code 1`. Nothing in devcroft's logs shows a denial
at that point. Diagnosing further needs Zed-side knowledge of what that
forked daemon does before its first log write; it was not pursued here.

So Zed is **not** blocked on the `scp` bug any more — that one is fixed and
was real. What remains is unattributed: it may be a further sandbox
interaction or may be Zed-specific, and this document should not guess
which.

## What would close this out

1. ~~Fix finding 3 (SFTP/`scp` exit-status delivery).~~ **Done** — and it
   was worth doing on its own merits, independent of Zed: every `scp` user
   was getting a false failure on a successful transfer.
2. ~~Find out what actually blocks VS Code past the install path.~~
   **Done** — a `$TMPDIR` over macOS's 104-byte unix socket limit, caused
   by this repo's own sample manifest. Fixed there.
3. **Decide whether `network` blocking should keep denying listening
   sockets.** Still the highest-impact item on the list, and not an editor
   concern: today no dev server can bind a port under the default policy,
   which contradicts the README's own port-conflict example and makes the
   default unusable for ordinary local development. Confirmed still true
   on 2026-08-15 — with the other two blockers removed, VS Code under the
   default policy fails on its very first step with `error listening on
   port: Operation not permitted`. Options are a separate knob, or
   treating loopback binds as distinct from egress. Whatever is chosen,
   `up` should say so rather than leaving `EPERM` from a `bind` call as
   the only signal.
4. **Warn about a `$TMPDIR` that is too long for a unix socket.** devcroft
   already respects this limit for its own state paths but tells users
   nothing, and the project-local `$TMPDIR` that trips it is a pattern
   devcroft's own sample was recommending. A `doctor` check is the natural
   home.
5. **Retest Zed — its finding 4 is now a live hypothesis, not a mystery.**
   Zed's server "exits without writing a single byte to its log" is the
   same shape of symptom VS Code showed, and Zed was run against the same
   sample with the same 64-character project-local `$TMPDIR`. Cheap to
   test now, and worth doing before anyone reads Zed's startup path.
6. Retest Cursor. Its 2026-08-14 row is probably wrong for the same
   listening-socket reason and no longer reflects a default-policy sandbox.
7. Decide what devcroft does about Zed finding 1 (missing-path grants
   rendered as live but silently dropped by the backend) — that one is a
   correctness bug in the policy surface independent of any editor, and it
   remains the finding that contradicts a stated invariant.
8. Consider whether the remote-server-directory-under-`$HOME` pattern
   deserves a first-class answer in devcroft's own docs rather than a
   per-editor workaround, since all three editors tested hit some version
   of it — VS Code and Cursor via a redirectable setting, Zed via a
   hard-coded path that needs a symlink.
