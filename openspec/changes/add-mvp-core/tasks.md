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
- [x] 6.5 Validation matrix: OpenSSH client, rsync, VS Code Remote-SSH,
      Zed, Cursor — document what works per editor. **Closed for 0.0.1 on
      2026-08-31, on the deliverable as written**: the task asks for a
      matrix that documents what works per editor, and
      `docs/ssh-validation.md` is that matrix, negative rows included. It
      was left open for Zed, which is the wrong shape for a release gate —
      the failure is Zed's forked daemon exiting without writing its own
      log, is not attributed to devcroft, and has no CLI to drive it
      non-interactively, so holding on it holds indefinitely. It ships as
      a documented negative instead, which is what a `0.0.z` number is
      for (`docs/roadmap.md`). One thing below has since changed and is
      corrected in the matrix: the "no way to express no-egress-but-can-
      listen" gap is closed by `network.ports` for *named* ports
      (`tests/network_ports_listen.rs`), which backs the README's port
      example but does **not** fix VS Code, whose supervisor picks its
      port at runtime. See
      docs/ssh-validation.md — OpenSSH client (ssh/scp/sftp/-L) and rsync
      are validated by real end-to-end tests (`tests/ssh_channels.rs`);
      VS Code Remote-SSH and Cursor were manually validated on 2026-08-14
      after redirecting `remote.SSH.serverInstallPath` into the project
      root (devcroft's default policy correctly denies writing to `$HOME`)
      — but **VS Code was retested on 2026-08-15, on the same host and the
      same build, and does not work**: devcroft's default `network.block`
      denies `bind`+`listen` including on loopback, so VS Code's server
      dies with `error listening on port: Operation not permitted`. That
      generalizes well past editors — under the default policy no dev
      server can bind a port at all, which contradicts the README's own
      port-conflict example, and there is no way to ask for "no egress but
      I can still listen". Cursor's row is untested since and suspect for
      the same reason. Recorded in docs/ssh-validation.md as the largest
      open item on this task.
      Zed was attempted for real on 2026-08-15 (its CLI *is* installable —
      `zed ssh://<name>.devcroft/<path>`) and now **connects, authenticates
      and transfers, but its server does not come up**: the forked daemon
      exits without writing to its own log, which is not attributed to
      devcroft. Getting that far fixed a real devcroft bug worth more than
      the Zed row itself — `scp` reported failure on every successful
      transfer, because the exit-status was sent from a spawned task racing
      the channel drop's `close`, and a client never accepts an
      exit-status after `close`. `ssh::server::NotifyOnEof` now withholds
      EOF until the exit-status resolves, so ordering is structural rather
      than a race; `scp -vvv` goes from `Exit status -1` to `Exit status 0`
      and the test asserts it in both directions. Zed also needs five
      separate `$HOME` grants, one being the local editor's own data dir.
      Two further findings came out of it (both in docs/ssh-validation.md): the
      backend silently drops `filesystem.allow` grants whose path does not
      exist yet while `policy --render` still shows them as live (an
      inspectability-invariant break, still open), and SFTP resolves
      relative paths against the project root while the exec channel's bare
      `cd` goes to `$HOME`, unlike OpenSSH which uses home for both
      (documented, deliberately not changed). A third, fixed here: `why`
      passed `~`-rooted paths and grants to `nono why` as literal CLI
      flags, which nono reads as relative, so `why` died with
      `parsing 'nono why' output: expected value at line 1 column 1` on any
      manifest carrying a `~` grant — and would have answered DENIED for a
      granted path even had it parsed (`policy::why::expand_home`).
      Getting rsync working
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
- [ ] 7.5 Publish crate `devcroft`; reserve/point npm name. **Everything
      but the publish itself is done as of 2026-08-31; the remaining step
      needs the maintainer's crates.io and npm accounts, which is why this
      stays unchecked.** `cargo package` verifies clean, `cargo clippy` and
      `cargo doc --no-deps` are both warning-free, `openspec validate --all`
      is 18/18, and the name is free on both registries (checked
      2026-08-31: crates.io returns "crate `devcroft` does not exist",
      npm returns 404). Publish is then `cargo publish`.

      **The version is `0.0.1`, not `0.1.0`, and that is a decision rather
      than a placeholder.** `0.0.z` is the only range cargo treats as
      incompatible with itself, which is the only numbering consistent with
      what `src/lib.rs` already states — the modules are internals,
      published so `tests/` can drive them, with no stability offered. A
      `0.1.0` promises patch-compatibility across an uncurated surface, and
      it would also put the more confident number on the less finished
      thing while `tests/unix_socket_not_mediated.rs` still asserts an open
      hole in the boundary. `docs/roadmap.md` records when `0.1.0` gets
      cut: when `add-mount-isolation` lands.

      **A second, found by running the packaged binary rather than
      inspecting the package.** `devcroft --help` did not exist —
      `unknown command "--help"` — and neither did `--version`, on a CLI
      whose own crate docs tell readers to depend on "the `devcroft`
      binary and its documented command surface (`devcroft --help`, and
      the README)". The fallback message also sent a user of a published
      binary to "the cli spec", which ships in the repository and not in
      the crate. The first thing anyone does with a freshly installed CLI
      is ask it for help, and it answered with an error naming a file
      they do not have. Added, with `tests/cli_help_and_version.rs`
      pinning the contract: every command in the closed surface appears
      in the text, hidden `__` re-exec modes do not, explicit help is
      stdout/0 while misuse is stderr/2, and no message points at
      anything the crate does not ship.

      **One release blocker found by this pass, now fixed.** `src/bin/`
      binaries are auto-discovered, and the `include` allowlist shipped
      `spike.rs` — so `cargo install devcroft` would have put a second,
      generically-named `spike` binary on the user's PATH. The allowlist
      now carries `!/src/bin/spike.rs`; excluding the source removes the
      target, and `cargo package --list` is 49 files with no `spike` row.
      Worth noting the class: the earlier audit checked the package for
      *file count and size* and got both right, which is a different
      question from what the package *installs*.

      **Release blockers found by auditing what `cargo publish` would
      actually ship, and now fixed** — "packages clean" had been asserted
      from a dry-run's exit code, which says nothing about the contents:
      - The package swept in **265 files / 2.0 MB**, including 131
        `openspec/` files, `.claude/`, `.devcontainer/` and `samples/` —
        all of it published permanently. An anchored `include` allowlist
        cuts it to 50 files / 781 KB. The first attempt used unanchored
        globs and made it *worse* (`README.md`/`LICENSE` matched at any
        depth, pulling in flox store symlinks and a vendored Go module
        cache); leading `/` is what fixes it.
      - `chacha20 0.10.1` in `Cargo.lock` was **yanked**. Updated to
        0.10.2.
      - No `rust-version`, so a pre-1.85 toolchain would fail with syntax
        errors rather than a clear MSRV message. Declared.
      - **15 rustdoc warnings**, several broken intra-doc links in
        *public* docs — what docs.rs would have rendered. Now zero.
      - `src/lib.rs` published the entire internal architecture as public
        API with no stability statement, making every refactor a semver
        break. The modules stay public (the integration suite drives them
        out-of-process), but the crate doc now says plainly that this is
        not a stable API and points users at the binary.
      - License: MIT, while **189 of 335 shipped dependencies are
        Apache-2.0** — including two direct ones, `nono` and `russh`.
        Legal as-is (Apache-2.0 is permissive), but the binary shipped
        **no third-party attribution at all**, which Apache-2.0 §4(a)
        requires. Now **Apache-2.0** (`LICENSE-APACHE` + `NOTICE`), with
        `THIRD-PARTY-LICENSES.md` generated by a committed script.
        Dual `MIT OR Apache-2.0` was the first choice — the Rust
        convention — and was reconsidered before release: the MIT half
        buys a binary user nothing, since nono, russh and every
        `sigstore-*` are Apache-2.0-only and linked regardless, so it
        would only let someone opt out of the patent grant while their
        actual obligations stayed identical. Apache-2.0 alone matches
        the stack devcroft sits on and makes the patent grant
        mandatory. Decided pre-release deliberately: dropping the MIT
        option after publishing would be a breaking change for anyone
        who relied on it. Worth
        recording what that audit turned up: the crates that most needed
        attribution — `nono`, `russh`, every `sigstore-*` — are exactly
        the ones that vendor no license file, so a naive generator lists
        precisely the wrong 15 as "not vendored". The generator
        substitutes the canonical Apache-2.0 text for those, which is
        exact rather than approximate because that text carries no
        per-holder copyright line.
