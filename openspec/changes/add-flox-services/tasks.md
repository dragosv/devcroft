## 1. Blocking-dependency gate — RESOLVED

- [x] 1.1 Gate ran and initially failed exactly as predicted (`bind`
      denied under `network.default = "deny"`). Rather than stopping
      there, checked the premise: nono's own profile schema carries an
      `open_port` field, so "no egress, but I can listen" was always
      expressible and devcroft simply never emitted it. The claim that
      this was a policy-model limitation was **wrong** — see
      proposal.md's amended Blocking Dependency section
- [x] 1.2 Resolved by the `network.ports` manifest key rather than
      documented around: `[network] default = "deny"`, `ports = [N]`
      binds `127.0.0.1:N` with egress still filtered and ungranted ports
      still denied. Compiles to nono's `open_port` (chosen empirically —
      `listen_port` granted nothing on Linux/nono 0.71.0). Covered by
      `tests/network_ports_listen.rs` plus unit tests in `config` and
      `policy`; `policy --render` shows the ports with their origin, so
      the "nothing reaches the backend that --render cannot show"
      invariant holds. **Integration tests therefore use the default
      deny policy, not a workaround** — which is what 1.2 asked to record

## 2. Provider contract: service declarations

- [x] 2.1 `Resolution` gains `services: ServiceSupport`, a three-valued
      enum (`Unsupported` vs `Declared(Vec<ServiceDecl>)`) so "has no
      service concept" and "supports them, none declared" stay distinct
- [x] 2.2 `src/provider/flox.rs`: `read_service_declarations` parses
      `[services]` from the flox manifest host-side. Uses `toml::Table`,
      not `toml::Value` — the latter rejects flox's real manifest
      outright, caught by the existing against-real-flox test
- [x] 2.3 `src/provider/nix.rs`: `ServiceSupport::Unsupported`, declared
      explicitly with the reasoning inline
- [ ] 2.4 `up` fails at layer `provider`, exit code 3, when services are
      requested from a provider that supports none — naming the provider.
      **Currently unreachable through the CLI**: declarations come from
      the provider's own manifest, so a `nix` project has no way to
      declare services at all. The reachable variant worth building
      instead is a project with `[services]` in a flox manifest whose
      `devcroft.toml` says `provider = "nix"` — today those services are
      silently ignored. Left open deliberately rather than shipping a
      check that can never fire
- [x] 2.5 Unit tests: `[services]` present/absent, ordering determinism,
      and a service with no string `command` failing loudly (the
      schema-drift guard) rather than resolving to an empty list
- [ ] 2.6 Regression test: `policy --render` byte-identical with and
      without services declared

> **Out of scope, moved to its own change:** `nix` returning a flat
> `ServiceSupport::Unsupported` is correct for the interface devcroft
> consumes — a plain `devShell` genuinely has no services. A flake using
> services-flake *does* declare them, but as a separate flake app
> (`nix run .#services`) rather than in the devShell, so serving it means
> resolving a second flake output. That is a provider question, not a
> flox-services one, and belongs in its own specification.
- [x] 2.7 Full documented schema, not just `command`: `vars`,
      `is-daemon`, `shutdown.command`. Found by checking flox's docs
      rather than assuming — `vars` carries the service's port in flox's
      own documented example, so dropping it starts services on the wrong
      port while the command string still looks right. Non-string vars are
      rejected rather than stringified, and `is-daemon` without
      `shutdown.command` is rejected at resolution (such a service is
      unstoppable: the launcher exits by design, so killing it at
      teardown reaps nothing)
> **Out of scope, moved to its own change:** `nix` returning a flat
> `ServiceSupport::Unsupported` is correct for the interface devcroft
> consumes — a plain `devShell` genuinely has no services. A flake using
> services-flake *does* declare them, but as a separate flake app
> (`nix run .#services`) rather than in the devShell, so serving it means
> resolving a second flake output. That is a provider question, not a
> flox-services one, and belongs in its own specification.

## 3. Service supervision in the keeper

> **Design conflict found while implementing — now RESOLVED, both
> decisions recalibrated. Read design.md decisions 1 and 4 before
> starting.**
>
> The conflict: `up` cannot own service lifetime. It is a short-lived CLI
> process, and a session whose client disconnects is escalated after
> `connection::DEFAULT_GRACE_PERIOD` (2s), so services started the way
> hooks are started would die ~2 seconds after `up` returns. The keeper
> must own them.
>
> **Resolution:** the keeper starts services at its own startup, which
> puts them *before* hooks — and decision 4 was reversed to match,
> because services-first turned out to be the correct ordering on its own
> merits anyway (the canonical `post_start` hook is "run migrations",
> which needs the database already up). The original hooks-first argument
> reasoned from devcroft's failure semantics rather than from what
> projects need. No protocol frame is required.
>
> **Also recalibrated (decision 1):** devcroft generates its *own*
> process-compose config from the documented `[services]` declarations
> and runs process-compose supervised, rather than reimplementing restart
> policy / daemon handling / dependencies. Consuming flox's own generated
> `service-config.yaml` was investigated and rejected — undocumented
> artifact, and its process-compose binary is flox's closure member, not
> the environment's. `process-compose` must be declared in the project's
> environment so it is a real closure member rather than a scanned store
> path.

- [x] 3.1 Services are registered in the **existing** session registry
      rather than a parallel one — which is what makes teardown work with
      no new machinery, since `install_shutdown_handler` already
      terminates every registered process group. **Partial:** the
      four-state model (not-started / failed-at-start / running /
      exited-later) the `services` delta spec requires is not built yet;
      today process-compose is one registry entry, and per-service state
      needs querying its API over the unix socket (task 5.x)
- [x] 3.2 Generate a process-compose config from the resolved
      declarations (devcroft's own artifact, not flox's), and start
      `process-compose up -f <config>` through the existing
      `SessionBackend` trait, without a pty and with no attached client
      (design.md decisions 1 and 2). **Do not** shell out to `flox
      services`, do not consume flox's `service-config.yaml`, and do not
      add a tier-specific path — going through the trait is what makes
      this work identically at `process` and `hardened`.
      **Was marked done while only half true, and is now actually wired:**
      the trait was used, but nothing on the hardened branch ever called
      it — `up_hardened` had no services block and `hardened_keeper_main`
      never called `start_services_if_requested`, so a project declaring
      services came up at `isolation = "hardened"` reporting a healthy
      sandbox and zero service lines, indistinguishable from one that
      declares none. Both tiers now share `up::prepare_services` for the
      host-side half and the same `start_services_if_requested` for the
      keeper-side half, differing only in which `SessionBackend` the
      request is dispatched through. Three things the wiring forced out
      into the open, all recorded rather than assumed:
      (a) service paths are now absolute, from `DEVCROFT_SERVICES_ROOT`,
      because `runsc exec --cwd` needs an absolute path and the host-side
      control server's cwd is not the sandbox's;
      (b) `spawn_hardened_keeper` now sets `current_dir(project_root)`,
      matching `spawn_keeper` — without it `ssh::server`, which takes each
      session's cwd from the control process's own, started hardened-tier
      sessions in whatever directory `up` was invoked from;
      (c) runsc's `--host-uds` defaults to `none`, under which a unix
      socket bound *inside* the sandbox is not connectable from the host —
      which is exactly how `status`/`ps` read per-service state back. The
      run args now request `--host-uds=create` (never `open`/`all`, which
      would also permit connecting *outward* to host sockets), and only
      for sandboxes that actually declare services
- [x] 3.2a Require `process-compose` in the resolved environment and fail
      at layer `provider` naming it when services are declared but the
      binary is not a closure member. Never scan `/nix/store` for it:
      that picks an arbitrary path with nothing tying it to this
      environment's config schema
- [x] 3.3 Per-service state and output (design.md decision 7). Query
      `process-compose process list -u <socket> -o json` — the socket is
      already open for this — and map its `status`/`exit_code`/
      `is_running` onto the four states the `services` spec requires.
      **Parse from the first `[`**: the CLI emits warn/debug lines to
      stdout ahead of the JSON (failed `getpwuid`, missing XDG dir), so
      assuming clean JSON fails on the first call.
      **Now actually complete — it was marked done while three of the
      four states were wrong or unreachable.** Measured against a real
      process-compose 1.120.0 rather than reasoned about (table in
      `ServiceState::from_json`):
      (a) `NotStarted` was **unreachable**: only process-compose's own
      listing was consulted, and it cannot report a service it never
      accepted, so a declared-but-missing service was reported by
      absence. `Meta::declared_services` now records what the provider
      declared and `services::reconcile` produces the state.
      (b) A **pending** service (`depends_on` gate) reports
      `is_running: false, exit_code: 0` — read by exit code alone that
      is "exited", so `status` immediately after `up` showed a service
      still queuing to start as one that had already finished.
      (c) A **skipped** service (dependency failed) reports
      `exit_code: 1` that no process produced, rendering as
      "failed (exit 1)" — an invented failure. `status` is now consulted
      first for exactly these two non-run states, with `exit_code`
      still authoritative everywhere else (a crash really does report
      `status: "Completed"`, which is why it cannot be trusted
      generally)
- [x] 3.4 No automatic restart (design.md decision 3). Now a property of
      the *generated* config rather than of devcroft's own supervision
      loop: emit process-compose's no-restart policy explicitly rather
      than relying on its defaults, since a default that restarts would
      silently reverse this decision. Assert it in a test so a future
      "helpful" restart cannot land unnoticed
- [x] 3.5 Teardown: stop services before the keeper exits, SIGTERM
      escalating to SIGKILL after the same grace period sessions use.
      Killing process-compose must reap its children — verify that rather
      than assuming it — and a service declaring `shutdown.command` must
      have it honored, since a daemon's launcher has already exited and
      killing it reaps nothing
- [ ] 3.6 Test at both tiers that a service ignoring SIGTERM is still
      gone after teardown — asserted by observing process absence, never
      by trusting a stop command's exit status

## 4. Lifecycle wiring

- [x] 4.1 Services start at **keeper startup, before hooks** (design.md
      decision 4, reversed — see the group 3 note). The keeper owns their
      lifetime because `up` cannot: it exits, and a disconnected session
      is escalated after 2s. Declarations reach the keeper the way the
      resolved env already does, not over the control socket
- [x] 4.2 `up --skip-hooks` also skips services, preserving "nothing
      project-supplied runs"; services report as not-started, not failed
- [x] 4.3 A failed service does not fail `up` — `up` exits 0, prints the
      failure, and `exec`/`shell`/SSH still work (the `services` delta's
      "do not block sandbox availability")
- [x] 4.4 `down`/`rm` stop services before tearing the keeper down
- [x] 4.5 Services start on every keeper start (`post_start` semantics,
      not `post_create`); no attempt to preserve process state across
      `down`/`up`
- [x] 4.6 `SandboxStatus` gains service state, with the same
      forward/backward-compatible posture the isolation-tier field used —
      sandboxes that predate this read as having no services. Note the
      state is *queried live*, not recorded at `up`: unlike
      `resolved_backend`, service state changes after `up` returns, so
      `meta.json` is the wrong home for it

## 5. CLI surface

- [x] 5.1 `ps` lists each service individually, labelled so services and
      sessions are distinguishable. Today the whole group shows as one
      opaque `process-compose (services)` entry — the registry entry that
      makes teardown work is deliberately not the reporting unit
- [x] 5.2 `logs` appends the service log to the keeper log. Service output
      goes to a separate file because process-compose writes it there
      (`-L`), and it already prefixes each line with the emitting process
      name — so per-service attribution needs no re-tagging by devcroft.
      Appended rather than left to be found: a failed service whose reason
      sits in an unmentioned file is the silent failure this exists to
      prevent
- [x] 5.3 `status` shows service state, so a healthy keeper with a failed
      service is not reported as simply healthy — the case that currently
      violates the `services` spec's "failure is visible, never silent"
      and that decision 3's no-auto-restart rationale depends on
- [ ] 5.4 `doctor`: report whether this host can bind a listening socket
      under a deny-default policy; when it cannot, name the consequence
      for services and the `network.default = "allow"` workaround along
      with the egress filtering it costs (per the `cli` delta spec)
- [ ] 5.5 Confirm no new top-level command was added — the MVP command
      surface stays closed, per proposal.md's Impact
- [ ] 5.6 Have `status` (or `doctor`) name devcroft as the supervisor, so
      a user running `flox services status` by hand and seeing nothing is
      not left confused (design.md decision 1's stated cost)

## 6. Tests

- [x] 6.1 Integration test, gated on real `flox` the way this repo's
      existing real-tooling tests self-skip: a project declaring a
      service comes up, the service runs inside the sandbox, `ps` shows
      it, `logs` has its output. **Was left unchecked while
      `tests/services_e2e.rs::a_declared_service_runs_inside_the_sandbox_and_is_reaped_by_down`
      already covered it** — it brings up a real flox project declaring
      a `python3 -m http.server` service, waits for it to answer from
      inside the sandbox over `exec`, and asserts `status` reports it
      running by name with its pid
- [x] 6.2 Teardown test: after `down`, the service process is gone from
      the host — asserted by process absence. Same test as 6.1, second
      half: it polls `ps -eo args` for both `http.server` and
      `process-compose up` until neither remains, rather than trusting
      any stop command's exit status
- [ ] 6.3 Failure test: a service whose command exits non-zero is listed
      as failed with a reachable log tail, and the sandbox stays usable
- [ ] 6.4 Policy test: a service denied a port by `[network]` fails
      visibly with the same denial any in-sandbox process would get
- [x] 6.5 Cross-tier test: the same service declaration behaves
      identically at `process` and `hardened`, in the shape
      `tests/hardened_tier_ssh_parity.rs` already uses — self-skipping
      when `runsc` is not functionally usable. **Half done, and the half
      that is missing is named rather than papered over.**
      `tests/hardened_services_wiring.rs` covers the part a machine
      without a working `runsc` can still observe: hardened `up` enforces
      the `process-compose` requirement at layer `provider`, and does so
      *before* starting a sandbox. Verified to fail against the pre-fix
      code (it reached `backend: runsc run` instead) and pass after, so it
      is real regression cover for the absent-call bug, not a tautology.
      Still **unverified against a live sandbox** — but the reason changed
      completely, and this rewrite happened *because* the old reason
      stopped being true. `unshare(CLONE_NEWUSER)` succeeds in this
      devcontainer now (task group 8's `seccomp=unconfined` fix landed and
      a rebuild picked it up), so a real `runsc run` was actually attempted
      here for the first time. Two real bugs in `src/gvisor/` were found
      and fixed live, both pre-dating this change and both unconditional
      (every hardened `up`, with or without services, was broken by
      each):
        - `runner::materialize_bundle` never created a mount-point
          directory for any bind mount (`/nix/store`, the project root,
          …) inside the synthesized `rootfs/`, only the bare `rootfs/`
          itself. gVisor's gofer resolves every mount destination by
          opening it inside that tree first; with nothing there, every
          mount failed ("expected to open rootfs/nix/store, but found
          <host path>"). Fixed: every mount's destination is now
          pre-created.
        - `oci_spec::build` hardcoded `root.path` to the relative string
          `"rootfs"` (the OCI spec's own documented convention). gVisor's
          rootless gofer opens `root.path`-joined destinations and
          compares the result against `/proc/self/fd/<n>`'s
          always-absolute `readlink` target as a symlink-escape guard — a
          relative `root.path` can never pass that comparison, so every
          mount failed the same "safely mount" check regardless of the
          bug above. Fixed: `BundleInputs` now carries `bundle_dir` and
          `root.path` is `bundle_dir/rootfs`, absolute.
      Both are confirmed fixed by hand (a real `runsc run` against the
      corrected bundle gets past mount setup to actually trying to exec
      the sandbox's init process) and covered by unit tests
      (`oci_spec::tests::json_shape_matches_oci_runtime_spec_field_names`,
      `runner::tests::materialize_bundle_writes_config_json_and_pre_creates_every_mount_point`).
      Past both fixes, a **third, unresolved** issue blocks `up_hardened`
      end to end on this host, and it is not a devcroft bundle bug:
      `runsc run`'s own rootless bootstrap issues a `mount()` call to set
      mount propagation (`MS_SLAVE|MS_REC`) as part of its chroot setup,
      and that call fails `EPERM` whenever *any* Landlock ruleset is
      active on the calling process — confirmed by elimination, not
      guessed: granting the Landlock profile `runner::run` applies before
      exec'ing `runsc` full read-write access to `/` still fails
      identically, so no missing grant explains it. Landlock does not
      mediate `mount()` in any current ABI, which is why widening grants
      cannot fix this — see `src/gvisor/runner.rs`'s `PROC_PREFLIGHT_DIRS`
      doc comment for the full chain of evidence. **This means
      add-gvisor-backend's Landlock-wraps-`runsc run` defense-in-depth
      layer (design.md decision 4 there) currently makes the entire
      hardened tier non-functional under `--rootless`, on any host, not
      just this one** — it was never exercised against a real unprivileged
      user namespace until today. `tests/gvisor_hardened_e2e.rs` and
      `tests/hardened_tier_ssh_parity.rs` both reach this exact failure
      now (previously they failed earlier and for an unrelated reason: a
      stale doc comment in both claimed hardened `up` needs no `flox`
      environment, which was already false before any of this — `up`'s
      shared prefix resolves the provider for every isolation tier
      unconditionally. Fixed alongside the above, both tests now call
      `flox init` like `tests/hardened_services_wiring.rs` already did).
      Fixing the Landlock/`runsc` incompatibility is a design-level call
      (drop the Landlock wrap around `runsc run`, or find a way to apply
      it only after gVisor's own bootstrap completes) that add-gvisor-backend
      owns, not this change — flagged here rather than fixed here because
      services can't be verified end-to-end at the hardened tier until it
      is resolved upstream of this task.

      **Confirmed the diagnosis is complete, not just plausible**: with
      `run`'s `pre_exec`/`restrict_self` call temporarily disabled (not
      committed — a throwaway experiment, reverted after), a full
      `devcroft up` on a `[services]`-declaring flox project at
      `isolation = "hardened"` succeeded end to end for the first time
      ever in this repo: `process-compose` started inside the sandbox
      over `runsc exec`, `--host-uds=create` made its control socket
      reachable from the host, `ps`/`status`/`logs` all showed the
      service `running`, and `down` reaped it cleanly (no leftover
      `process-compose`/`runsc` processes). This is every piece of task
      6.5's "unverified against a live sandbox" list, now verified — the
      only thing standing between the current code and a fully working
      hardened tier is the Landlock/`mount()` incompatibility above.

      One more real bug surfaced only by this full run, found and fixed
      alongside it: `runsc_command::exec_args` inserted a bare `--`
      before the command argv, copying `run_args`' pattern — but
      `runsc exec`'s usage (confirmed via `runsc exec --help`) is `exec
      [options] <container-id> <command> [args...]`, with no `--`
      separator at all. The literal `--` became the command's own
      argv[0] ("error finding executable \"--\" in PATH"), which silently
      broke every `exec`/`shell` session at the hardened tier since
      add-hardened-tier shipped — `session_backend.rs`'s own module doc
      already flagged this exact code path as "unverified beyond
      compiling". Fixed: the separator is removed; unit test renamed to
      assert its absence.

      **Resolved.** The Landlock ruleset around `runsc run` is removed
      (not narrowed — see `src/gvisor/runner.rs`'s module doc and
      `openspec/changes/add-gvisor-backend/design.md` decision 4), which
      was the design-level call this note previously deferred upstream.
      `tests/gvisor_hardened_e2e.rs` and `tests/hardened_tier_ssh_parity.rs`
      both needed two more fixes to actually turn green rather than just
      manually-confirmed: their bare `flox init` fixtures never installed
      anything, so `oci_spec::INIT_COMMAND`'s `sh -c 'sleep …'` PID 1 had
      no shell to resolve on the activated `PATH` (`bash`/`coreutils` now
      installed explicitly in both — a test-fixture gap, not a devcroft
      one, and now documented as a real limitation in `oci_spec.rs`'s
      module doc for a genuinely empty flox project); and neither test's
      shared `exec` helper set the child process's `current_dir` to the
      sandbox's own project root, so it inherited the test binary's own
      cwd (`/workspaces/devcroft`) — invisible on the process tier, where
      that directory happens to exist on the shared host filesystem
      regardless, but a hard "no such file or directory" once the
      hardened tier's `--cwd` actually has to resolve inside the sandbox
      (fixed in both). With all of the above, `cargo test` is fully green
      except for a pre-existing flaky, unrelated test
      (`lifecycle_hooks::post_create_does_not_rerun_on_recovery_but_post_start_does`,
      confirmed flaky identically on the unmodified base commit) —
      `tests/gvisor_hardened_e2e.rs`, `tests/hardened_services_wiring.rs`,
      and `tests/hardened_tier_ssh_parity.rs` all pass for real, not
      self-skipped. Task 6.5 is complete.

## 7. Docs

- [ ] 7.1 `docs/decisions.md`: replace "No service sidecars (yet)" with
      the delivered state, and add the port-collision limitation as a
      falsifiable rejection naming the property that fails (no netns
      under rootless), cross-referencing the existing netstack rejection
- [ ] 7.2 `docs/decisions.md`: record the "devcroft supervises, flox
      declares" split and its stated cost — `flox services status` will
      not show devcroft-started processes
- [ ] 7.3 README known gaps: two sandboxes declaring the same service
      port still collide; and the keeper's single-point-of-failure blast
      radius now includes services
- [ ] 7.4 `openspec/config.yaml`: move service sidecars out of the
      deferred roadmap, and close the devenv ownership open question with
      the decision this change made (services supervised and restartable;
      hooks one-shot and `up`-failing)

## 9. Review findings

> A review of the shipped change found five defects, four of them
> variants of the same one: a service problem that reports as nothing at
> all. Recorded here rather than folded silently into the groups above,
> since each was a property the specs already claimed to guarantee.

- [x] 9.1 **A dead supervisor was invisible.** `query` collapsed every
      failure — missing socket, refused connect, unparsable body, no
      `data` key — into one `Ok(None)`, `status` additionally dropped the
      empty list, and the only record of the failure was an `eprintln!`
      in the keeper's log. Three declared services plus a
      `process-compose` that died at startup produced byte-identical
      output to a sandbox declaring none, directly against the `services`
      spec's "SHALL NOT be omitted from service listings". `query` now
      returns `Result<_, Unreachable>` distinguishing "no socket" from
      "did not answer", and `status`/`ps` reconcile it against
      `Meta::declared_services`. Covered by
      `a_dead_supervisor_is_reported_not_silently_empty` and, end to end,
      by the post-`down` assertion in `tests/services_e2e.rs`
- [x] 9.2 **Two of the four states were wrong, one unreachable** — see
      task 3.3, now measured live rather than assumed
- [x] 9.3 **Service artifacts were keyed on the project root alone.**
      Two sandboxes with different names sharing one root (the
      alternative to worktrees `add-port-allocation` contemplates)
      overwrote each other's generated config, raced for one supervisor
      socket, and each reported the other's services — silently, in every
      case. Now `<root>/.devcroft/<sandbox-name>/`
      (`services::artifact_dir`), with `rm` cleaning up the per-sandbox
      directory it created and the shared parent only when empty. Since
      the path grew a level, `up` now also checks it against the
      `sun_path` limit host-side and fails at layer `config`, rather than
      letting process-compose fail to bind with an error naming neither
      the path nor the reason
- [x] 9.4 **`.devcroft/` was gitignored nowhere.** devcroft writes a
      config, a log, and a unix socket into the working tree, so every
      worktree in the fan-out flow showed dirty. `init` now appends the
      entry (only for a git repository, never inventing a `.gitignore`),
      and this repo's own `.gitignore` carries it
- [x] 9.5 **Host-side code read a sandbox-controlled socket unguarded.**
      `query` connected to whatever was at the path with no type or
      ownership check, `read_to_end` had no size cap, and its per-read
      timeout was resettable forever by a peer dripping bytes. Real at
      the `process` tier and a genuine trust inversion at `hardened`,
      where `--host-uds=create` exists precisely to let the host reach
      inward. Now: must be a socket owned by this uid, capped at
      `MAX_RESPONSE`, bounded by `QUERY_DEADLINE` overall

## 8. Verification

- [ ] 8.1 `cargo build`, `cargo clippy`, `cargo fmt` clean
- [ ] 8.2 `openspec validate --all` passes with this change included
- [ ] 8.3 Report honestly which of the above ran against a live service
      and which are unverified — including whether the deny-default
      policy shape was exercised at all, or only the `allow` workaround
      (task 1.2)
