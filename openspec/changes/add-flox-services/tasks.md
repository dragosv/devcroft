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

## 3. Service supervision in the keeper

> **Unresolved design conflict, found while implementing — read before
> starting this group.** design.md decision 4 has `up` start services
> after hooks, and decision 2 has them spawn through `SessionBackend`.
> Both are individually right; together they do not work. Hooks are
> spawned over the keeper's control socket by `up`, which then exits —
> and a session whose client disconnects is escalated after
> `connection::DEFAULT_GRACE_PERIOD` (2s). A service started the way a
> hook is started would therefore be killed ~2 seconds after `up`
> returns. Nothing holds the connection, because `up` is a short-lived
> CLI process by design.
>
> So the keeper must own service lifetime, not `up`. That means either
> (a) the keeper starts services at its own startup, which puts them
> *before* hooks and contradicts decision 4's ordering, or (b) a new
> protocol frame lets `up` tell the keeper to start services after hooks
> complete, keeping the ordering but adding protocol surface. Decide
> this before writing code; do not resolve it by holding a connection
> open from `up`, which would make service lifetime depend on a process
> whose whole contract is to exit.

- [ ] 3.1 Service model and registry alongside the existing session
      registry: per-service state distinguishing not-started,
      failed-at-start, running, and exited-later (the `services` delta
      spec requires all four be distinguishable)
- [ ] 3.2 Start each declared service's command through the existing
      `SessionBackend` trait, without a pty and with no attached client
      (design.md decision 2). **Do not** shell out to `flox services`,
      and do not add a tier-specific path — going through the trait is
      what makes this work identically at `process` and `hardened`
- [ ] 3.3 Capture service output with per-service attribution for `logs`
- [ ] 3.4 No automatic restart (design.md decision 3): an exited service
      records its exit and stays dead. Explicitly assert this in a test
      so a future "helpful" restart cannot land silently
- [ ] 3.5 Teardown: stop all services before the keeper exits, SIGTERM
      escalating to SIGKILL after the same grace period sessions use
- [ ] 3.6 Test at both tiers that a service ignoring SIGTERM is still
      gone after teardown — asserted by observing process absence, never
      by trusting a stop command's exit status

## 4. Lifecycle wiring

- [ ] 4.1 `up`: start services after the keeper is responsive and after
      `post_create`/`post_start` hooks (design.md decision 4)
- [ ] 4.2 `up --skip-hooks` also skips services, preserving "nothing
      project-supplied runs"; services report as not-started, not failed
- [ ] 4.3 A failed service does not fail `up` — `up` exits 0, prints the
      failure, and `exec`/`shell`/SSH still work (the `services` delta's
      "do not block sandbox availability")
- [ ] 4.4 `down`/`rm` stop services before tearing the keeper down
- [ ] 4.5 Services start on every keeper start (`post_start` semantics,
      not `post_create`); no attempt to preserve process state across
      `down`/`up`
- [ ] 4.6 `SandboxStatus` gains service state, with the same
      forward/backward-compatible posture the isolation-tier field used —
      sandboxes that predate this read as having no services

## 5. CLI surface

- [ ] 5.1 `ps` lists services alongside sessions, labelled so the two are
      distinguishable
- [ ] 5.2 `logs` includes service output attributed per service
- [ ] 5.3 `status` shows service state, so a healthy keeper with a failed
      service is not reported as simply healthy
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
