# Design — Egress Proxy

## E1 — The proxy is resident and owned by the supervisor

**Decision.** A long-lived proxy task in the supervisor process, on the host,
outside whatever sandbox the client runs in.

**Rationale.** Domain filtering needs a resident component by construction: the
decision happens at connection time, against a name, and there is nobody to make
it if the enforcement is a one-shot policy applied at exec. This is the specific
reason filtering never worked under the CLI's wrap mode, and re-creating that
shape inside the library would reproduce the same failure.

Host placement, rather than inside the client's sandbox, gives two properties
that are otherwise separate work:

- **Attribution.** Which client called is known from the listener the connection
  arrived on. No in-band identification.
- **Credential isolation.** A proxy inside the client's sandbox holds real
  credentials in the same policy domain as the code it is filtering, which is
  reachable by `ptrace` or `/proc/<pid>/mem` from within that domain. Outside,
  it is not reachable at all. This is what makes the capability-not-custody
  principle enforceable rather than aspirational — see
  [docs/threat-model.md](../../../docs/threat-model.md). The same
  sentinel-plus-proxy pattern is what container-side agent sandboxes are
  converging on, which makes it table stakes rather than a differentiator.

## E2 — The kernel layer stays deny-by-default; the proxy is the only exit

**Decision.** Direct egress is denied at the kernel level. The client's only
route out is the proxy endpoint.

**Rationale.** A cooperative proxy that the client can decline to use is not a
policy. Bypassing it must be structurally impossible, not a matter of the client
choosing to behave.

**Mechanism, confirmed in the library and corrected from the original draft of
this section.** `CapabilitySet::proxy_only_with_bind(port, bind_ports)` sets
`NetworkMode::ProxyOnly`, which `apply_auto` compiles to a plain Landlock
`NetPort` rule on any kernel with ABI V4+ (measured live in this devcontainer:
ABI V6) — the same port-based primitive `network.ports` already uses, aimed at
one more port. The lower-level seccomp-notify machinery this section
originally named (`install_seccomp_proxy_filter`, `recv_notif`,
`read_notif_sockaddr`, `inject_fd`, `continue_notif`, `notif_id_valid`) is real
but is `apply_auto`'s own fallback for pre-V4 kernels; devcroft calls the
builder method and never touches the notification loop directly. See design.md
Open Questions for the full trail, including why `inject_fd` could not have
completed a `connect()` even if devcroft did reach for it.

This still explains why domain filtering never worked under the CLI's wrap
mode: whichever layer enforces the kernel gate, *something* still has to
terminate the connection and decide by hostname, and `wrap` has no resident
process to be that something. The gap was architectural, not a missing
feature — just smaller to close than the original mechanism paragraph implied.

## E3 — Network policy is per-context

**Decision.** Provisioning and runtime carry separate network policies, in the
same way they carry separate path policies.

**Rationale.** Their needs differ in kind. Provisioning needs the package
registries and should have nothing else; runtime often needs neither, or needs a
different set. Deriving one from the other means granting one of them something
it should not have.

## E4 — Filtering is by name, and the name is the unit of policy

**Decision.** Allowlist entries are domains. The decision is made on the name
the client asked for.

**Rationale.** Port-level or address-level filtering does not express what
anybody wants to say. A name-based decision is also what makes the policy
readable and reviewable in the manifest.

**Known limits, to be stated rather than papered over:** an allowlisted name
resolves to addresses that may host other services; the resolved-IP scope may be
wider than intended. This is a real gap and it is the reason the non-goal above
says "constrained", not "prevented".

## E5 — Refusals must be legible

**Decision.** A refused connection reports the destination and the rule that
decided it, both to the operator and, where the protocol allows, to the client.

**Rationale.** This is the same risk that shows up everywhere else in this
project: a policy failure that surfaces as a package manager's generic network
error is worse than no policy, because nobody can act on it. A developer whose
`npm ci` fails needs to see which host was refused, not a timeout.

## Open Questions — RESOLVED against the pinned library (0.74.0) and a live probe

Task 0 asked whether `install_seccomp_proxy_filter`'s signature and completion
mechanism would decide the shape of section 1. It did, and the shape is smaller
and different from what the proposal assumed.

**The mechanism, precisely.** `install_seccomp_proxy_filter(has_bind_ports:
bool)` takes no policy at all — it is a pure syscall trap, and it is *only*
installed as a fallback for kernels whose Landlock ABI lacks `AccessNet`
(< V4). This devcontainer measured **Landlock ABI V6** live
(`nono::Sandbox::detect_abi()`), so on this host and any modern kernel the
seccomp-notify path is dead code: `apply_auto` takes the Landlock branch
instead. `CapabilitySet` already exposes exactly the shape this change needs
as a builder method — `.proxy_only_with_bind(port, bind_ports)` — which sets
`NetworkMode::ProxyOnly { port, bind_ports }`. On Linux this compiles to a
plain Landlock `NetPort` rule (`ConnectTcp` for `port`, `BindTcp` for each of
`bind_ports`); on macOS, to `(allow network-outbound (remote tcp
"localhost:PORT"))`. Both are ordinary port-based kernel rules — the same
primitive `network.ports` already uses today, just aimed at one more port.

**Consequence for section 1.** The "resident notification loop / TOCTOU-safe
responses / one filter instance per client" tasks describe machinery
`apply_auto` already owns internally on the ABI/platform where it is actually
needed. Devcroft never touches `recv_notif`/`read_notif_sockaddr`/
`inject_fd`/`continue_notif` directly — those are what `nono-cli`'s own
`SeccompPolicy::proxy_fallback` used them for (confirmed from
`CapabilitySet::localhost_ports`'s doc comment), and that component lived in
the CLI binary devcroft deliberately stopped depending on
(`use-nono-library`). Devcroft's job is one level up, exactly as the proposal's
"What devcroft supplies" section already said, but now concretely: compile
`network.allow` to `NetworkMode::ProxyOnly`, and run the actual host-side proxy
process that terminates connections at `127.0.0.1:<port>` and makes the
per-hostname decision. Section 1 is retitled below from "supervisor loop" to
"proxy process" to match.

**1. RESOLVED.** As above.

**2. RESOLVED — not moot: an HTTP-level proxy is required.** `ProxyOnly`'s
kernel gate permits exactly one thing: a literal `connect()` to
`127.0.0.1:<port>`. It does not rewrite or redirect a `connect()` aimed
anywhere else — that call is simply denied. There is no "let it through
already-filtered" path: `inject_fd`'s `SECCOMP_ADDFD_FLAG_SEND` makes the
injected descriptor *become the syscall's return value*, which is the right
shape for `openat()` (a real Linux use, confirmed at `read_notif_path`'s call
site) but not for `connect()`, whose successful return is `0`, not an fd
number — nono does not expose the `pidfd_getfd`-plus-zero-response pattern
that would be needed to complete an in-flight `connect()` to an
kernel-rewritten destination. So the destination-terminating proxy the
proposal already sized ("What devcroft supplies... the transport that carries
an allowed connection out") is not optional plumbing; it is the only place a
per-hostname decision can take effect at all.

**3. RESOLVED — not moot either: `HTTP_PROXY`/`HTTPS_PROXY` must be set.**
Because the kernel only ever permits `connect()` to the literal proxy address,
a client that ignores proxy environment variables and dials a destination
directly gets `EPERM`/denied at the kernel layer — full stop, not silently
mediated. This is fail-closed (consistent with this project's posture) but it
means client environment plumbing is required, not speculative; task 3 keeps
its shape, just with the "may be unnecessary" hedge removed.

**4. RESOLVED — it's the HTTP protocol itself, not DNS or SNI tricks.** The
proxy terminates ordinary HTTP: a `CONNECT host:443` request line (for
HTTPS/TLS, tunneled byte-for-byte afterward — no TLS interception, per the
non-goals) or an absolute-URI request (for plain HTTP) both carry the
hostname in the request itself. `net_filter.rs`'s own module doc says as much
— `HostFilter` is described as deciding "CONNECT requests" — confirming this
is what the library's author already had in mind, not a devcroft invention.

**5. Still open, deferred to fleet.** `add-linux-agent-fleet`'s design.md D4
already commits to one proxy instance per agent. For pre-fleet devcroft
there is exactly one "agent" per sandbox, so one proxy per `up` is the
non-fleet instance of the same decision, not a competing one — recorded here
so fleet work confirms rather than re-decides it.
