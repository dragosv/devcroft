# filesystem-view Delta Specification (add-mount-isolation)

## Purpose

Give each sandbox its own view of the filesystem, so that what it cannot
use is also what it cannot see or name.

Landlock answers "may this process read this path". It does not answer
"does this path exist here", and there are reaches it cannot mediate at
all — AF_UNIX connect being the measured one. A view closes both: an
absent path is unreadable, unnameable, and unconnectable, without a rule
for each case.

## ADDED Requirements

### Requirement: A sandbox cannot reach a unix socket outside its view

A sandbox SHALL NOT be able to `connect()` to a unix socket that its
filesystem view does not contain, regardless of that socket's filesystem
permissions.

This is the gap the change exists to close. Landlock's network rules cover
TCP only, so a world-accessible unix socket — a package-manager daemon, a
container runtime, a session bus — is reachable from inside a sandbox
whose compiled policy grants none of it. Denying it by rule is not
possible; no Landlock ABI expresses AF_UNIX. Removing it from the view is.

#### Scenario: A package-manager daemon socket

- **WHEN** a sandbox runs on a host whose package-manager daemon exposes a
  world-accessible unix socket, and the manifest grants no path to it
- **THEN** the sandbox cannot connect to it
- **AND** the failure is that the path does not resolve, not that a rule
  refused it

#### Scenario: A socket the sandbox was never told about

- **WHEN** a sandbox attempts to connect to any unix socket outside its
  view, whether or not devcroft knows that socket exists
- **THEN** the connection fails
- **AND** this holds without devcroft enumerating sockets to deny, since
  the view is an allowlist of what is present rather than a denylist of
  what is not

### Requirement: The view contains what the sandbox was granted, and the toolchain it was resolved

A sandbox's view SHALL contain its project root, the provider's resolved
runtime paths, and the system paths the keeper itself requires. It SHALL
NOT contain paths the manifest did not grant and the provider did not
resolve.

A view narrower than the grants would break the sandbox in ways that
surface as a failed compile rather than as a policy error. A view wider
than them would reintroduce exactly what this change removes.

#### Scenario: The resolved toolchain is usable

- **WHEN** a sandbox built from a closure-tier provider compiles or runs
  tests
- **THEN** every path the provider resolved is present and readable
- **AND** the store's non-package areas — a daemon socket, a mutable
  database — are not present, since resolution never named them

#### Scenario: An ungranted host path

- **WHEN** a sandbox looks for a host path the manifest did not grant
- **THEN** that path is absent from its view

### Requirement: The sandbox's own egress path stays reachable

Where a sandbox has an egress proxy, its view SHALL contain the socket
that proxy listens on.

Stated as a requirement rather than left to the implementation because the
obvious hardening — masking devcroft's state directory, which the baseline
already denies for filesystem access — would silently remove it. The
control and SSH sockets in that same directory are inherited as file
descriptors and never resolved by path again; the proxy socket is dialled
once per outbound connection and is therefore the one that breaks. The
symptom would be a sandbox that starts, reports healthy, and has no
network.

#### Scenario: An isolated sandbox with an allowlist

- **WHEN** a sandbox has both its own filesystem view and a declared
  `network.allow`
- **THEN** it reaches its allowlisted hosts
- **AND** the view's construction is what makes that possible, not an
  exception to it

#### Scenario: Another sandbox's proxy socket

- **WHEN** a sandbox's view is constructed
- **THEN** it contains its own proxy socket and no other sandbox's

### Requirement: A view that cannot be constructed prevents startup

If the filesystem view cannot be constructed, `up` SHALL fail. It SHALL
NOT fall back to the host's namespace.

This is deliberately stricter than how network isolation degrades. A
sandbox without port isolation loses a convenience and nothing it was told
becomes false. A sandbox without its view is one whose rendered policy
describes a boundary that is not there — and the operator learns this from
a warning they cannot act on without restarting anyway.

#### Scenario: The host cannot create the namespace

- **WHEN** `up` runs on a host that cannot create an unprivileged mount
  namespace
- **THEN** `up` fails, naming the capability and the host limitation
- **AND** no sandbox starts with a view weaker than the one compiled

#### Scenario: Diagnosis before the attempt

- **WHEN** the operator runs `doctor` on such a host
- **THEN** it reports mount isolation as unavailable
- **AND** it does so alongside the network-namespace report, since both
  rest on the same unprivileged user namespace

### Requirement: The view is inspectable

`policy --render` SHALL show what a sandbox's view contains, with the same
origin attribution every other compiled rule carries.

devcroft's standing invariant is that nothing reaches the backend which
`--render` cannot show. A view is a stronger constraint than a rule — it
decides what exists, not merely what is permitted — so leaving it
unrendered would put the most consequential part of the policy outside the
one command that exists to explain it.

#### Scenario: Rendering a sandbox with a view

- **WHEN** the operator renders the policy of a sandbox with mount
  isolation
- **THEN** the view's contents appear, each with its origin
- **AND** a path present because the provider resolved it is
  distinguishable from one present because the manifest granted it
