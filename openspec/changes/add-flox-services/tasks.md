## 1. Blocking-dependency gate

- [ ] 1.1 **Do not start task group 2 until a sandbox can bind a
      listening socket under a deny-default policy.** Verify directly,
      not by reading a changelog: bring up a sandbox with
      `network.default = "deny"` and run a real loopback bind
      (`python3 -c "…bind(('127.0.0.1', 0))…"`). Today this fails with
      `Operation not permitted` — see proposal.md's Blocking Dependency.
      If it still fails, stop and report; every task below produces a
      feature whose only working configuration disables the sandbox's
      network policy.
- [ ] 1.2 Record which policy shape the integration tests will use, and
      whether it is the default one. If services can only be tested under
      a non-default policy, that fact belongs in the test module doc, not
      discovered later by a reader.

## 2. Provider contract: service declarations

- [ ] 2.1 Extend the `Provider` trait with a service declaration
      alongside the existing `Resolution` — an explicit "supports none"
      variant, not an empty list, per the `env-provider` delta spec's
      "declares services or explicitly declares none"
- [ ] 2.2 `src/provider/flox.rs`: read declared services host-side during
      resolution (trusted phase). Prefer a flox-provided machine-readable
      listing if one exists; otherwise parse `[services]` from the flox
      manifest. A schema shape flox no longer produces SHALL fail loudly
      at `up`, never yield a silently empty list (design.md, schema-drift
      risk)
- [ ] 2.3 `src/provider/nix.rs`: declare no service support explicitly
- [ ] 2.4 `up` fails at layer `provider`, exit code 3, when services are
      requested from a provider that supports none — naming the provider
- [ ] 2.5 Unit tests: flox manifest with/without `[services]`; a nix
      project asking for services fails with the right layer and code;
      a malformed `[services]` fails loudly rather than resolving empty
- [ ] 2.6 Regression test: `policy --render` is byte-identical for the
      same manifest with and without services declared — the
      `env-provider` delta's "do not widen the policy" requirement

## 3. Service supervision in the keeper

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
