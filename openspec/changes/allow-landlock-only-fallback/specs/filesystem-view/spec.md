## MODIFIED Requirements

### Requirement: A view that cannot be constructed prevents startup

If the filesystem view cannot be constructed, `up` SHALL fail. It SHALL
NOT fall back to the host's namespace — except on an explicit operator
opt-in, given on the `up` command line and never in the manifest, and
only for a sandbox whose rendered policy makes no claim the fallback
cannot keep.

This is deliberately stricter than how network isolation degrades. A
sandbox without port isolation loses a convenience and nothing it was told
becomes false. A sandbox without its view is one whose rendered policy
describes a boundary that is not there — and the operator learns this from
a warning they cannot act on without restarting anyway. The opt-in exists
because the same host limitation is a stock Ubuntu 24.04 default, and a
sandbox that never claimed unix-socket mediation — `network.default =
"allow"` — has the exact boundary every Linux sandbox had before the
view existed: real, and declared as such. A sandbox that did claim it —
`network.default = "deny"`, hence an egress proxy — SHALL be refused
regardless of the opt-in, because Landlock alone does not mediate
AF_UNIX and the policy would be false.

#### Scenario: The host cannot create the namespace

- **WHEN** `up` runs on a host that cannot create an unprivileged mount
  namespace, without `--allow-degraded-isolation`
- **THEN** `up` fails, naming the capability and the host limitation
- **AND** the failure names both remedies: the host setting that would
  restore the namespace, and the opt-in flag
- **AND** no sandbox starts with a view weaker than the one compiled

#### Scenario: The operator opts in, and the policy permits it

- **WHEN** `up --allow-degraded-isolation` runs on such a host for a
  sandbox with `network.default = "allow"`
- **THEN** the sandbox starts confined by the process backend alone,
  with no filesystem view
- **AND** `up` prints exactly one warning naming the aspect, the reason,
  the residual exposure (ungranted unix sockets, the nix daemon's
  included, remain reachable; nothing narrows what the sandbox can see),
  and the host remedy
- **AND** the fallback is recorded with the sandbox so `status` reports
  it for as long as the sandbox exists

#### Scenario: The operator opts in, and the policy forbids it

- **WHEN** `up --allow-degraded-isolation` runs on such a host for a
  sandbox with `network.default = "deny"`
- **THEN** `up` fails, naming the claim the fallback cannot keep
- **AND** the failure names the two ways out: the host setting, or a
  manifest that does not make the claim
- **AND** the flag is not treated as overriding the claim

#### Scenario: Diagnosis before the attempt

- **WHEN** the operator runs `doctor` on such a host
- **THEN** it reports mount isolation as unavailable
- **AND** it does so alongside the network-namespace report, since both
  rest on the same unprivileged user namespace
- **AND** it names the opt-in and what it costs
