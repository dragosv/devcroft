## 0. Measurement gate — after `add-mise-provider` Group 0, before any code here

Record each result in `design.md` under "Evidence", tagged as measured, with versions.

- [ ] 0.1 Pin the mise and pitchfork versions. From fixtures run under real mise, record: the complete `[daemons]` key list and which keys mise consumes versus passes through; whether `init` shares a shell with `run` (do its exports and `cd` reach it); when `retry` applies (startup only, or any failed exit); the stop timeout used when `stop_signal` carries none; which shell a string `run` goes through.
- [ ] 0.2 Pin the process-compose version. Record: that probes are `exec` and `http_get` only; what it does when a readiness probe exhausts `failure_threshold`; whether a process that exited before an `exec` probe's initial delay is ever marked ready; whether it reads `shutdown.timeout`, `shutdown.timeout_seconds` or both, and whether it escalates to SIGKILL with the key it does read. If the key devcroft emits today is ignored, fix that first and re-run the existing teardown tests.
- [ ] 0.3 Measure whether devcroft's own binary is executable from inside a sandbox, on Linux's mount view and under Seatbelt.
- [ ] 0.4 On the pinned mise, record which backend each preset's server tool resolves to and what its lock entry contains. This changes nothing in this change; it is the evidence design D8 names for reopening presets.
- [ ] 0.5 Stop criterion. If `process-compose` cannot be installed through a backend `add-mise-provider` admits, stop and report to the owner.

## 1. Supersede the placeholder refusal

- [ ] 1.1 Remove the `config` requirement "mise daemons are refused until supported" from `add-mise-provider`'s delta if that change is unarchived; if it has been synced, add a `REMOVED Requirements` delta here instead.

## 2. Declared shutdown grace (design D5), independent of mise

- [ ] 2.1 Compute a sandbox's service grace at `up` (largest declared `Shutdown::Signal` grace, floored at today's value) and record it in `Meta`. Refuse a declared grace above `MAX_SERVICE_GRACE_SECS` by service and key at layer `provider`.
- [ ] 2.2 `src/services/mod.rs`: render the declared grace instead of clamping it. `Shutdown::Default` and `Shutdown::Command` keep `SHUTDOWN_TIMEOUT_SECS`.
- [ ] 2.3 Keeper: wait the service grace before sweeping service process groups; sessions keep `DEFAULT_GRACE_PERIOD`. Rewrite the comments in `src/keeper/mod.rs`, `src/services/mod.rs` and `src/bin/devcroft.rs` that state the two are one number.
- [ ] 2.4 `src/lifecycle/terminate.rs`: `down` waits `GRACE_PERIOD` plus the recorded service grace before killing the keeper.
- [ ] 2.5 Tests: a service that takes four seconds to stop on its declared signal exits cleanly; one that ignores the signal is gone after its grace and nothing survives `down`; the ordering supervisor < keeper < `down` asserted on computed values; a devenv fixture with a grace above one second, with the behavior change recorded in `docs/implementation-log.md`.

## 3. TCP readiness (design D4)

- [ ] 3.1 Add `Probe::Tcp { host, port }` in `src/provider/mod.rs` and render it in `src/services/mod.rs` as an `exec` probe running devcroft's binary with `__probe-tcp`.
- [ ] 3.2 Add the hidden `__probe-tcp <host> <port>` subcommand in `src/bin/devcroft.rs`: connect with a timeout, exit zero on success. Keep it out of `USAGE`.
- [ ] 3.3 If 0.3 found the binary unreachable, grant its file read-only with origin `baseline` and show it in `policy --render`.
- [ ] 3.4 Tests: a service becomes ready only once its port accepts a connection; a dependent waits for it; a probe that never passes leaves `up` successful and `status` at `started-but-not-ready`.

## 4. The mise daemon reader (design D1 to D3, D8)

- [ ] 4.1 In `src/provider/mise.rs`, read `[daemons]` with devcroft's parser and no subprocess. Require `[settings] experimental = true`; refuse the table by name without it. Require `process-compose` among the locked tools, with the add-and-lock hint.
- [ ] 4.2 Translate the admitted keys of design D2 into `ServiceDecl`, capturing the rest with `provider::Unconsumed` and refusing the first that declares something by daemon and dotted key. Return `ServiceSupport::Declared`, empty when the table is absent.
- [ ] 4.3 Explicit refusals with reasons: `retry = true`; non-scalar `port` and `ports`; `mise = false`; object-form ready checks and their `timeout`; `ready_output`; explicit `ready_delay`; the forms in design D8.
- [ ] 4.4 Render `init` as 0.1 measured, and the implicit `ready_delay` as design D3 decides after 0.2.
- [ ] 4.5 Refuse dependency cycles and unknown targets before any service starts.

## 5. Ports and the cross-provider check

- [ ] 5.1 `src/lifecycle/ports.rs`: when a daemon's `ready_port` equals its declared `port` and that port is allocated, the probe follows the allocation; refuse a service whose `ready_port` differs from an allocated `port`.
- [ ] 5.2 `src/lifecycle/up.rs`: add a best-effort read of mise `[daemons]` to `ensure_no_services_declared_for_another_provider`, under its existing condition and its existing `--skip-hooks` exemption.

## 6. Tests, sample, documentation

- [ ] 6.1 Translation tests for every admitted key and a refusal test for every key named in design D2 and D8, plus one unknown nested key.
- [ ] 6.2 Lifecycle tests: opt-in absent, supervisor tool absent, `--skip-hooks`, a flox project carrying mise daemons is not refused, a nix project carrying them is.
- [ ] 6.3 `samples/mise-services-sample`: two daemons with a `depends` edge and a `ready_port`, with a README listing what is and is not carried, presets first.
- [ ] 6.4 Once the above pass: `docs/decisions.md` (services section, the preset decision), `README.md`, `docs/known-gaps.md`, and the services framing rule in `CLAUDE.md`.
