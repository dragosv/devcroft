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
- [x] 1.7 README known gaps / status: note the behavior change — two
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

- [x] 2.1 User configuration file in `src/config/`:
      `$XDG_CONFIG_HOME/devcroft/config.toml`, falling back to
      `~/.config/devcroft/config.toml`, on macOS too. Absent is not an
      error. Unknown keys rejected with the file and the full key path,
      like every section of `devcroft.toml`. It is devcroft's first user
      configuration; keep it to the layer and credentials
- [x] 2.2 `[tools]` in it: `provider` and `path`. `up --tools <path>`
      takes precedence over the file (the flag's provider is detected
      from the directory, or given; decide which while implementing and
      record it)
- [x] 2.3 `devcroft.toml` keeps rejecting `[tools]` and gains no
      credential key; assert both, since a project-declared layer is the
      decision this change reversed
- [ ] 2.4 Credential request in the user configuration, distinguishing
      env-var shape from single-file shape. Reject at parse time a
      file-shaped credential naming a directory, and any glob or
      wildcard — a credential is per named file (`credentials` delta
      spec)
- [x] 2.5 Unit tests for each rejection, asserting layer `config` and
      exit code 2
- [x] 2.6 Regression test: no user configuration and no `--tools`
      produces a byte-identical `policy --render` and captured env to
      before this change — the migration plan's stated invariant. The
      test must not read the developer's real `~/.config`; point
      `XDG_CONFIG_HOME` at a temp dir

## 3. Tooling layer resolution

- [x] 3.1 Resolve the layer host-side at `up`, in the same trusted phase
      as the project environment, through the provider's **hook-free**
      route (`print-dev-env --json`, `shellenv --pure`, `devenv build
      shell`, the derived flox copy); never from inside the boundary, and
      the layer's hook never runs, there or later
- [x] 3.2 Reject at layer `provider`, exit code 3: a layer with no lock;
      one naming an artifact-tier provider (`swift`), whatever the
      project's provider; one that would pass a host binary through. This
      is the `host`-passthrough guard, not a nicety (design.md Decisions
      1, 5)
- [x] 3.3 Keep only the layer's `PATH` entries, in its provider's order,
      and place them after the project's and before devcroft's resolved
      shell (`KeeperEnv::vars`). No other variable from the layer reaches
      a session. This is a new rule in `tooling`, not an extension of
      `env-provider`'s "Fixed composition order", which is about
      resolving before restriction
- [x] 3.4 The layer's store root as a read-only grant with origin
      `tools:<provider>`, rendered even where the project's provider grant
      already covers it, and folded into the grants in `up()` beside the
      resolved shell's, so `Meta` and the compiled profile cannot
      disagree
- [x] 3.5 `up` fails naming the path if resolving the tooling layer would
      need write access outside the project root
- [x] 3.6 Record the layer's path, provider and lock fingerprint in
      `Meta`; `policy --render` renders the grant from it. `status`
      reports stale when the lock changed or the configuration names a
      different layer or none; `up --recreate` applies it; a plain `up`
      does not re-resolve
- [x] 3.7 `up` prints one line naming the layer; `status` names it and
      lists the layer's binaries the project's `PATH` shadows. No line,
      and no warning, when there is no layer or nothing is shadowed
- [ ] 3.8 Fleet: resolve the layer once per fleet and give every agent
      the same entries; check they are untouched by the minimal root's
      `/workspace` rewrite
- [x] 3.9 Determinism test: repeated resolution of the same project and
      layer yields byte-identical env and rendered policy
- [x] 3.10 Test: with and without a layer, every session variable other
      than `PATH` is identical, and `policy --render` differs *only* by
      the read-only grant carrying the tooling origin
- [x] 3.11 Test: a name in both (`git`) resolves to the project's, and
      `status` names it shadowed
- [ ] 3.12 Test: a `swift` project with a devenv layer gets the layer's
      tools (macOS; on Linux, skip naming why)

> **Groups 2 and 3 landed (2026-11-02), except 2.4, 3.8 and 3.12.** What
> implementing them decided or found:
>
> - **2.2, decided: the provider is detected, and `provider` is optional
>   in the file too.** One rule for both: the file each provider keeps
>   (`.flox`, `devenv.nix`, `devbox.json`, `flake.nix`, `Package.swift`),
>   with more than one refused rather than guessed, naming `provider` as
>   the way to say which (`tooling::detect_provider`). A `provider` given in
>   the file is validated at layer `config`, like `env.provider`.
> - **2.4 is deferred to group 4, on purpose.** A credential key that
>   parses and delivers nothing is the silent no-op this project refuses,
>   and 4.0 must first measure what the file shape even needs. Until then
>   `[credentials]` in the user configuration is an unknown key, asserted.
> - **2.6 is asserted by the no-layer half of the 3.10 test** rather than
>   against an older binary: no `tools:` rule and no `tools` line at `up`.
>   Every binary run in `tests/tooling_layer.rs` points `XDG_CONFIG_HOME`
>   at the test's own directory, and `up` itself reads no user
>   configuration (the CLI selects the layer and passes it in
>   `UpOptions::tools`), so every pre-existing test is hermetic by
>   construction.
> - **3.4's origin is `tools:<provider>`, not `tools:<path>`**, parallel
>   to `provider:<name>` and a `&'static str` like it; the path is what
>   `up` and `status` print. The design's diagram is corrected.
> - **3.5 holds by construction, not by a check.** A provider's
>   resolution yields read-only grants and nothing else, and the layer's
>   go only through `with_tools_grants`, which writes `filesystem_read`;
>   there is no path by which a layer could ask for write access, so a
>   check would be one that cannot fire.
> - **Shadowing compares store paths, not names alone.** The same store
>   path on both sides (two environments from one nixpkgs) is the same
>   tool, so it is not reported; the 3.11 test uses `gitMinimal` against
>   `git` for that reason. Found by the test, which first used `git` on
>   both sides and saw nothing shadowed.
> - **flox locks an unlocked manifest instead of refusing it**, unlike
>   devbox, devenv and nix, so the layer checks `.flox/env/manifest.lock`
>   itself. And flox's derived hook-free copy lands under the *layer's*
>   directory (`<layer>/.devcroft/flox-env-*`), which is the user's own
>   directory and which `rm` does not clean.
> - **3.8 is open, and larger than it reads.** A fleet agent's store grant
>   is narrowed to its own closure (`fleet::closure::narrow`), so a layer
>   must add its closure's roots there too, and "once per fleet" needs
>   the fleet's supervisor to record the layer. That touches
>   `add-linux-agent-fleet`'s state as well as this change.
> - **3.12 needs macOS**; `swift` fails closed on Linux.
> - Measured on Linux with `tests/tooling_layer.rs` (4 tests, devenv on
>   `PATH`) and by hand with Claude Code in a devenv layer: `claude -p`
>   reached `api.anthropic.com` and was refused for a dummy key, and the
>   layer's `enterShell` never ran.

## 4. Credentials

- [ ] 4.0 **Measure before building the file shape**, on a host with a
      subscription login: (a) does an agent survive a token refresh with
      its credentials file granted read-only, or does it fail at first
      expiry; (b) does `claude setup-token` / `CLAUDE_CODE_OAUTH_TOKEN`
      authenticate a subscription through `[env] forward` with no file
      grant. Record both in design.md Decision 2. If (b) holds, 4.3–4.4
      and 7.1 are re-scoped before any code (design.md Decision 2)
- [x] 4.1 Env-var shape, requested in the user configuration: delivered
      as an environment variable, adding no filesystem grant. `[env]
      forward` in the project is unchanged
- [x] 4.2 Assert the secret value never reaches the compiled policy,
      `meta.json`, or the logs — a test, not a code comment
> **The env-var shape landed (2026-11-02): 4.1, 4.2, 4.6, and the env
> half of 2.4.** `[credentials] env = ["NAME", …]` in the user
> configuration; names checked at layer `config`, duplicates collapsed.
> `credentials.file` stays an unknown key until 4.0 is measured, so 2.4
> stays open for that half. Decided while implementing, from the specs
> rather than freshly: a requested name this shell does not hold fails
> `up` at `config` before any state exists (the `lifecycle` requirement
> says fail, and it is the user's own explicit request, unlike `forward`,
> which warns); a name the manifest also sets in `[env.vars]` is refused,
> as within one manifest. `Meta` records the names, never the values, and
> `up` discloses each one on every run, from `Meta`, so a repeat `up`
> against a running sandbox names what it has. `why --env` reads the user
> configuration too, or it would call a delivered credential `ABSENT`.
> 4.2 is asserted by walking every file under the state dir and the
> project's `.devcroft/` for the value, with the walk checked to reach
> `meta.json`. The disclosure covers the user's requests; the project's
> own `[env] forward` is unchanged and still reviewable in the repository.

- [ ] 4.3 File shape: grant exactly one file, read-only, using the
      backend's per-file grant. Granting the containing directory is
      explicitly not an acceptable implementation
- [ ] 4.4 Test: sibling files in the credential's directory stay
      unreadable, and the credential file itself is not writable
- [ ] 4.5 Baseline-denied paths stay denied even when named by a
      credential request
- [x] 4.6 `up` prints exactly one disclosure line per exposed credential,
      including on a repeat `up` against an already-running sandbox
- [ ] 4.7 File credentials appear in `policy --render` with a credential
      origin, distinguishable from an ordinary manifest grant

## 4b. Brokered credentials (design.md Decision 6)

The measurement this group rests on is done. All eleven agents can be
pointed at an `http://` endpoint and send the key header in clear to the
proxy: nine as an absolute-URI request, and two (Qwen, Cursor) inside a
`CONNECT :80` tunnel (`docs/agents.md`, design.md Decision 6).

- [x] 4b.0 **Owner's decision (2026-11-15): adopt `nono-proxy`**, after the
      investigation in design.md Decision 6, and upgrade nono to match. Three
      steps come before 4b.1:
- [x] 4b.0a nono 0.77 → 0.82: `Cargo.toml` pin, full suite, the
      `THIRD-PARTY-LICENSES.md` regeneration, and the documents that cite
      0.77. Read the library diff for what changed in enforcement, not only
      in types
      **Done (2026-11-15).** devcroft compiled unchanged. The suite passed
      769/0, as on 0.77. The boundary tests ran with `--nocapture` and no
      skips, and every `fleet_*` file ran in a delegated cgroup (72/0, no
      skips). Shipped dependencies went from 339 to 285, since nono 0.82
      dropped its HTTP client. What changed in enforcement:
      - Linux: the enforced Landlock ABI is still capped at V6, and V7–V9
        are detected but not enforced. Its seccomp filters now refuse
        io_uring, on paths devcroft does not take on a V4+ kernel.
      - macOS: `process-exec*` is now scoped to the read grants, which is
        what `docs/nono-process-exec-issue.md` asked for, so that draft is
        marked superseded. devcroft's own rules are kept until a macOS run.
        The keychain mach-lookup denials are now unconditional. Not run on
        macOS.
- [x] 4b.0b Swap `proxy::server`'s HTTP handling for `nono-proxy`, behind
      the existing spawn and bridges:
      - `strict_connect_auth = true`;
      - the token passed as `session_token`, so it stays devcroft's;
      - the bridge sends `Basic nono:<token>`, and `client_env`'s userinfo
        becomes `nono:<token>@`;
      - TLS interception off, and `strict_filter` on.

      `nono-proxy` binds its own listener and `add_listener` accepts only
      its own port, so `spawn` no longer pre-binds the TCP port: the child
      reports the port it got over a pipe before `up` compiles the policy.
      The unix socket is still bound before exec. The allow/refuse log
      lines come from its audit sink. Fleet's label prefix is kept
      **Done (2026-11-16).** `proxy::server::start` configures and runs
      `nono-proxy`; `__egress_proxy` reports the port over a pipe once both
      listeners serve (`proxy::spawn_at`). An auth failure is logged as
      `requested=<h>:<p>`, never `host=`, so `lifecycle::attribution`
      cannot read it as an allowlist refusal. A plain-HTTP allow is the
      L7 audit event and carries no port. Suite 751/1, zero skips; the one
      failure is `tooling_layer`'s devenv build, on a host store path
      registered valid but missing from disk (`amp-linux-arm64.gz`),
      before any proxy runs. Teeth-check: with `strict_connect_auth =
      false`, `an_unauthenticated_caller_is_refused_even_for_an_allowed_host`
      fails. Shipped dependencies 285 → 380, `THIRD-PARTY-LICENSES.md`
      regenerated. The test build now needs `-j 2` on this devcontainer:
      at full parallelism the linker is OOM-killed
- [x] 4b.0c Refusal status `502` → `403`, wherever a test or document
      asserts it. The host is still named
      (`403 Forbidden: host <h>:<p> is not in the allowlist`, measured)
      **Done (2026-11-16).** Tests (`egress_proxy_e2e`,
      `loopback_any_port`, `agent_hook`) assert `403`. Where a document
      records a `502` that was *measured* (the macOS table in
      `docs/known-gaps.md`, `policy::degraded`, `backend_capabilities`),
      the measurement stays and is footnoted rather than rewritten
**4b.1–4b.9 follow `nono-proxy`'s route form, not the own-proxy form**
(reconciled 2026-11-16, design.md Decision 6's investigation). A route is a
path prefix on the proxy's own port, so the sandbox's endpoint is
`http://127.0.0.1:<port>/<prefix>`, not `http://<real host>`, and
`nono-proxy` refuses an absolute-URI or `CONNECT` request to a route's host
by design. The placeholder is the session token, optionally wrapped by
`phantom_format`, not a free string. Since the port is chosen at `up`, the
user names the variable that carries the endpoint as well.

- [x] 4b.1 User configuration, `[[credentials.broker]]` in the user's
      `config.toml`: `host` (the upstream, reached as `https://<host>`),
      `header` (the one carrying the key), `env` (the variable the sandbox
      gets the placeholder in, and the one the real value is read from in
      the user's shell at `up`), `endpoint_env` (the variable the sandbox
      gets `http://127.0.0.1:<port>/<prefix>` in), and an optional
      `placeholder`, a `phantom_format` template such as `ghp_{}`. Shape is
      checked at parse; at `up`, still at layer `config`: the host must be
      on the project's allowlist, since brokering grants no egress of its
      own, and the value must be set in the shell
      **Done (2026-11-16).** `config::user::parse_brokers` and
      `lifecycle::up::check_brokers`. A `host` is a bare DNS name of two
      labels or more; `env` and `endpoint_env` may not be a proxy variable,
      `HOME`, `PATH` or `DEVCROFT_*`, nor shared with another entry or with
      `credentials.env`. A placeholder is checked by `nono-proxy`'s own
      `PhantomTemplate::parse_route`
- [x] 4b.2 The sandbox gets the placeholder in `env` and the endpoint in
      `endpoint_env`, never the value. Extend 4.2's walk (state dir,
      `.devcroft/`, logs, `meta.json`) to the brokered value, and add the
      keeper's and a session's `/proc/<pid>/environ`
      **Done (2026-11-16)** in `tests/broker_up.rs`, inside an isolated
      sandbox: the session holds the token and the endpoint and not the
      value, and neither the state dir, the project, the keeper's environ
      nor the proxy's holds it. Teeth-check: with `spawn_at` no longer
      removing the variable, the proxy-environ assertion fails
- [x] 4b.3 Proxy: one `nono-proxy` route per brokered credential, upstream
      `https://<host>`, injecting into the declared header with
      `nono-proxy`'s default format (`Bearer {}` for `Authorization`, the
      bare value otherwise). The upstream is verified against the trust
      store the sandbox's environment names (`tls_ca`), when there is one.
      Responses, SSE included, are `nono-proxy`'s to copy back
      **Done (2026-11-16).** `proxy::server::broker_route`; `tls_ca` is the
      environment's `SSL_CERT_FILE` after `trust_store::rewrite`, which is
      why `up` now starts the proxy after the trust store is resolved.
      `a_brokered_request_reaches_the_upstream_with_the_real_value` runs it
      against a TLS upstream with a test CA; with the credential removed
      the route does not load and the test fails
- [x] 4b.4 The real value lives only in the proxy's process. `up` writes it
      to the proxy's stdin, never its argv or environment block; the proxy
      sets it in its own environment before starting any thread, for
      `nono-proxy`'s `env://` source, so `/proc/<pid>/environ` never holds
      it. Not written to the proxy's state, never logged. The placeholder is
      rendered by `nono-proxy`'s own `PhantomTemplate`, and the endpoint in
      its own `credential_env_vars` shape, so a proxy reused across a stale
      recovery yields the same values; it is reused only when `Meta`
      records the same brokers
      **Done (2026-11-16).** The proxy also has the variable removed from
      the environment it inherits from `up`, since `up` read it there, and
      exits if `nono-proxy` loaded fewer credentials than it was given
- [x] 4b.5 Refuse rather than downgrade:
      - a brokered request naming a file credential;
      - a host with no allowlist entry;
      - `network.default = "allow"`, where traffic need not pass the proxy;
      - a variable the manifest's `[env.vars]` or the user's
        `[credentials] env` also sets.

      Each refusal is at layer `config`, naming the key
      **Done (2026-11-16).** A file credential cannot be named at all: an
      entry has no such key, and `credentials.file` is still unknown.
      `a_broker_is_refused_rather_than_downgraded` covers the rest, and
      that a refusal leaves no state
- [x] 4b.6 `up` discloses each brokered credential as brokered (host and
      variable, never the value), distinct from the env-delivered
      disclosure, and says that the host is now reachable only through its
      endpoint (`nono-proxy` refuses a plain `CONNECT` to a route's host).
      `policy --render` shows the host with origin `credential:broker`
      **Done (2026-11-17).** The disclosure is from `Meta`, on every `up`.
      `CompiledPolicy::credentials_broker`, folded in by `with_brokers`
      alongside the proxy port, both in `up` (so the policy artifact
      carries it) and in `--render`'s reconstruction from `Meta`; rendered
      as `credentials.broker:` only when there is one, each entry naming
      host, header, `env` and `endpoint_env`. Nothing in the backend
      profile corresponds: the host's egress is still `network.allow`'s.
      The provisioning policy carries none. Asserted in
      `render_shows_brokered_hosts_with_their_origin_only_when_any` and,
      against a live sandbox, in `tests/broker_up.rs`. Teeth-check: without
      the fold-in in `up`, the e2e assertion fails
- [x] 4b.7 Tests against a local TLS upstream (a test CA as the trust
      store):
      - the upstream receives the real value;
      - the sandbox's environment and every file under the state dir hold
        only the placeholder;
      - a request with a wrong or missing placeholder never reaches the
        upstream;
      - a request to a different allowed host never gets the value.

      Teeth-check: with the route removed, the upstream never sees the
      value
      **Done (2026-11-19).** In `proxy::server`'s tests, against
      `tls_upstream` (an `rcgen` CA, given to the route as `tls_ca`):
      `a_brokered_request_reaches_the_upstream_with_the_real_value`,
      `a_wrong_or_missing_placeholder_never_reaches_the_upstream` (a wrong
      key, and the bare token where a `sk-ant-{}` shape is declared), and
      `a_request_to_another_allowed_host_never_gets_the_value`: the
      placeholder sent to a second allowed host, absolute-URI and inside a
      `CONNECT` tunnel, arrives unchanged, and a `CONNECT` to the route's
      own host is refused with nothing reaching it. The environment and
      state-dir walk is `tests/broker_up.rs` (4b.2), in a real sandbox.
      Teeth-checks: without the route the first test fails (4b.3); with the
      real value sent to the other host, its assertion fails
- [x] 4b.8 End to end with Claude Code inside a real sandbox, with a dummy
      key, against the local TLS upstream above, isolated and not. A real
      key is the owner's, and is not needed to show the substitution.
      Cursor is out: it sends a loopback endpoint as a `CONNECT`, which
      `nono-proxy` refuses (measured)
      **Done (2026-11-20).** `tests/broker_claude.rs`: Claude Code 2.1.293
      from the store, `claude -p` against a TLS upstream that answers enough
      of the Messages API (streamed and not), under a shaped placeholder
      (`sk-ant-api03-{}`). Asserted: the reply comes back, every request the
      upstream sees carries the real key and never the token, and neither
      the session's environment nor the proxy log holds the value. Both
      paths run in one test, sequentially since every variable `up` reads is
      process-wide: isolated (relay and unix bridge), then not, through
      `DEVCROFT_TEST_NO_NETNS`, which makes `fleet::netns::probe` answer as a
      host without namespaces. Like `DEVCROFT_TEST_BROKER_UPSTREAM` it exists
      in `test-support` builds only, and it can only remove isolation. Which
      path ran is checked from inside the session, not assumed: only the
      loopback path's `HTTPS_PROXY` carries the token as userinfo.
      Teeth-checks: with the lever misnamed, the non-isolated run's mode
      assertion fails; with a different value in `ANTHROPIC_API_KEY`, the
      real-key assertion fails
- [x] 4b.9 `docs/agents.md`: per agent, the brokered configuration (its
      `endpoint_env`; Codex's endpoint is a config flag, so it reads the
      variable through `-c openai_base_url=$…`). Cursor is stated as not
      brokerable. `docs/threat-model.md`: brokering moves the key out of
      custody, and the clear leg is sandbox → proxy (the unix bridge on
      Linux, loopback on macOS)
      **Done (2026-11-20).** `docs/agents.md`, "Brokering the key": the
      configuration, the refusals, and one row per agent saying how far it
      was tested through a route (Claude Code end to end in devcroft; Codex,
      Antigravity, OpenCode and Qwen Code through `nono-proxy` alone; the
      rest only in the request form). Copilot's upstream is left unsettled
      rather than guessed, since it uses two hosts. `docs/threat-model.md`,
      "Brokered credentials: capability": custody removed, capability kept
      for the sandbox's lifetime, the clear leg, and what is not covered.
      The README no longer says there is no broker

## 5. CLI and doctor

- [x] 5.1 `doctor`: report tooling-layer resolvability, distinguishing
      "none configured" from "configured but unresolvable" and naming
      the fix
- [x] 5.2 Confirm no new top-level command — `--name` and `--tools` are
      flags, tooling and credentials are user configuration; list
      `--tools` in `USAGE` (`tests/cli_help_and_version.rs`)

> **5.1 resolves the layer, not just its provider's binary** (CLAUDE.md:
> guard on the capability): `[INFO]` for none configured, `[PASS]` with its
> `PATH` entry count, `[FAIL]` naming the layer and the fix otherwise.
> Asserted for the two cases that need no devenv
> (`doctor_tells_no_layer_from_a_broken_one`). **1.7 needed nothing new**:
> the README's "Git worktrees" section already states the refusal and
> `--name`, and 0.0.1 is the first release, so no published version had the
> old behaviour. The README's agent section now shows the layer, and its
> command block lists `--tools`.

## 6. End-to-end: an agent actually runs

- [x] 6.1 Integration test, self-skipping like the other real-tooling
      tests: an agent CLI in a user tooling layer (via `XDG_CONFIG_HOME`
      or `--tools`), a project whose committed files mention no agent;
      `up`, then `exec -- <agent> --version` runs it **inside** the
      sandbox. The 2026-10-29 spike (design.md Decision 4) is the manual
      version of this test, with all eleven agents
      **Done (2026-11-03)**: `tests/tooling_layer.rs`,
      `every_agent_in_a_user_layer_runs_inside_a_project_that_names_none` —
      all eleven, in one devenv layer declared in the user configuration
      (not `--tools`), each resolved from its own package and printing its
      version inside the sandbox. The project's `devenv.yaml` is written,
      not copied, since the samples' names `claude-code` in a comment; and
      `allowUnfree` is the layer's alone. Checked to fail on a wrong
      binary name before being trusted.
- [ ] 6.2 Auth test, both shapes: an env-var key, and a single-file
      credential. Skip honestly rather than faking a token if no
      credential is available in the test environment
- [x] 6.3 Re-run the probe from proposal.md's Why and record the new
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
      **Re-run with this change's code (2026-11-06), the after half.** The
      project was a devenv one whose committed files name no agent:
      `ripgrep` and `jq`, with `network.default = "deny"` and only
      `api.anthropic.com` allowed. The agent came from a user tooling layer
      in `XDG_CONFIG_HOME`'s `config.toml` (`[tools] path`, the
      `devenv-claude-sample` environment). The key was a dummy, delivered
      by `[credentials] env = ["ANTHROPIC_API_KEY"]`, not by the project's
      `forward`. `up` printed one line for the layer and one for the
      credential, with no workaround anywhere. Inside: `claude` resolved
      to the layer's store path (2.1.293). `sh` and `SHELL` were closure
      paths. `SSL_CERT_FILE` named a readable bundle. `/proc` was the
      sandbox's own (pid 1 is devcroft, 5 pids visible). The key was
      present. `claude -p` reached the API, and the API refused the key
      (`Invalid API key`). Before → after, per stop: the CA bundle,
      `sh`, `/proc` and the credential route all work with nothing granted
      by hand, and the agent comes from a declared, locked environment
      instead of `npm install` inside the sandbox. **`node` is not on
      `PATH`, and does not need to be:** the nixpkgs package carries its
      own runtime, so the probe's "`node` present" now reads "the agent
      present". **Still unmeasured:** the file-shaped credential (4.0,
      6.2), and a key the API accepts.

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
- [x] 6b.2 Decide how the agent reaches `why`, and record the trade:
      **(a)** a hook script that reads the policy artifact directly — no new
      policy surface, answers "what was I granted"; **(b)** grant read+exec
      on `current_exe()`, folded into the grants in `up()` the way the
      resolved shell's store root already is, which buys real per-path `why`
      but lets an agent run *every* devcroft subcommand inside the sandbox.
      (b) needs its own audit of that surface — `rm`, `up --recreate` — and
      is not a free consequence of (a).
      **Decided (2026-11-03): (a) ships; (b)'s reach is the status quo,
      now narrowed and audited.** Measured in a real devenv sandbox:
      - The grant was the binary's *directory*, so every neighbour ran
        inside (all of `~/.cargo/bin` after `cargo install`). Narrowed to
        the file; `tests/sandbox_attention.rs` asserts a sibling is refused.
      - The surface, run inside: every subcommand acts on the sandbox's
        own `HOME`, never the host's state dir (baseline-denied and absent
        from the view). `ps` says `no sandboxes`; `down` and `rm --yes`
        "succeed" against an empty state and the host's keeper stays
        healthy; `up --recreate` fails at layer `provider`, after clearing
        the sandbox's own `report.json`, which the sandbox can write
        anyway. Misleading, not harmful. `why --path`/`--host` gives the
        host's verdict per path (6b.1).
      - Why (a) still ships: a hook is committed (6b.4), and the binary's
        path is per machine (`target/debug`, `~/.cargo/bin`), so a
        committed hook cannot name it. The policy artifact is found from
        `HOME` (`.devcroft/<name>/policy.json`), the same on every machine.
        Per-path `why` stays available to an agent told where devcroft is,
        and is not what the hook depends on.
      **Amended (2026-11-05), while implementing 6b.3:** the hook does run
      devcroft after all, through a link `up` writes beside the sandbox's
      home (`.devcroft/<name>/devcroft`, `services::EXE_LINK`), so the
      committed script names `$HOME/../devcroft` — the same on every machine,
      which was the objection. Forced by a measured fact: a path the mount
      view hides fails `ENOENT`, exactly like a missing file, so only the
      policy can tell the two apart (6b.6), and a POSIX script cannot
      evaluate it. The artifact is still what is read; the link adds no
      reach, since the binary was already granted.
- [x] 6b.3 Implement (a) as the shipped path: a denial-triggered hook that
      fires on the agent's tool-failure event, gates on a denial signature so
      ordinary failures pass through untouched, and injects the sandbox's
      grants plus the instruction to diagnose **before** asking the user for
      permission.
      **Done (2026-11-05)**: `src/agent_hook.rs`, run as `devcroft agent
      hook claude-code` on Claude Code's `PostToolUseFailure` (its
      documented input; output `hookSpecificOutput.additionalContext`). The
      error text only nominates — a refusal- or missing-shaped message, the
      paths and hosts it names — and the policy decides
      (`policy::why_resolved`, `why_host`). The context is phrased as fact,
      since Claude Code's documentation warns that imperative, system-styled
      injected text reads as prompt injection. To judge a `~` rule inside,
      the artifact now records the host's home (`services::PolicyArtifact`).
      **Found on the way, and fixed: `why --path` inverted for every
      absolute path in the project** — compared against written rules
      (`.`), `$PWD/src` came back `DENIED / not granted by any rule`, on the
      host and inside. `why_resolved` resolves the rules first, as the
      backend does; asserted inside a sandbox by `tests/denial_hook.rs`.
- [x] 6b.4 Wiring delivery: `devcroft init --agent` writes it once and the
      user commits it. Rejected alternatives, recorded rather than
      re-litigated: silently generating it at `up` (devcroft's requirement
      appearing unasked in the user's tree — the same criticism
      `decouple-service-supervisor` levelled at the process-compose
      coupling), and requiring the user to hand-install it.
      **Done (2026-11-05)**: `init --agent claude-code` writes
      `.claude/hooks/devcroft-denial.sh` and merges one entry into
      `.claude/settings.json`, key order kept; once only; an unparseable
      file is refused untouched. On a project with a manifest it writes the
      hook alone. Registered in exec form, `sh` + the script, because
      `/bin/sh` is not in the sandbox's view and exec form resolves on
      `PATH`, where devcroft's shell is. The script is silent outside a
      sandbox, so committing it costs a host-side user nothing.
- [x] 6b.5 State the limitation in the change rather than discovering it
      later: this is **agent-specific by necessity**. devcroft cannot rewrite
      the `EPERM` a tool sees — it comes from the kernel through whatever
      binary the agent ran — so an interception point is required, and the
      agent's own hook system is the only one available. Per-agent wiring,
      not a general mechanism.
      **Done (2026-11-05)**: stated in `src/agent_hook.rs`'s module doc, and
      `init --agent <other>` refuses as not yet supported. The gate itself is
      agent-neutral; only the input and output shapes are Claude Code's.
- [ ] 6b.7 Adopt ArcBox's credential-forwarding shape, which is better than
      this change's current plan and costs nothing to copy: forward only
      `ANTHROPIC_*`/`CLAUDE_*`, for that session only, never written into any
      image or sandbox record, every other host variable left behind
      (`docs/prior-art.md`). Independent evidence for 7.1's premise arrives
      with it: a project with a real microVM boundary states that OAuth
      credentials are *deliberately not copied in*, so subscription auth is
      unsolved there too, not merely unsolved here.
- [x] 6b.6 Test that the gate is real: an ordinary failure (a genuinely
      missing file) must **not** trigger the sandbox explanation. A hook that
      fires on everything teaches the agent to ignore it.
      **Done (2026-11-05)**: `tests/denial_hook.rs`, inside a real devenv
      sandbox, on errors the sandbox itself produced: `/etc/hostname`
      (hidden, `ENOENT`) and a write to the store (read-only) are explained;
      a missing file at the root, under `src/`, in `$HOME` and in `/tmp`, a
      mode-000 file, `false` and a nonzero exit are not. Two mutations
      checked: firing regardless of the policy, and firing on every
      `ENOENT`, each fail it. **Not measured: Claude Code itself running the
      hook** — that needs a login this host lacks; the test feeds the
      documented input, which is the gap to close on a logged-in host.

## 7. Docs

- [x] 7.1 `docs/decisions.md`: amend the "secret injection ... never via
      mounted files or plain env vars" position, naming the property that
      made it unachievable (subscription auth has no env-var form), per
      that file's own convention that a rejection whose premise stops
      holding is revisited rather than defended
      **Done (2026-11-20).** §3, "Secrets → `[env] forward`, or brokered at
      the proxy". The refutation of both halves was already there (written
      with `own-sandbox-environment`); what was stale was brokering as
      unbuilt. It now records that brokering narrows the rule's domain
      rather than restoring it, and names the two properties that keep a
      second route: a subscription login has no env-var form and rewrites
      its file on refresh, and some clients exchange the key or cannot
      reach a route (Cursor). It also states the condition for revisiting
      it again. `openspec/config.yaml`'s context line says brokering is
      built
- [x] 7.2 `docs/decisions.md`: record why a host-binary tooling shortcut
      is rejected, naming the criteria it fails, so it is not proposed
      again as "just for agents"
      **Done (2026-11-03)**: §1, "Rejected: the host's binary, 'just for
      agents'" — criteria 2 and 5, with the keeper-directory grant as the
      measured case of the shortcut shipping by accident.
- [x] 7.3 Document the residual credential exposure plainly: in-sandbox
      code can read an exposed credential; the mitigations are narrowness
      and disclosure, not isolation from the code under edit
      **Done (2026-11-03)**: `docs/threat-model.md`, "What ships today is
      custody" — both routes (`[env] forward`, `[credentials] env`), the
      user's reaching every project, and what egress does and does not
      bound. `docs/decisions.md` §3 already said the same in one paragraph.
- [x] 7.4 README: the fleet-of-agents claim can now be stated as
      supported — but only to the extent tasks 6.1/6.2 actually passed,
      and tier-qualified as always
      **Done (2026-11-21)**, to the extent that passed: Status gains "Coding
      agents inside a sandbox, Linux only so far". It covers 6.1 (eleven
      agents from a user layer), API keys forwarded (the matrix) and
      brokered (4b.8, Claude Code end to end), and says what is not
      claimed: no real key, no macOS run, and no subscription login, since
      6.2's file half and 4.0/4.3–4.7 are open. "Not claims today" names
      the subscription login in the same terms. When 6.2 passes, that
      sentence is the one to revise

## 8. Verification

- [x] 8.1 `cargo build`, `cargo clippy`, `cargo fmt` clean
      **Done (2026-11-22)**: build, `fmt --check`, `clippy --all-targets`
      with and without `test-support`, and `cargo doc --no-deps` with
      zero rustdoc warnings
- [x] 8.2 `openspec validate --all` passes with this change included
      **Done (2026-11-22)**: 20 passed, 0 failed
- [ ] 8.3 **macOS**: verify the tooling layer, both credential shapes,
      and the project-root check on a real macOS host, or report plainly
      that they are unverified there. Desktop agent fan-out is mac-heavy;
      a Linux-only result materially limits this change and must not be
      papered over
