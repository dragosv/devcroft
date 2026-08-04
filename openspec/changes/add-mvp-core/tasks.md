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
- [ ] 3.1 Provider trait; provider validation (reject `host`/`none`/version
      managers with the reproducibility message)
- [ ] 3.2 `flox` provider: activation env-diff capture, store-path grant
      injection, staleness detection (hash of manifest.toml + lockfile)

## 4. Keeper & lifecycle
- [ ] 4.1 Keeper main loop: spawn protocol over control socket (spawn,
      signal, resize, reap), session registry
- [ ] 4.2 Supervisor: `up` (idempotent, recovery, --recreate), `down`, `rm`,
      pid/state management, grace-period termination
- [ ] 4.3 `status`, `logs`, `ps`

## 5. Sessions
- [ ] 5.1 `exec` with exit-code and cwd mapping
- [ ] 5.2 `shell` with pty, resize, signal forwarding, orphan reaping
- [ ] 5.3 Auto-up on cold sandbox

## 6. SSH endpoint
- [ ] 6.1 russh server in keeper on unix socket; publickey auth against the
      devcroft client key; ephemeral host keys
- [ ] 6.2 `proxy` subcommand; `ssh-config` emit + idempotent `--write`
- [ ] 6.3 Channels: exec, pty/shell, window-change, env allowlist, exit
      status; SFTP subset for scp/rsync
- [ ] 6.4 direct-tcpip (`-L`) gated by policy
- [ ] 6.5 Validation matrix: OpenSSH client, rsync, VS Code Remote-SSH,
      Zed, Cursor — document what works per editor

## 7. CLI polish & release
- [ ] 7.1 `init` with flox detection; `doctor` with actionable checks
- [ ] 7.2 Error contract: layers, exit codes, non-interactive safety
- [ ] 7.3 Two-sandbox concurrency test; suspend/resume test
- [ ] 7.4 README with honest limitations section (no inter-sandbox process
      hiding, cooperative network filtering, not a hard security boundary)
- [ ] 7.5 Publish crate `devcroft`; reserve/point npm name
