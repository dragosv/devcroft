# Spec review, 2026-09-14 — findings, verified

An external review of the OpenSpec change set (17 findings, 4 secondary
observations) checked here against the code and the spec text it cited.
Kept because `openspec validate --all` is green (35 passed) and proves
only that artifacts are well-formed — it says nothing about whether two
changes contradict each other, which is what most of this is. Every
verdict below names what was read to reach it; a reader who disagrees
should be able to open the same lines.

**Verdict vocabulary.** *Confirmed*: the finding is right as stated.
*Partial*: the underlying fact is right, the framing is not. *Fixed*: a
code change landed in this repo. *Decision*: needs the owner — an
archive, a supersession, or a design call on an unstarted change.
*Edit*: a small spec/code correction with no decision attached.

## Fixed

### 1. Stale-keeper recovery left the old egress proxy running — confirmed, fixed

`src/lifecycle/up.rs` `Health::Stale` branch called
`state::clear_runtime_state`, which removes `proxy.pid` and whose own
comment said the kill "happens before this runs" — true for `down` and
`--recreate`, not for recovery. A recovered sandbox therefore ran two
proxies: the new one and an orphan still answering with the previous
allowlist and token, owned by nobody.

Fixed in `02ba511`: `terminate_and_wait` on the proxy pidfile before the
clear, as the other two paths already did. Regression test
`tests/egress_proxy_e2e.rs::recovering_a_dead_keeper_stops_the_orphaned_egress_proxy`
— up, SIGKILL the keeper, up again, assert the first proxy's pid is gone
and the pidfile names a new one. Passes with the fix; without it, fails
on exactly the orphan assertion (measured).

## Confirmed — decisions

### 2. Fleet prescribes two PID-1 topologies — confirmed, decision

`add-linux-agent-fleet/specs/agent-supervisor/spec.md` ("Init helper is
single-threaded and re-executed"): the re-executed helper applies setup
"then exec[s] the agent command". `specs/sandbox-runtime/spec.md` ("The
init helper is PID 1"): "SHALL NOT replace itself with the workload; if
PID 1 exits…". An `exec` is the replacement the second forbids. Policy
application is also assigned to both the helper and the keeper across
`specs/lifecycle/spec.md`. Pick one topology — the second spec's
(supervisor → init/PID 1 → restricted keeper → sessions) matches the
current keeper design — and one owner for the policy.

### 3. Fleet egress mandates seccomp-notify against a suspended premise — confirmed, decision

`specs/agent-networking/spec.md` requires the proxy-only seccomp filter,
slirp and listener transfer (D9). `design.md` §D9 and CLAUDE.md both say
the premise (a userspace network helper making proxy variables
cooperative) was invalidated by the shipped loopback-only namespace plus
unix-socket relay — `src/lifecycle/up.rs` (proxy socket handed to the
view) and `src/proxy/server.rs` are that relay. CLAUDE.md's standing
instruction is "re-derive before building either way"; the spec has not
been re-derived. Either the fleet adopts the shipped topology (and D9
goes, taking fleet's hardest phase-0 item with it) or it does not, and
the design says why.

### 4. Agent workload reopens unconfined activation — confirmed, decision

`add-agent-workload/specs/tooling/spec.md`: "resolve the tooling layer
once, host-side, during `up`, before restrictions are applied".
`sandbox-provisioning/specs/env-provider/spec.md` exists to confine
exactly that step. Today's code resolves the provider before restriction
by design (CLAUDE.md, two-phase execution) — so the workload spec is
consistent with the present and inconsistent with the change that is
meant to replace the present. Make `add-agent-workload` depend on
`sandbox-provisioning`, or scope its resolution to routes already shown
hook-free (nix `print-dev-env --json`, devbox `shellenv --pure`, devenv
`build shell`, flox's derived env).

### 5. macOS service VM has no host/guest protocol — confirmed, decision

`add-macos-service-vm/specs/services/spec.md`: the guest provides "a
Linux Nix store and SSH access" and "SHALL NOT preinstall" flox, devenv
or devbox; the project's tooling "SHALL be materialized into the shared
store from the project's own definition"; `design.md` Decision 2: the
worktree is not mounted. Nothing says *who* runs the provider against
*what* to produce a Linux closure in the guest, or how the service plan,
shell, environment and process-compose artifacts get there. Unstarted
change; define the protocol before tasks.

### 6. Credential brokering is a SHALL with no design or tasks — confirmed, decision

`add-agent-workload/specs/credentials/spec.md` ("the value SHALL NOT be
present in the sandbox's environment, filesystem, or memory" for
network credentials — a placeholder-and-inject model). `tasks.md` has no
task containing "broker" or "placeholder"; the proxy forwards bytes
after authenticating (`src/proxy/server.rs`). Either design the broker
(source of secrets, per-upstream matching, placeholder format,
lifecycle, audit) with tasks, or drop the SHALL and keep direct delivery
with its exposure stated. CLAUDE.md already records `nono-proxy`'s
credential brokering as a *separate* adoption decision — this is where
it would be taken.

### 11. `add-devenv-services` still refuses devbox services — confirmed, decision

`add-devenv-services/specs/env-provider/spec.md` "devbox service
support is refused for a measured reason" vs `add-devbox-services`, which
captures and runs them, and `src/provider/devbox.rs` which implements
that. Both deltas are live; a sync would carry the refusal into main.
Supersede the older requirement explicitly or archive the change with
that delta struck.

### 12. `own-policy-baseline` requires nono-CLI behaviour — confirmed, decision

`own-policy-baseline/specs/policy/spec.md` scenario "Excluded groups are
named": "the emitted profile names the groups devcroft declines". That
is `groups.exclude`, a nono-CLI profile concept; `use-nono-library`
replaced the CLI with the library, `CapabilitySet` has no groups
(`src/policy/capability_set.rs`), and `doctor` reads
`nono::Sandbox::support_info()`. The baseline grants, signal mode and
render/projection congruence are still true and worth keeping; the
CLI-shaped scenarios are not.

### 15. `own-sandbox-environment` keeps the abandoned ambient-subtraction rule — confirmed, decision

`specs/sandbox-environment/spec.md`: a variable present in both the
ambient environment and the provider's output with the same value is
kept "unchanged from its own ambient environment". `src/lifecycle/up.rs`
uses `.env_clear()` and injects the resolved environment plus declared
forwards — nothing ambient survives on any value. Rewrite the
requirement in clean-environment terms, with a scenario that an ambient
variable *absent* from the provider does not survive regardless of
value.

### 17. `add-hardened-tier` still accepts `isolation = "hardened"` — confirmed, decision

`add-hardened-tier/specs/config/spec.md`: `"process"` or `"hardened"`;
`src/config/mod.rs` rejects `"hardened"` at parse, per
`remove-gvisor-backend`. CLAUDE.md lists `add-hardened-tier` as
implemented with its `SessionBackend` seam deliberately kept; the delta's
*config* requirement is what must not resurface. Archive or mark
superseded.

### 16. Manifestless `--with` selects no provider — confirmed, decision (unstarted)

`add-manifestless-mode/specs/env-provider/spec.md`: packages on the
command line "producing a usable environment for a repository containing
no provider configuration". Every provider needs project-owned files and
a lockfile; nothing says which provider materializes an ad-hoc set or
where its lock persists. Require `--provider` with `--with` and a
provider-owned ephemeral manifest/lock, or hold the feature until
pinning is defined.

## Confirmed — edits

### 8. Readiness promises a "started but not ready" report that has no state — confirmed, edit

`add-service-readiness/specs/services/spec.md`: "reports the service as
started but not ready" until probes pass. `src/services/mod.rs`
`ServiceHealth` is `Running | Failed | Exited | NotStarted | Pending |
…` — no ready/not-ready; `status` prints that enum. Dependency gating
(`process_healthy`) is implemented and tested; the *reporting* half is
not. Add a readiness field to the supervisor payload and `status`, or
narrow the requirement to gating.

### 9. Proxy authentication over-claims against same-UID processes — confirmed, edit

`add-egress-proxy/specs/network/spec.md`: "Every process on the host can
reach [loopback], including processes devcroft did not start", hence
the token. The token lives in `meta.json` under a 0700 state dir
(`src/lifecycle/state.rs`), which stops other *users*, not other
processes of the same user — and that is the adversary the sentence
names. A same-UID process can also read the SSH client key and the
project, so this is not a new exposure; it is an inaccurate claim.
`docs/threat-model.md` already scopes the boundary to accident
protection. Restate the requirement as cross-UID, and say the same-UID
case is out of scope.

### 10. Symlinked grant spellings are absent from the mount view — confirmed, edit

`fix-symlinked-grant-spelling/specs/policy/spec.md`: both spellings of
a grant are granted. `src/policy/capability_set.rs` does that for the
backend; `src/fleet/mount.rs` binds only the canonical target and
recreates only the merged-`/usr` symlinks (`setup_merged_usr_compat`).
A grant reached through any other symlink — `~/proj → /data/proj` — has
`/data/proj` in the view and no `~/proj`. Scope the requirement to the
policy layer and record the view as canonical-only, or reconstruct
alias chains in the view; either way, a Linux test with a non-standard
absolute symlink.

### 13. The view contains `/proc` and `/dev` beyond "what was granted" — partial, edit

`add-mount-isolation/specs/filesystem-view/spec.md`: the view "SHALL NOT
contain paths the manifest did not grant and the provider did not
resolve" — but the same requirement also admits "the system paths the
keeper itself requires". `/dev/null`, `/dev/urandom`, `/dev/tty` and the
private devpts are those. The host's *entire* `/proc` is not, in that
narrow sense; `policy::render`'s view note already names it as an
unbounded exposure. One sentence in the spec, distinguishing "present in
the namespace" from "granted by policy", closes the gap the reviewer
saw.

### 14. macOS unix-socket scoping names a proxy-UDS grant — partial, edit

The requirement (`specs/macos-unix-socket-mediation/spec.md`, "Where a
sandbox has an egress proxy…") already says: "An earlier draft of this
requirement mandated a scoped *unix-socket* grant; that was measured to
be the wrong mechanism for macOS… this one names the property that has
to hold." The reviewer read the requirement as still mandating the
mechanism; it does not. One *scenario* under it ("After the macOS spike
confirms it": "the scoped proxy-socket grant admits the sandbox's own")
still carries the old wording. Fix that sentence; the requirement stands.

### 7. Port allocation's "allocated port" — partial, edit

`add-port-allocation/specs/port-allocation/spec.md` already says
allocation "SHALL NOT allocate for a sandbox that has its own network
namespace" — the two models are not in conflict in the spec. What the
spec does not model is what `design.md` §P-NEW adds: a host-side relay
port for a namespaced sandbox, distinct from the service's own port.
That is a missing requirement (name it `host_forward_port`; say what
`status`, `policy --render` and the environment show), not a
contradiction.

## Secondary observations — all confirmed

- `add-test-runtime-fixture` defines three rows (nix, flox, devbox);
  `tests/common/mod.rs` `ROWS` matches; devenv is the fourth provider and
  has no row. CI's devenv leg selects the nix row for matrix tests as a
  result (`.github/workflows/ci.yml`).
- `add-nix-provider/design.md` orders the flake-metadata check before the
  lockfile check; the code checks the lockfile first.
- `add-not-applicable-status/design.md` says the status vocabulary "has
  four values"; `src/backend_capabilities.rs` `Status` has five.
- The devbox and devenv provider changes still list services as a
  non-goal; `add-devbox-services` and `add-devenv-services` implemented
  them.

## What this review could not see

It reads specs against code as of `02ba511`. It does not cover the two
findings this repo's own CI made the same day and that are recorded in
`docs/implementation-log.md` — `/tmp` read-only no longer expressible by
a manifest, and the mount view's devpts — because they are not spec
contradictions; they are measurements the specs have not caught up with
yet. `add-mount-isolation`'s filesystem-view spec and
`own-policy-baseline`'s `/tmp` reasoning are the two that should.
