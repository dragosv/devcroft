# Tasks: add-mvp-core

Ordered so the load-bearing risk (fd passing across sandbox application) is
retired first, and every phase ends in something runnable.

## 1. Spike: prove the keeper trick (de-risk before building anything)
- [x] 1.1 Minimal Rust binary: create unix listener, exec under `nono run`
      with fd inheritance, accept a connection post-restriction, fork a
      child, verify the child is inside the boundary (Linux/Landlock)
- [x] 1.2 Same spike on macOS/Seatbelt
- [x] 1.3 Decision record: fd passing vs socket-activation fallback; pin the
      tested nono version range

## 2. Config & policy
- [x] 2.1 Manifest schema (serde), discovery walk, validation with typo
      suggestions and name slug rules
- [x] 2.2 Policy compiler: manifest + baseline denials -> nono profile JSON,
      deterministic output, origin annotations
- [x] 2.3 `policy --render`, `why` (delegating to `nono why`)
- [x] 2.4 Degraded-capability detection per host (network domain filtering)

## 3. Provider layer
- [x] 3.1 Provider trait; provider validation (reject `host`/`none`/version
      managers with the reproducibility message)
- [x] 3.2 `flox` provider: activation env-diff capture, store-path grant
      injection, staleness detection (hash of manifest.toml + lockfile)

## 4. Keeper & lifecycle
- [x] 4.1 Keeper main loop: spawn protocol over control socket (spawn,
      signal, resize, reap), session registry
- [x] 4.2 Supervisor: `up` (idempotent, recovery, --recreate), `down`, `rm`,
      pid/state management, grace-period termination
- [x] 4.3 `status`, `logs`, `ps`

## 5. Sessions
- [x] 5.1 `exec` with exit-code and cwd mapping
- [x] 5.2 `shell` with pty, resize, signal forwarding, orphan reaping
- [x] 5.3 Auto-up on cold sandbox

## 6. SSH endpoint
- [x] 6.1 russh server in keeper on unix socket; publickey auth against the
      devcroft client key; ephemeral host keys
- [x] 6.2 `proxy` subcommand; `ssh-config` emit + idempotent `--write`
- [x] 6.3 Channels: exec, pty/shell, window-change, env allowlist, exit
      status; SFTP subset for scp/rsync
- [x] 6.4 direct-tcpip (`-L`) gated by policy
- [ ] 6.5 Validation matrix: OpenSSH client, rsync, VS Code Remote-SSH,
      Zed, Cursor — document what works per editor. Nearly done: see
      docs/ssh-validation.md — OpenSSH client (ssh/scp/sftp/-L) and rsync
      are validated by real end-to-end tests (`tests/ssh_channels.rs`);
      VS Code Remote-SSH and Cursor are validated by real manual runs
      against a live sandbox (real connection, publickey auth, remote
      server install/start, workbench window open for VS Code / both
      servers reachable for Cursor) — the fix both needed was redirecting
      `remote.SSH.serverInstallPath` inside the project root, since
      devcroft's default policy correctly denies writing to `$HOME`. Only
      Zed remains: it has no CLI to drive non-interactively and uses a
      different remote-dev mechanism than the other two, so needs a real
      display and manual GUI setup to close out. Getting rsync working
      surfaced and fixed a real bug along the way, not covered by any task
      number:
      `lifecycle::up`'s keeper spawn looked up `nono` by bare name after
      the child's environment had already been replaced with the
      provider-resolved (flox-activated) one, so `up` failed outright
      with ENOENT on any host where `nono` doesn't live under that fixed
      PATH (e.g. Homebrew on Apple Silicon, `/opt/homebrew/bin`) — fixed
      by resolving `nono` against the ambient PATH first, same as
      `provider::flox` already does for `flox` itself
      (`paths::resolve_on_path`, now shared by both).

## 7. CLI polish & release
- [x] 7.1 `init` with flox detection; `doctor` with actionable checks
- [x] 7.2 Error contract: layers, exit codes, non-interactive safety.
      Wired up the rest of the command surface this required to be
      meaningful: `up`, `down`, `rm`, `status`, `logs`, `ps`, `ssh`,
      `policy`, `why`. Two gaps found along the way, not covered by any
      task number, both now closed (see `lifecycle::hooks` and
      `provider::Resolution::unset`): the lifecycle spec's "Hooks run
      inside the boundary" requirement (`hooks.post_create`/
      `hooks.post_start` execution — `Manifest.hooks` parsed but nothing
      ran it) and the env-provider spec's activation-diff gap (a variable
      activation *removed* leaked through from whoever's shell ran `up`,
      since a plain `BTreeMap<String, String>` diff has no way to
      represent "unset").
- [x] 7.3 Two-sandbox concurrency test; suspend/resume test
- [x] 7.4 README with honest limitations section (no inter-sandbox process
      hiding, cooperative network filtering, not a hard security boundary)
- [ ] 7.5 Publish crate `devcroft`; reserve/point npm name. Deliberately
      held back: two of the three gaps 0.1.0's maturity was judged against
      (hooks unimplemented, the env-diff unset gap) are now closed; the
      SSH validation matrix is down to just Zed (see
      docs/ssh-validation.md). Cargo.toml has the publish-required
      metadata (description, license, repository) ready; `cargo publish
      --dry-run` packages clean. The actual publish and npm name
      reservation are the maintainer's own accounts/call, not something
      to do preemptively.
