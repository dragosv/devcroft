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

**Mechanism, confirmed in the library.** `install_seccomp_proxy_filter` installs
a seccomp-notify filter for proxy-only network mode, with
`prepare_seccomp_proxy_filter` building the program in the parent before clone.
The notification loop — `recv_notif`, `read_notif_sockaddr`, `deny_notif`,
`continue_notif`, `respond_notif_errno`, `inject_fd`, and `notif_id_valid` for
TOCTOU safety — is exposed, along with the syscall constants for connect, bind,
sendto, sendmsg and sendmmsg. So enforcement is kernel-mediated and the decision
runs in the supervisor.

This also explains why domain filtering never worked under the CLI's wrap mode:
the mechanism requires a resident process to answer notifications, and wrap has
none. The gap was architectural, not a missing feature.

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

## Open Questions

1. **RESOLVED — the library provides mechanism and policy; devcroft provides the
   loop.** `HostFilter` decides, `install_seccomp_proxy_filter` enforces, the
   notification API mediates. What remains is the resident supervisor loop and
   the transport that carries an allowed connection out. Substantially smaller
   than a from-scratch proxy. Confirm `install_seccomp_proxy_filter`'s signature
   — specifically what policy it takes and how a permitted connection is
   completed — before designing the transport.
2. **Protocol coverage.** Because interception is at the socket layer rather
   than the HTTP layer, this may be less of a problem than it first appeared:
   non-proxy-aware clients are mediated too. Determine what the notify path
   actually does with an allowed connection — inject a connected descriptor, or
   let the original syscall continue — since that decides whether HTTP-level
   proxying is needed at all.
3. **Client configuration.** May be moot for the same reason. Package managers
   that ignore proxy environment variables are still mediated at the socket
   layer. Verify before building environment-variable plumbing that turns out to
   be unnecessary.
4. **Name-based decisions.** Socket-layer interception sees addresses, not
   names. How the requested name reaches the decision — DNS interception,
   connection-time correlation, or SNI — is the real open question here, and it
   is what E4's stated limits depend on.
5. **Fleet interaction.** One proxy instance per client, or one instance with
   per-listener policy? The second is cheaper; the first is simpler to reason
   about. Decide before fleet work rather than during it.
