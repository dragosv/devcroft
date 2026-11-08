# Change: add-flox-services

Status: proposed (post-MVP). Depends on: `add-mvp-core` complete, **plus
one hard blocker that is not yet a change** — the listening-socket gap
(see Blocking Dependency below). This change is the concretization of the
"Service sidecars (postgres, redis) via provider service support (flox
`[services]`, devenv) supervised by the keeper" item already on the
roadmap in `openspec/config.yaml`, and the mitigation `docs/decisions.md`
promises under "No service sidecars (yet)".

## Blocking Dependency — RESOLVED

**This section was wrong, and is kept rather than deleted because the
error is instructive.** It claimed that "no outbound access, but my dev
server still runs" was inexpressible in the policy model, and that this
change therefore could not be implemented until a separate, larger change
landed.

That premise was never checked against nono's own profile schema. It is
false: the schema has always carried an `open_port` field ("Localhost TCP
IPC (connect+bind)"). devcroft simply never emitted it — `NonoNetwork`
projected only `block` and `allow_domain`. The gap was one unemitted
field, not a limitation of the model.

Resolved by the `network.ports` manifest key (implemented, tested end to
end): a sandbox with `network.default = "deny"` and `ports = [5432]`
binds `127.0.0.1:5432` while egress stays filtered and every ungranted
port stays denied. `open_port` rather than the adjacent `listen_port` was
settled empirically — against nono 0.71.0 on Linux, `listen_port` granted
neither a loopback nor a `0.0.0.0` bind.

The lesson worth keeping: this was asserted as a hard architectural
blocker across a whole design discussion, and one command against
`nono profile schema` refuted it. A rejection whose premise was never
tested is not a rejection, and `docs/decisions.md`'s own rule — revisit
when the stated reason stops holding — applies to blockers too.

## Why

Every dominant coding-agent environment pattern leaves stateful services
unsolved (README, "How coding-agent products provision environments
today"): the choice today is baking Postgres into an image as a
`services:` block, or hand-allocating a port and schema per agent. For a
fleet of parallel agents this is the difference between real isolation and
theatre — agents that share one host Postgres corrupt each other's test
data no matter how well the filesystem is isolated.

flox already solves the declarative half. A flox manifest's `[services]`
section (`myservice.command = "…"`, driven by `flox services
start|stop|status|restart|logs`, verified against flox 1.14.0) describes
services in the same lockfile-pinned manifest devcroft already treats as
the source of truth. What is missing is the supervision half: devcroft
never starts them, never tracks them, and never stops them.

## What Changes

- A new `services` capability: services declared by the resolved provider
  are started inside the sandbox after `up`, tracked by the keeper for
  their whole lifetime, and stopped on `down`/`rm`.
- Services run **inside the boundary, after restriction** — the same
  posture as hooks, and for the same reason: a service `command` is
  project code. It never receives provisioning privileges, and a service
  needing network access needs an allowlist entry like anything else.
  This follows directly from the two-phase execution invariant and is not
  negotiable within this change.
- The keeper supervises services as tracked, persistent child processes,
  not as a fire-and-forget shell-out. `ps` lists them alongside sessions,
  `logs` includes their output, and `down`/`rm` reap them.
- `env-provider` gains an optional service-declaration capability on the
  `Provider` trait. Providers that have no service concept (`nix` today)
  return none, and a manifest asking for services under such a provider
  fails at layer `provider` naming the provider — never silently doing
  nothing.
- **Ownership decision, recorded here:** provider-declared services and
  devcroft hooks are separate mechanisms with a stated precedence, closing
  the open question `openspec/config.yaml` flags against devenv ("its
  built-in services overlap with devcroft hooks, ownership must be decided
  first"). Services are supervised and restartable; hooks are one-shot and
  fail `up`. A hook that starts a long-lived process remains supported and
  remains the user's problem to reap — this change does not retroactively
  adopt it.
- Port allocation is **explicitly out of scope** and stated as a known
  limitation (see Success Criteria): N sandboxes each starting Postgres on
  5432 still collide with `EADDRINUSE`, because no PID/mount/net namespace
  separates sandboxes (`add-mvp-core` design.md Decision 5) and gVisor does
  not fix it under rootless (`docs/decisions.md`, "Rejected (for now):
  non-rootless gVisor for netstack"). Per-sandbox port allocation is named
  as the follow-up this change deliberately does not attempt.

## Capabilities

### New Capabilities

- `services`: service lifecycle — declaration discovery from the provider,
  start ordering relative to hooks, keeper supervision and restart policy,
  teardown on `down`/`rm`, and failure semantics when a service exits.

### Modified Capabilities

- `env-provider`: the `Provider` trait gains an optional service
  declaration alongside the existing `Resolution`; flox implements it from
  `[services]`, and providers without a service concept explicitly declare
  none rather than being assumed to.
- `lifecycle`: `up` starts services after hooks; `down` and `rm` stop them
  before tearing down the keeper; `status` reports service state; `ps` and
  `logs` cover services as well as sessions.
- `cli`: `status`/`ps`/`logs` output gains service rows; `doctor` reports
  whether the current host can actually bind listening sockets under the
  compiled policy, so the Blocking Dependency above is visible as a
  diagnostic rather than as a mysterious runtime failure.

## Impact

- Affected specs: new `services`; modified `env-provider`, `lifecycle`,
  `cli`.
- Affected code: new `src/services/` (declaration model, supervision);
  `src/provider/` (trait extension, `flox.rs` parsing `[services]`,
  `nix.rs` returning none); `src/keeper/` (supervised child processes
  alongside the session registry); `src/lifecycle/` (`up` ordering,
  `down`/`rm` teardown, `status`); `src/bin/devcroft.rs` (`ps`, `logs`,
  `status`, `doctor` output).
- No change to policy compilation: services are governed by the same
  compiled `[network]`/`[filesystem]` rules as every other in-sandbox
  process. This is deliberate — a service that needs a port is asking the
  manifest for it, not asking devcroft for an exemption.
- MVP's command surface stays closed: this change adds **no** new
  top-level command. Service state is surfaced through the existing
  `status`/`ps`/`logs` rather than a new `devcroft services` verb; if a
  dedicated verb turns out to be necessary, that is a separate proposal.

## Success Criteria

- A flox project declaring a service in `[services]` comes `up` with that
  service running inside the sandbox; `devcroft ps` shows it; `devcroft
  logs` includes its output.
- `devcroft down` leaves no service process behind — verified by asserting
  the process is gone, not by trusting the stop command's exit code.
- A service whose command exits non-zero at startup surfaces as a failed
  service in `status` with its log tail, and does not silently look
  healthy.
- A manifest declaring services under a provider with no service concept
  (`nix`) fails at layer `provider`, exit code 3, naming the provider.
- Services are subject to the sandbox's compiled policy: a service denied
  a port by `[network]` fails visibly with the same denial any other
  in-sandbox process would get.
- **Stated, not fixed:** two sandboxes declaring the same service port
  still collide. The limitation is published in the README's known gaps
  with the follow-up named, rather than discovered by a user.

## Open Questions

- **Restart policy.** flox's own supervisor already restarts services;
  layering devcroft supervision on top risks two supervisors fighting.
  Decide whether devcroft supervises `flox services` as a single unit
  (simplest, one process to reap, but service-level granularity in `ps`
  becomes a parsing exercise against `flox services status`) or
  supervises each declared service directly (better granularity, but
  reimplements what flox already does and diverges from what `flox
  services` reports).
- **Start ordering vs `post_start` hooks.** A hook that expects a
  database to be up must run after services; a service that expects a
  hook-seeded fixture must run after hooks. Both are real. Leaning:
  services first, `post_start` after, documented — but this forecloses
  the second case and may need an explicit dependency key later.
- **Are services part of the staleness fingerprint?** Editing
  `[services]` changes the environment's behavior but not its closure.
  Leaning: yes, since `manifest.toml` is already fingerprinted whole —
  which means editing a service command flips `status` to stale and
  requires `up --recreate`, possibly heavier than users expect.
- **Whether `up` should wait for readiness**, and if so how readiness is
  expressed. A `command` alone cannot say "ready"; a health-check key
  would be new manifest surface devcroft does not own (it is flox's
  manifest). Leaning: start-and-report, never block `up`, with readiness
  left to the project's own hooks.
