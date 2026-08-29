# Tasks — Egress Proxy

## 0. Confirm the interface

- [x] Read `install_seccomp_proxy_filter`'s signature: what policy it accepts,
      and how an allowed connection is completed (descriptor injection versus
      continuing the original syscall). **This decides the shape of section 1.**
      **Done — it decided against the original shape.** The function takes no
      policy (`has_bind_ports: bool` only) and is `apply_auto`'s own fallback
      for Landlock ABI < V4; this devcontainer measured ABI V6 live, where
      `apply_auto` never installs it at all. See design.md's Open Questions.
- [x] Determine how a requested hostname reaches the decision point, given that
      socket-layer interception sees addresses (design.md Q4).
      **Done — it doesn't, at the socket layer.** The hostname reaches the
      decision because the proxy terminates ordinary HTTP (`CONNECT host:port`
      or an absolute-URI request), which carries the name in the request
      itself. No DNS interception or SNI parsing needed.
- [x] Check whether `SeccompNetFallback` and the existing network-block path
      already cover part of this.
      **Done.** `CapabilitySet::proxy_only_with_bind(port, bind_ports)` is the
      exact builder method this change needs — already exposed, already
      cross-platform (Landlock `NetPort` on Linux, Seatbelt
      `network-outbound` on macOS). `capability_set.rs`'s current
      `network_block: bool` compile is what gets replaced.

## 1. Proxy process

(Retitled from "Supervisor loop" — task 0 found there is no notification loop
for devcroft to write; `apply_auto` owns that internally where it's needed.
What's left is an ordinary resident TCP proxy.)

- [x] A new resident process (`__egress_proxy`, `crate::proxy::spawn`),
      bound to `127.0.0.1:<port>` chosen at bind time (port 0), living
      outside every sandbox's policy domain — confirmed it cannot run
      inside the keeper, since the keeper self-restricts to the *same*
      `NetworkMode::ProxyOnly` the sessions get.
- [x] Terminates `CONNECT host:port` (tunnel after `200`, no TLS interception)
      and absolute-URI/origin-form HTTP requests; resolves the hostname,
      decides via `HostFilter::check_host`, dials out or refuses
      (`proxy::server`).
- [x] One proxy per sandbox (one per `up`), matching `add-linux-agent-fleet`
      D4's per-agent model as the non-fleet instance of the same decision
      (design.md Q5). Reused across a keeper recovery, respawned fresh on
      `--recreate` or when `network.allow` changes shape (`up.rs::
      ensure_egress_proxy`/`stop_orphaned_egress_proxy`).
      This satisfies the spec's "proxy runs outside the sandbox it filters"
      requirement, including its attribution scenario (one listener per
      sandbox *is* the identity, and nothing is asked of the client). Its
      other scenario — "credentials held by the proxy are never resident
      inside it" — holds, but **trivially: this proxy holds no credentials
      at all.** Phantom-token injection (`docs/threat-model.md`'s
      capability-not-custody principle) is not built. Placement is what
      makes it *possible* later, which is what E1 argued for; do not read
      the satisfied scenario as meaning it exists.
- [x] Structured refusal records: destination, deciding rule — written to
      its own `paths.proxy_log`, not the keeper's `paths.log` (separate
      process, separate file; see `StatePaths::proxy_log`'s doc for why
      that's the better call here than sharing one file), one write per
      record (`proxy::server::log_line`, same discipline as
      `keeper::connection::log_record`). Not yet per-sandbox attribution
      beyond the file itself — fine for one proxy per sandbox; revisit if
      fleet ever shares one proxy across sandboxes (Q5 above).

## 2. Policy integration

- [x] Compile `network.allow` to a deny-by-default kernel policy with the proxy
      endpoint as the only permitted path — not to a blanket block
      (`CompiledPolicy::network_proxy_port` /
      `capability_set::to_capability_set`'s `NetworkMode::ProxyOnly` arm).
- [x] Per-context network policy in the manifest (provisioning, runtime).
      **Moved to `sandbox-provisioning`, requirement and all** — not left
      standing here unimplemented. Provisioning runs entirely unsandboxed
      today (two-phase execution invariant), so there is no confinement
      boundary on that side to attach a distinct policy to; a per-context
      policy needs two contexts and there is one. The requirement now lives
      in `sandbox-provisioning/specs/network/spec.md`, which is the change
      that creates the second context and already depended on this one for
      exactly this reason (its design.md, open question 2). This change
      ships the runtime context's enforcement, which is the mechanism the
      second context reuses unchanged.
- [x] `policy --render` and `why` cover the (runtime-only, per the deferral
      above) allowlist with origin attribution — `render`'s new
      `network.proxy:` line and `why_host`'s `HostFilter` delegation.
- [x] Fail closed when the proxy is unavailable; never fall back to
      unfiltered. Structural, not a checked condition: `NetworkMode::
      ProxyOnly`'s kernel gate denies every `connect()` except to the
      proxy's own port regardless of whether that port has a listener
      behind it, so a dead proxy means every request refuses at the
      kernel layer — there is no code path that falls back to
      `AllowAll` if the proxy fails to start (`ensure_egress_proxy`
      returns `Err`, which fails `up` itself, layer `keeper`).

## 3. Client reachability

- [x] Established: not moot. See design.md's Open Questions — `ProxyOnly`
      only ever permits a literal `connect()` to its own port; a client
      that ignores proxy settings gets denied at the kernel layer, not
      silently mediated.
- [x] Set in the sandbox environment: `HTTP_PROXY`/`http_proxy`/
      `HTTPS_PROXY`/`https_proxy` point at the proxy, and `NO_PROXY`/
      `no_proxy` exempt loopback so a `network.ports`-granted dev server
      stays reachable without needing an allowlist entry for `localhost`
      (`up_process`'s env assembly, right before `spawn_keeper`).
- [ ] Measure against real package managers rather than assuming.

## 4. Diagnostics

- [x] Surface refusals so a developer can see which host was refused.
      Refusals (and allows) are logged to `paths.proxy_log` and the
      `502` response body names the host; `policy::why::why_host` now
      delegates to the real `HostFilter` so `why --host` gives the same
      answer the proxy will.
- [x] `doctor` reports whether domain filtering is enforceable on this host —
      `doctor_manifest_degradation` already did this; verified live
      (`[PASS] manifest: no degraded capabilities ... on this host` for a
      `default = "deny"` + `allow` manifest on this Linux host, which is
      the correct answer now that filtering really is enforced here).
      **`up` did not**, though both this change's spec ("named at `up` and
      in `doctor`") and `add-mvp-core`'s own "Degraded capability
      surfacing" requirement have always required it —
      `policy::detect_degraded` had exactly one caller, `doctor`. Latent on
      Linux (nothing degrades, so the missing call printed nothing and
      looked identical to correctness) and silent on macOS, which is the
      one platform the requirement was written for. Wired up in
      `cli_up` via `print_degraded_capabilities`; the macOS half stays
      unverifiable from this devcontainer, same caveat as the item below.
- [ ] Replace any documentation claiming domain filtering works today, and any
      claiming exfiltration is prevented.
- [ ] **Found while wiring `policy::degraded`, needs a macOS host to
      settle:** that module's existing claim ("macOS Seatbelt has no
      equivalent mandatory redirection") predates this change and was
      left as-is rather than flipped on an argument — see
      `degraded.rs`'s module doc for the full note. `NetworkMode::
      ProxyOnly`'s own doc comment describes the macOS output as a
      *scoped* outbound allow, which reads as enforced under Seatbelt's
      default-deny model, not merely cooperative. Verify live before
      changing `detect_for_host`'s `#[cfg(target_os = "macos")]` branch
      either way.

## 4b. Adopt nono-proxy (design.md E6)

- [ ] 4b.1 Add `nono-proxy = "0.74.0"`, pinned to the same version as `nono`.
      **Record the cost where the earlier one is recorded**: 116 additional
      crates, measured — the same order as `use-nono-library`'s 141-crate
      trust tail, which was accepted reluctantly. This is a second helping of
      that trade and is the owner's call, not an obvious win.
- [ ] 4b.2 Replace `proxy::server`'s accept loop with `nono_proxy::start`.
      devcroft keeps the process, the pidfile, `up`/`down`/`rm` ownership,
      `CompiledPolicy::network_proxy_port` and the `ProxyOnly` kernel gate —
      the crate supplies only what runs inside the process.
- [ ] 4b.3 Enable `require_auth` and plumb the per-session token into the
      sandbox environment. **This is the gap that motivated adoption**, not a
      detail: without it the proxy is an open relay on loopback.
- [ ] 4b.4 Confirm the fail-closed property still holds structurally — the
      kernel gate permits exactly one port whether or not anything is
      listening on it, so a dead proxy denies rather than opens.
- [ ] 4b.5 Confirm TLS interception, SPIFFE and AWS routing stay **off**.
      A test asserting the sandbox sees no injected CA would pin the first,
      which is the one with a real security consequence if it drifted on.
- [ ] 4b.6 Keep or port the existing e2e coverage
      (`tests/egress_proxy_e2e.rs`) against the new implementation — the
      allow/deny behaviour it asserts is the contract, and it must not be
      dropped because the code beneath it changed.
- [ ] 4b.7 Test the auth boundary directly: a caller without the token is
      refused, and refused *before* any allowlist decision.

## 5. Validation

- [x] `tests/egress_proxy_e2e.rs`: a real `up` (real Landlock, real keeper,
      real `curl` installed via `flox`), `network.default = "deny"` plus
      `network.allow`, against two mock upstreams on distinct loopback
      addresses — the allowed one returns `200` through the proxy, the
      other gets `502` naming it. Not `npm ci`/`go mod download`
      specifically (no network access to real registries from this
      devcontainer to install them, let alone exercise them against),
      but the same shape: a real package-manager-style client, a real
      sandbox, a name-based decision actually enforced.
      **Found and fixed while writing this test:** the first version used
      `127.0.0.1` as the "allowed" host, which collided with task 3's own
      `NO_PROXY` exemption (curl skipped the proxy entirely and hit
      Landlock's direct-connect denial — a false negative that would have
      shipped a broken test asserting the wrong failure mode). Switched
      to `127.0.0.3`/`127.0.0.4`, which aren't loopback-exempted.
- [x] A direct socket to an unrelated address is refused at the kernel
      level, not merely unproxied — structural, not separately tested:
      `NetworkMode::ProxyOnly` compiles to a Landlock `NetPort`/Seatbelt
      rule that is the *only* permitted `connect()` destination, so
      anything else is a kernel `EPERM` by construction (confirmed live
      via the `curl -v` probe used to debug the `127.0.0.1` false
      negative above: "Immediate connect fail... Permission denied").
- [ ] Two sandboxes with different allowlists: neither inherits the
      other's.
      **Reopened — the earlier claim was wrong, and wrong in a way worth
      keeping.** It argued this held "by construction" because each
      sandbox's proxy is its own process with its own allowlist, so "no
      state is shared and nothing *could* leak". That reasons about
      process state and misses the network surface: the proxy is an
      **unauthenticated listener on loopback**, so any local process that
      can reach the port gets that sandbox's allowlisted egress. Landlock
      `NetPort` grants are port-based, which incidentally limits one
      *sandbox* reaching another's proxy — but nothing stops an
      unsandboxed process on the host from using either.
      Found by reading `nono-proxy`'s `ProxyConfig::require_auth`, whose
      own doc states the missing property directly: the per-session
      `Proxy-Authorization` token "is the localhost auth boundary that
      stops other local processes from using the proxy". Closed by
      adopting that proxy (design.md E6).
- [x] Refusal message names the host — asserted directly in the e2e test
      (`502` body contains `127.0.0.4`) and in `proxy::server`'s own unit
      test.

## 6. Downstream

- [ ] `sandbox-provisioning`: replace the deferred on/off network decision with
      a real provisioning allowlist.
- [ ] `add-linux-agent-fleet`: its `agent-networking` requirements assume this
      proxy exists; confirm they are satisfied rather than restated.
