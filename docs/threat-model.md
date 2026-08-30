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

**Where the per-backend detail lives.** This document states which use case each
tier backs. What any given backend can and cannot do — fleet, service ports,
resource limits, egress filtering, and the platform differences — is declared
data, not prose: see `add-backend-capabilities`. Prefer that matrix over any
caveat written here or in the README, and treat a discrepancy as a bug in the
prose.

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
- **Unix sockets are outside the policy entirely.** Landlock mediates TCP,
  not AF_UNIX, so a sandboxed process reaches any unix socket the
  filesystem permissions allow — including ones in ungranted directories.
  On a host with a nix daemon that means the sandbox holds whatever
  authority that daemon extends to a local user; on a host with a Docker
  socket it would mean far more. Measured in
  `tests/unix_socket_not_mediated.rs`; see `docs/known-gaps.md`. This is a
  property of the mechanism, not an oversight in the policy, and closing
  it needs seccomp rather than a Landlock rule.

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
