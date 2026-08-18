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
- [x] 2.7 Full documented schema, not just `command`: `vars`,
      `is-daemon`, `shutdown.command`. Found by checking flox's docs
      rather than assuming — `vars` carries the service's port in flox's
      own documented example, so dropping it starts services on the wrong
      port while the command string still looks right. Non-string vars are
      rejected rather than stringified, and `is-daemon` without
      `shutdown.command` is rejected at resolution (such a service is
      unstoppable: the launcher exits by design, so killing it at
      teardown reaps nothing)
- [ ] 2.8 Reconsider `nix` returning a flat `Unsupported`. A plain
      `devShell` genuinely has no services, so this is correct for the
      interface devcroft consumes — but a flake using
      [services-flake](https://github.com/juspay/services-flake) *does*
      declare services, exposed as a separate flake app (`nix run
      .#services`) rather than in the devShell. Such a project brought up
      under devcroft gets silence. Service support is a property of the
      project, not of the provider, so detecting that output is the
      honest fix

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
      this work identically at `process` and `hardened`
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
      assuming clean JSON fails on the first call. **Confirm against a
      real failing service** what `status` reads for failed-at-start vs
      exited-later; only the running case is verified so far
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

- [ ] 6.1 Integration test, gated on real `flox` the way this repo's
      existing real-tooling tests self-skip: a project declaring a
      service comes up, the service runs inside the sandbox, `ps` shows
      it, `logs` has its output
- [ ] 6.2 Teardown test: after `down`, the service process is gone from
      the host — asserted by process absence
- [ ] 6.3 Failure test: a service whose command exits non-zero is listed
      as failed with a reachable log tail, and the sandbox stays usable
- [ ] 6.4 Policy test: a service denied a port by `[network]` fails
      visibly with the same denial any in-sandbox process would get
- [ ] 6.5 Cross-tier test: the same service declaration behaves
      identically at `process` and `hardened`, in the shape
      `tests/hardened_tier_ssh_parity.rs` already uses — self-skipping
      when `runsc` is not functionally usable

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

## 8. Verification

- [ ] 8.1 `cargo build`, `cargo clippy`, `cargo fmt` clean
- [ ] 8.2 `openspec validate --all` passes with this change included
- [ ] 8.3 Report honestly which of the above ran against a live service
      and which are unverified — including whether the deny-default
      policy shape was exercised at all, or only the `allow` workaround
      (task 1.2)
