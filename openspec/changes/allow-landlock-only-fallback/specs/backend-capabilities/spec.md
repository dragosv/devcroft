## MODIFIED Requirements

### Requirement: Declared capabilities are reported against the actual host

`doctor` SHALL report the declared capabilities alongside what this host can
provide, so that "devcroft supports this" and "this machine can do this" are
distinguishable without inference. Where a capability's enforcement on
Linux rests on the filesystem view, its report on a host without
unprivileged user namespaces SHALL name that the view is unavailable,
that the opt-in exists, and what a sandbox started under it does not get.

#### Scenario: A host lacks something devcroft enforces elsewhere

- **WHEN** a capability is `enforced` in the declaration but unavailable on this
  host
- **THEN** `doctor` reports both facts and the difference between them
- **AND** the user is not left to deduce whether the gap is devcroft's or their
  machine's

#### Scenario: An unadopted capability

- **WHEN** a capability is `not-adopted`
- **THEN** `doctor` does not probe the host for it
- **AND** it is not reported as a host deficiency, since the host is not the
  reason it is absent

#### Scenario: Pathname unix sockets on a host without user namespaces

- **WHEN** `doctor` runs on Linux and the mount-namespace probe fails
- **THEN** `pathname-unix-sockets` is reported as degraded on this host,
  naming that ungranted unix sockets are reachable from a sandbox
  started with `--allow-degraded-isolation`
- **AND** the same entry reports `enforced` on a host where the probe
  succeeds
