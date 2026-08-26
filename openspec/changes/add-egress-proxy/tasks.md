# Tasks — Egress Proxy

## 0. Confirm the interface

- [ ] Read `install_seccomp_proxy_filter`'s signature: what policy it accepts,
      and how an allowed connection is completed (descriptor injection versus
      continuing the original syscall). **This decides the shape of section 1.**
- [ ] Determine how a requested hostname reaches the decision point, given that
      socket-layer interception sees addresses (design.md Q4).
- [ ] Check whether `SeccompNetFallback` and the existing network-block path
      already cover part of this.

## 1. Supervisor loop

- [ ] Resident notification loop in the supervisor: receive, read the
      destination, decide via `HostFilter`, respond.
- [ ] TOCTOU-safe responses — validate the notification is still pending before
      acting on it.
- [ ] Transport for allowed connections, in whatever form the interface requires.
- [ ] One listener or filter instance per client sandbox, so attribution comes
      from the source rather than from the client.
- [ ] Structured refusal records: destination, deciding rule, originating
      sandbox.

## 2. Policy integration

- [ ] Compile `network.allow` to a deny-by-default kernel policy with the proxy
      endpoint as the only permitted path — not to a blanket block.
- [ ] Per-context network policy in the manifest (provisioning, runtime).
- [ ] `policy --render` and `why` cover both contexts' allowlists with origin
      attribution.
- [ ] Fail closed when the proxy is unavailable; never fall back to unfiltered.

## 3. Client reachability

- [ ] Establish whether proxy environment variables are needed at all. Because
      mediation is at the socket layer, clients that ignore proxy settings are
      still covered — verify this before building plumbing that may be
      unnecessary.
- [ ] If they are needed, set them in the sandbox environment.
- [ ] Measure against real package managers rather than assuming.

## 4. Diagnostics

- [ ] Surface refusals so a developer can see which host was refused.
- [ ] `doctor` reports whether domain filtering is enforceable on this host.
- [ ] Replace any documentation claiming domain filtering works today, and any
      claiming exfiltration is prevented.

## 5. Validation

- [ ] A real `npm ci` / `go mod download` succeeds with the registries
      allowlisted and everything else refused.
- [ ] A direct socket to an unrelated address is refused at the kernel level,
      not merely unproxied.
- [ ] Two sandboxes with different allowlists: neither inherits the other's.
- [ ] Refusal message names the host, verified by reading it as a developer
      would.

## 6. Downstream

- [ ] `sandbox-provisioning`: replace the deferred on/off network decision with
      a real provisioning allowlist.
- [ ] `add-linux-agent-fleet`: its `agent-networking` requirements assume this
      proxy exists; confirm they are satisfied rather than restated.
