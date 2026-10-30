## 1. Sandbox identity per project root

Independent of everything below and the smallest real bug here — land it
first so the fan-out fix is not gated on the tooling design.

- [x] 1.1 `up`: compare the current project root against `meta.json`'s
      recorded `project_root` (already stored — no new bookkeeping) and
      refuse to adopt a state dir bound to a different root
- [x] 1.2 Error names both roots, the sandbox name, and `--name` as the
      fix, with a stable exit code per the error contract
- [x] 1.3 Confirm same-root `up` stays idempotent — the check must
      distinguish a different root from a repeated run, not conflate them
- [x] 1.4 `--name` override plumbed through the commands that resolve a
      sandbox; when given, the discovered manifest's declared name need
      not match
- [x] 1.5 An overridden name is used consistently for state dir,
      `status`/`ps`/`logs`, and SSH host naming
- [x] 1.6 Integration test with a **real** `git worktree`: two worktrees
      of one repo, same committed manifest. Without `--name` the second
      `up` fails naming both roots; with distinct `--name` values both
      sandboxes run independently
- [ ] 1.7 README known gaps / status: note the behavior change — two
      worktrees that silently shared a sandbox now fail loudly

> **Landed. Two things the implementation found, both by running it:**
>
> - **Placement was the fix, not the comparison.** The first version put the
>   check after the health decision, where it never fired for the case it
>   exists to catch: a *healthy* sandbox returns `AlreadyUp` early, and
>   adopting a healthy sandbox from the wrong root is precisely the silent
>   failure. Caught by the worktree test, not by review. "Does this state dir
>   belong to me" has to be answered before "should I adopt it".
> - **A test was relying on the bug.** `symlink_escape_cli` hardcoded a
>   sandbox name with no pid while its project root varied by pid, so a
>   leftover state dir from an earlier run belonged to a different root — and
>   was silently adopted. It now fails loudly, which is the change working.
>
> `--name` is an *override*, distinct from the positional `[name]` selector:
> it renames this project's sandbox for one invocation, so the discovered
> manifest deliberately need not agree. Applied to the parsed manifest rather
> than threaded through callers, which makes 1.5 hold by construction — the
> state dir, `status`/`ps`/`logs` and SSH naming all read
> `manifest.sandbox.name`.

## 2. Config surface

> **Revised 2026-10-30: the layer and credential requests are the
> user's, not the project's** (design.md Decisions 1, 2). Nothing in
> this group or the next two had been started, so they are rewritten
> rather than struck.

- [ ] 2.1 User configuration file in `src/config/`:
      `$XDG_CONFIG_HOME/devcroft/config.toml`, falling back to
      `~/.config/devcroft/config.toml`, on macOS too. Absent is not an
      error. Unknown keys rejected with the file and the full key path,
      like every section of `devcroft.toml`. It is devcroft's first user
      configuration; keep it to the layer and credentials
- [ ] 2.2 `[tools]` in it: `provider` and `path`. `up --tools <path>`
      takes precedence over the file (the flag's provider is detected
      from the directory, or given; decide which while implementing and
      record it)
- [ ] 2.3 `devcroft.toml` keeps rejecting `[tools]` and gains no
      credential key; assert both, since a project-declared layer is the
      decision this change reversed
- [ ] 2.4 Credential request in the user configuration, distinguishing
      env-var shape from single-file shape. Reject at parse time a
      file-shaped credential naming a directory, and any glob or
      wildcard — a credential is per named file (`credentials` delta
      spec)
- [ ] 2.5 Unit tests for each rejection, asserting layer `config` and
      exit code 2
- [ ] 2.6 Regression test: no user configuration and no `--tools`
      produces a byte-identical `policy --render` and captured env to
      before this change — the migration plan's stated invariant. The
      test must not read the developer's real `~/.config`; point
      `XDG_CONFIG_HOME` at a temp dir

## 3. Tooling layer resolution

- [ ] 3.1 Resolve the layer host-side at `up`, in the same trusted phase
      as the project environment, through the provider's **hook-free**
      route (`print-dev-env --json`, `shellenv --pure`, `devenv build
      shell`, the derived flox copy); never from inside the boundary, and
      the layer's hook never runs, there or later
- [ ] 3.2 Reject at layer `provider`, exit code 3: a layer with no lock;
      one naming an artifact-tier provider (`swift`), whatever the
      project's provider; one that would pass a host binary through. This
      is the `host`-passthrough guard, not a nicety (design.md Decisions
      1, 5)
- [ ] 3.3 Keep only the layer's `PATH` entries, in its provider's order,
      and place them after the project's and before devcroft's resolved
      shell (`KeeperEnv::vars`). No other variable from the layer reaches
      a session. This is a new rule in `tooling`, not an extension of
      `env-provider`'s "Fixed composition order", which is about
      resolving before restriction
- [ ] 3.4 The layer's store root as a read-only grant with origin
      `tools:<path>`, rendered even where the project's provider grant
      already covers it, and folded into the grants in `up()` beside the
      resolved shell's, so `Meta` and the compiled profile cannot
      disagree
- [ ] 3.5 `up` fails naming the path if resolving the tooling layer would
      need write access outside the project root
- [ ] 3.6 Record the layer's path, provider and lock fingerprint in
      `Meta`; `policy --render` renders the grant from it. `status`
      reports stale when the lock changed or the configuration names a
      different layer or none; `up --recreate` applies it; a plain `up`
      does not re-resolve
- [ ] 3.7 `up` prints one line naming the layer; `status` names it and
      lists the layer's binaries the project's `PATH` shadows. No line,
      and no warning, when there is no layer or nothing is shadowed
- [ ] 3.8 Fleet: resolve the layer once per fleet and give every agent
      the same entries; check they are untouched by the minimal root's
      `/workspace` rewrite
- [ ] 3.9 Determinism test: repeated resolution of the same project and
      layer yields byte-identical env and rendered policy
- [ ] 3.10 Test: with and without a layer, every session variable other
      than `PATH` is identical, and `policy --render` differs *only* by
      the read-only grant carrying the tooling origin
- [ ] 3.11 Test: a name in both (`git`) resolves to the project's, and
      `status` names it shadowed
- [ ] 3.12 Test: a `swift` project with a devenv layer gets the layer's
      tools (macOS; on Linux, skip naming why)

## 4. Credentials

- [ ] 4.0 **Measure before building the file shape**, on a host with a
      subscription login: (a) does an agent survive a token refresh with
      its credentials file granted read-only, or does it fail at first
      expiry; (b) does `claude setup-token` / `CLAUDE_CODE_OAUTH_TOKEN`
      authenticate a subscription through `[env] forward` with no file
      grant. Record both in design.md Decision 2. If (b) holds, 4.3–4.4
      and 7.1 are re-scoped before any code (design.md Decision 2)
- [ ] 4.1 Env-var shape, requested in the user configuration: delivered
      as an environment variable, adding no filesystem grant. `[env]
      forward` in the project is unchanged
- [ ] 4.2 Assert the secret value never reaches the compiled policy,
      `meta.json`, or the logs — a test, not a code comment
- [ ] 4.3 File shape: grant exactly one file, read-only, using the
      backend's per-file grant. Granting the containing directory is
      explicitly not an acceptable implementation
- [ ] 4.4 Test: sibling files in the credential's directory stay
      unreadable, and the credential file itself is not writable
- [ ] 4.5 Baseline-denied paths stay denied even when named by a
      credential request
- [ ] 4.6 `up` prints exactly one disclosure line per exposed credential,
      including on a repeat `up` against an already-running sandbox
- [ ] 4.7 File credentials appear in `policy --render` with a credential
      origin, distinguishable from an ordinary manifest grant

## 5. CLI and doctor

- [ ] 5.1 `doctor`: report tooling-layer resolvability, distinguishing
      "none configured" from "configured but unresolvable" and naming
      the fix
- [ ] 5.2 Confirm no new top-level command — `--name` and `--tools` are
      flags, tooling and credentials are user configuration; list
      `--tools` in `USAGE` (`tests/cli_help_and_version.rs`)

## 6. End-to-end: an agent actually runs

- [ ] 6.1 Integration test, self-skipping like the other real-tooling
      tests: an agent CLI in a user tooling layer (via `XDG_CONFIG_HOME`
      or `--tools`), a project whose committed files mention no agent;
      `up`, then `exec -- <agent> --version` runs it **inside** the
      sandbox. The 2026-10-29 spike (design.md Decision 4) is the manual
      version of this test, with all eleven agents
- [ ] 6.2 Auth test, both shapes: an env-var key, and a single-file
      credential. Skip honestly rather than faking a token if no
      credential is available in the test environment
- [ ] 6.3 Re-run the probe from proposal.md's Why and record the new
      result in the change — `node` present, credential readable — so the
      before/after is evidence, not assertion
      **Interim probe (2026-10-18), before any of this change's code**: a
      flox project with `nodejs`, `network.default = "deny"`, `up` on Linux
      (Landlock ABI V6, mount view, network namespace). Claude Code 2.1.302,
      installed the vendor's way (Decision 1's option (e), `npm install -g
      --prefix $HOME/.local` inside the sandbox, `registry.npmjs.org`
      allowed). In the order each one stopped it:
      1. **CA bundle outside the view.** flox sets `SSL_CERT_FILE=
         /usr/lib/ssl/cert.pem`, a host symlink to `/etc/ssl/certs/…`; `/etc`
         is not in the view (`ENOENT`), so nixpkgs' node, which trusts
         OpenSSL's store, fails TLS (`UNABLE_TO_GET_ISSUER_CERT_LOCALLY`).
         `NODE_OPTIONS=--use-bundled-ca` works around it.
      2. **No `sh` on `PATH`, and `SHELL=/bin/sh`**, a host path absent from
         the view. npm's lifecycle scripts `spawn sh` (`ENOENT`); an agent's
         shell tool does the same. devcroft's resolved shell exists but is
         on neither.
      3. **`/proc` is refused, and the agent is a Bun binary that aborts
         without it** (`SIGABRT` in 3 ms; strace: `/proc/self/maps`,
         `/proc/self/cgroup`, `/proc/self/stat`, `/proc/stat`,
         `/proc/sys/vm/overcommit_memory` all `EACCES`). Granting
         `/proc/self` does nothing (the rule binds the granting process's
         own pid directory); granting `/proc` makes it run.
      With all three worked around (and `[filesystem] read = ["/proc"]` as
      an experiment only — see below), `claude -p` with a dummy key
      forwarded by `[env] forward = ["ANTHROPIC_API_KEY"]` reached
      `api.anthropic.com` through the egress proxy and was refused by the API
      (`Invalid API key`); the proxy refused its telemetry host. So the
      env-var credential shape already works through `forward`.
      **Why `/proc` is not simply granted in `up`:** there is no PID
      namespace there and the host's procfs is bind-mounted, so a `/proc`
      grant reads every same-user process's `/proc/<pid>/environ`. The
      default-deny on `/proc` is the protection `docs/known-gaps.md` ("No
      inter-sandbox process visibility separation") relies on. Fleet agents
      get a fresh procfs (`ProcMount::Fresh`), where the grant would expose
      only the agent's own processes, but fleet's plan does not grant it
      either, so the same agent aborts there too.
      Also absent, not yet blocking: coreutils and `git` (not in the
      closure; the host's are denied), `/etc/passwd`, `/etc/localtime`,
      `/etc/hosts`. Not measurable on this host: the file-shaped
      credential (no `~/.claude/.credentials.json` here).
      **Fixed since (2026-10-18), for `up` and fleet alike:** 1, by granting
      the bundle the environment names (`crate::trust_store`, origin
      `trust-store`, the variable rewritten to its canonical path; only a PEM
      certificate bundle outside the home, never from `[env.vars]`); 2, by
      making the resolved shell `SHELL` and the last `PATH` entry
      (`KeeperEnv::vars`). Re-run: `npm install` of the agent succeeds with no
      workaround. 3 is fixed for fleet and, since `add-up-pid-namespace`, for `up`
      wherever the host can mount a fresh procfs (`with_own_procfs`, origin
      `pid-namespace`): Claude Code 2.1.302 runs in an `up` sandbox and
      `claude -p` reaches the API. `tests/agent_runtime_env.rs` covers 1 and 2 through `up`;
      each fails with its fix removed.

## 6b. Denial feedback: the agent learns why it was refused

> The technique taken from nono's registry packs (`docs/prior-art.md`).
> An agent that hits a policy denial today gets a bare `EPERM`: it does not
> know it is sandboxed, what it was granted, or that `why` answers exactly
> that. The likely outcome is a wrong inference — "the file does not exist",
> "this needs sudo" — and either a give-up or an absurd request.
>
> **Depends on `fix-silent-policy-degradation`.** Measured: `why` run from
> inside a sandbox does not fail, it inverts — `~/.local/share/devcroft` is
> baseline-denied and not overridable, so `compile_with_provider_grants`
> silently drops the provider grants and reports `DENIED / not granted by any
> rule` for a store path the sandbox is actually granted. Wiring an agent to
> a confidently inverted answer is worse than leaving it with the `EPERM`.

- [x] 6b.1 Confirm `fix-silent-policy-degradation` has landed and that `why`
      from inside a session returns the same verdict and origin as on the
      host. Everything below is unsafe to ship before that.
      **Confirmed (2026-10-10)** by `tests/policy_view_fidelity_live.rs`: a
      real nix sandbox, `why` inside a session gives `ALLOWED provider:nix`,
      and the two `policy --render` outputs are byte-identical.
      **For 6b.2, a fact its option (b) assumes otherwise:** devcroft's own
      binary is *already* reachable inside every sandbox. `up` grants the
      keeper binary's directory (that is how the test runs `devcroft why`
      inside, and the grant the host's rendering used to hide), so (b) adds
      no new reach; what it would add is only the instruction to use it.
- [ ] 6b.2 Decide how the agent reaches `why`, and record the trade:
      **(a)** a hook script that reads the policy artifact directly — no new
      policy surface, answers "what was I granted"; **(b)** grant read+exec
      on `current_exe()`, folded into the grants in `up()` the way the
      resolved shell's store root already is, which buys real per-path `why`
      but lets an agent run *every* devcroft subcommand inside the sandbox.
      (b) needs its own audit of that surface — `rm`, `up --recreate` — and
      is not a free consequence of (a).
- [ ] 6b.3 Implement (a) as the shipped path: a denial-triggered hook that
      fires on the agent's tool-failure event, gates on a denial signature so
      ordinary failures pass through untouched, and injects the sandbox's
      grants plus the instruction to diagnose **before** asking the user for
      permission.
- [ ] 6b.4 Wiring delivery: `devcroft init --agent` writes it once and the
      user commits it. Rejected alternatives, recorded rather than
      re-litigated: silently generating it at `up` (devcroft's requirement
      appearing unasked in the user's tree — the same criticism
      `decouple-service-supervisor` levelled at the process-compose
      coupling), and requiring the user to hand-install it.
- [ ] 6b.5 State the limitation in the change rather than discovering it
      later: this is **agent-specific by necessity**. devcroft cannot rewrite
      the `EPERM` a tool sees — it comes from the kernel through whatever
      binary the agent ran — so an interception point is required, and the
      agent's own hook system is the only one available. Per-agent wiring,
      not a general mechanism.
- [ ] 6b.7 Adopt ArcBox's credential-forwarding shape, which is better than
      this change's current plan and costs nothing to copy: forward only
      `ANTHROPIC_*`/`CLAUDE_*`, for that session only, never written into any
      image or sandbox record, every other host variable left behind
      (`docs/prior-art.md`). Independent evidence for 7.1's premise arrives
      with it: a project with a real microVM boundary states that OAuth
      credentials are *deliberately not copied in*, so subscription auth is
      unsolved there too, not merely unsolved here.
- [ ] 6b.6 Test that the gate is real: an ordinary failure (a genuinely
      missing file) must **not** trigger the sandbox explanation. A hook that
      fires on everything teaches the agent to ignore it.

## 7. Docs

- [ ] 7.1 `docs/decisions.md`: amend the "secret injection ... never via
      mounted files or plain env vars" position, naming the property that
      made it unachievable (subscription auth has no env-var form), per
      that file's own convention that a rejection whose premise stops
      holding is revisited rather than defended
- [ ] 7.2 `docs/decisions.md`: record why a host-binary tooling shortcut
      is rejected, naming the criteria it fails, so it is not proposed
      again as "just for agents"
- [ ] 7.3 Document the residual credential exposure plainly: in-sandbox
      code can read an exposed credential; the mitigations are narrowness
      and disclosure, not isolation from the code under edit
- [ ] 7.4 README: the fleet-of-agents claim can now be stated as
      supported — but only to the extent tasks 6.1/6.2 actually passed,
      and tier-qualified as always

## 8. Verification

- [ ] 8.1 `cargo build`, `cargo clippy`, `cargo fmt` clean
- [ ] 8.2 `openspec validate --all` passes with this change included
- [ ] 8.3 **macOS**: verify the tooling layer, both credential shapes,
      and the project-root check on a real macOS host, or report plainly
      that they are unverified there. Desktop agent fan-out is mac-heavy;
      a Linux-only result materially limits this change and must not be
      papered over
