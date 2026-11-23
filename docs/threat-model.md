# Threat model

What devcroft defends against, what it does not, and which use case each tier
actually backs. Every isolation claim in the README, in `devcroft`'s output, and
in the change specs should be traceable to a statement here.

## The three ingredients

Risk concentrates where three things meet:

- **private data** — source, credentials, tokens, config, company context;
- **untrusted content** — prompts, issues, pull requests, documents,
  repositories, web pages;
- **external communication** — the ability to push, upload, post, or call an
  API.

Any one alone is manageable. All three together mean a sentence in a prompt is
not a boundary. This framing is not ours — it is the standard one in the
current agent-security discussion, most recently in Docker's argument for
microVM-based agent sandboxes ([Šelajev, "Agents need real
sandboxes"](https://tessl.io/blog/agents-need-real-sandboxes), July 2026). We
adopt it because it maps cleanly onto what devcroft can and cannot enforce, and
we note the source's interest: it is a vendor case for a stronger boundary than
ours.

Mapping to what devcroft builds:

| Ingredient | Control | Where |
| --- | --- | --- |
| Private data | Path policy, sensitive-path denial, substituted home | MVP + `sandbox-provisioning` |
| Untrusted content | Provisioning sandbox — project code runs confined, before the toolchain exists | `sandbox-provisioning` |
| External communication | Domain-allowlisted egress through a resident proxy | `add-egress-proxy` |

The three in-flight changes are one control each. That is the shape of the
roadmap, not a coincidence.

## The boundary is not in the prompt

Instructions guide a model; they do not enforce anything. The demonstration
worth internalising: an agent refused to run a dangerous script, was asked to
restructure it into a module plus a boring entry point, still refused — and then
ran it fine once the context was cleared. The model did not change. The
guardrail lived in the conversation, and the conversation did not persist.
Filesystem access, network reach and credential exposure have to be enforced
somewhere that a context reset cannot move.

That is the whole reason this project exists, and it holds regardless of which
tier a user runs.

## Two use cases, and only one of them is backed

These are different products wearing the same interface, and conflating them is
the failure mode this document exists to prevent.

**A. Trusted code, many instances.** A developer's own repository, or one their
organisation controls, run as N parallel environments. The threat is accident
and interference: an agent deleting something outside its worktree, two agents
colliding on a port or a database, one runaway build starving the host.

**Backed.** The process tier — path policy plus namespaces plus resource limits
— is exactly the right instrument. This is the use case devcroft is for.

**B. Unreviewed code, many instances.** External pull requests, dependency
updates, repositories the agent itself fetched. The threat is deliberate: code
written to escape, exfiltrate, or persist.

**Not backed, and must not be claimed.** The process tier is accident
protection: the full host kernel syscall surface stays reachable, so a kernel
bug is an escape. The industry position — argued by parties with an interest in
selling the stronger boundary, but not thereby wrong — is that even a container
is insufficient here, which is why microVMs are the enterprise answer. devcroft
is below that bar and, with the hardened tier removed
(`remove-gvisor-backend`), has no path to it on this roadmap. The ceiling is
fixed at the process tier, and the named answer for anyone who needs more is a
VM — which is a supported path rather than a deflection, since the macOS path
already works that way. The criteria a future backend would have to meet before
that changes are recorded in that change's `design.md`.

**Consequence for the specs.** `sandbox-provisioning` is motivated by confining
activation of code nobody has read. That motivation is real and the change is
worth making — it closes an inversion where `up` runs project code on the host
*outside* any boundary, which is worse than the process tier, not equal to it.
But the change moves provisioning from "no boundary" to "process-tier boundary".
It does not deliver use case B. Any wording in that change implying otherwise is
overclaiming.

**Not part of the boundary.** The macOS exec redirect, which makes `/bin/sh`
and the other base names resolve inside a sandbox, is compatibility: a process
can drop it, and what it may then execute is decided by the policy alone.

**Where the per-backend detail lives.** This document states which use case each
tier backs. What any given backend can and cannot do — fleet, service ports,
resource limits, egress filtering, and the platform differences — is declared
data, not prose: `src/backend_capabilities.rs`, surfaced live by `devcroft
doctor` (`add-backend-capabilities`). The pointer above used to name a change
that did not exist yet; it now names real code. Prefer that matrix over any
caveat written here or in the README, and treat a discrepancy as a bug in the
prose — the two AF_UNIX bullets earlier in this document are the case in
point, corrected once already by the matrix having been built.

## Credentials: capability, not custody

The agent should be able to act with a credential without ever holding it. Real
secrets stay outside the sandbox and are attached to requests as they cross the
proxy; what is visible inside is a placeholder that is useless anywhere else.

This is the principle behind placing the egress proxy on the host rather than
inside the sandbox it filters (`add-egress-proxy`, E1). A proxy inside the
client's policy domain holds real credentials in memory reachable from that
domain, which defeats the placeholder scheme entirely. It is also the reason
the sensitive-path denial list is load-bearing rather than decorative: it is the
control for credentials devcroft never sees.

Convergent evidence: this is the same pattern Docker describes for its
sandboxes. Two independent designs arriving at sentinel-plus-proxy is a signal
the pattern is table stakes for agent sandboxing, not a differentiator.

### Delivered credentials: custody

Two routes hand the sandbox **the real value**, and they have the same
exposure:

- `[env] forward` in the project's `devcroft.toml`, which names a variable the
  project needs;
- `[credentials] env` in the user's own configuration
  (`~/.config/devcroft/config.toml`), which names a key the user's agent needs
  and reaches **every** sandbox that user brings up, whatever the project.

Either way the value is in the session's environment, and **every process in
the sandbox can read it**: the agent, and equally the project code the agent
runs — a test, a build script, a dependency's `postinstall`. A sandbox cannot
keep a credential from the code under edit when the agent that needs the
credential has to run in the same boundary as that code. Running devcroft in a
VM does not change this; the VM boundary is around both.

What devcroft does about it is narrowness and disclosure, not isolation:

- **Nothing arrives unnamed.** A sandbox inherits no host variable (the
  `own-sandbox-environment` finding in `docs/known-gaps.md`); a credential is
  there because a file names it.
- **Disclosed on every `up`**, one line per user credential, including against
  a sandbox that is already running, so it is never news afterwards that a
  freshly cloned repository's code could read your key.
- **Never written down.** Names are recorded; values are in no file devcroft
  writes (asserted over the whole state directory and the project's
  `.devcroft/`).
- **Egress bounds where it can go.** Under `network.default = "deny"` a key can
  leave only towards an allowed host. That is a real limit, and only as strong
  as the allowlist: a host that accepts uploads (a code host, a paste service)
  is also a way out.

What you can do: give the sandbox a key scoped to what the agent does — a
project-scoped or rate-limited key, never an organisation admin key — and
allow the agent's API host alone. For a key that travels in one header,
broker it instead (below).

### Brokered credentials: capability

`[[credentials.broker]]` in the user's configuration is the principle above,
built for key-shaped credentials. [agents.md](agents.md#brokering-the-key)
has each agent's configuration. The real value is read from the user's shell
at `up` and written to the proxy's stdin. It is never in the proxy's argv or
environment block, never in a file devcroft writes, and never logged. The
sandbox holds a placeholder, which is the session's proxy token, optionally
shaped, together with an endpoint on the proxy. The proxy puts the value into
the declared header on the way out, over TLS of its own to `https://<host>`,
verified against the trust store the environment names.

What that moves:

- **Code in the sandbox can no longer read the key.** A test, a build script
  or a dependency's `postinstall` that dumps the environment gets the
  placeholder, which is worthless outside this sandbox's proxy session.
- **It can still use the key, through the endpoint, while the sandbox is
  up.** Brokering removes custody, not capability. Anything in the sandbox
  can send requests to the host as the user, for as long as the sandbox
  runs. The bound is the host, which is the API the key was for, rather
  than every place the key could be copied to.
- **The host is reachable only through its route.** A direct `CONNECT` to it
  is refused, so a client that ignored the endpoint is refused rather than
  given the key.
- **A request elsewhere never gets the value.** The placeholder sent to
  another allowed host arrives as the placeholder (tested, absolute-URI and
  inside a `CONNECT` tunnel).

**The unencrypted hop is sandbox → proxy.** The agent speaks plain HTTP to
the endpoint, so the placeholder and the request body cross that hop
unencrypted. Nothing is intercepted: the agent negotiates TLS with no one,
and the proxy's own TLS to the upstream is ordinary client TLS. On Linux,
with the sandbox's own network namespace, the hop is the relay and the
proxy's unix socket. The socket's 0700 directory is the boundary, as it is
for SSH. On macOS, and on a Linux host without network namespaces, it is the
host's loopback, readable by anything on the host that can capture loopback
traffic. That exposes the placeholder and the conversation, never the key.

What brokering does not cover:

- **A key the client exchanges.** Cursor trades its key for a session token
  that comes back into the sandbox. It also cannot reach a loopback
  endpoint through the proxy at all, so it is not brokerable.
- **A file credential.** Subscription logins write their refresh back to the
  file, so they cannot be held outside. A broker entry has no way to name
  one.
- **`network.default = "allow"`**, where traffic need not pass the proxy.
  `up` refuses the combination rather than fall back to handing over the
  value.

## What a sandbox does not solve

A sandbox bounds local blast radius. It does not bound what the agent is
authorised to do through the tools it legitimately has.

- An allowlisted destination that accepts uploads is an outbound channel. Egress
  policy constrains destinations; it does not prevent exfiltration, and no
  wording in this project should say it does.
- If the agent can open pull requests, merge policy still matters. If it can
  reach a SaaS API, that product's permissions still matter. If it can reach
  production, the sandbox has not solved production governance.
- Application-level permissions, approval policies, logging and review sit
  beside the sandbox, not under it.
- **Unix sockets are within the threat model, not outside it — status is
  the matrix's job, not this document's.** Landlock mediates TCP, not
  AF_UNIX by itself; whether a given socket (pathname or abstract) is
  actually reachable from inside a sandbox today is exactly the kind of
  claim that drifted here in prose before `add-backend-capabilities`
  existed, and belongs in `src/backend_capabilities.rs` /
  `devcroft doctor` now (`pathname-unix-sockets`,
  `abstract-unix-sockets` entries) rather than restated in this bullet.
  What stays here, since a matrix cannot carry it: this class of
  reachability is inside what devcroft claims to bound, so a status
  change here is a security-relevant one, not a routine capability
  update — treat any change to either matrix entry as threat-model-review
  material, not a changelog line.
- **On macOS, that pathname-socket boundary is conditional on the
  manifest, and that is a threat-model fact rather than a capability
  one.** `add-macos-unix-socket-scoping` measured the mechanism live:
  Seatbelt treats a unix-socket `connect()` as network activity, so
  `network.default = "deny"` denies an ungranted socket there — but a
  sandbox that does *not* set it reaches every world-accessible socket on
  the host, including a nix daemon's. Linux does not have this
  conditionality: a mount view removes the path whatever the network mode
  is. So the same manifest yields a strictly weaker boundary on macOS,
  and the difference is invisible in the manifest itself. Anyone reasoning
  about a macOS deployment has to read the network section to know whether
  this boundary exists at all; `devcroft doctor` names both halves of the
  degradation, and `up` says which case a given sandbox is in.

The value is that the boundary moves out of the prompt and into infrastructure.
That is a real gain and a bounded one.

## Usability is a security property

A sandbox people bypass protects nothing. Two concrete consequences for this
project:

- **An empty environment is a bypass generator.** This is where the declarative
  manifest earns its place: the environment is complete from the lock, rather
  than being an image someone has to maintain or a machine someone has to
  reshape. Container-side tooling is converging on the same need through
  declarative sandbox configuration layered on a base image; the difference that
  remains is bit-reproducibility and marginal cost per instance, not the idea.
- **Ceremony is a bypass generator.** The agent should run inside the boundary
  through the tools people already use. devcroft's answer is an SSH endpoint per
  sandbox, which works with any SSH-capable editor without a per-IDE
  integration — a genuine advantage over integration-per-editor approaches, and
  currently undersold.
