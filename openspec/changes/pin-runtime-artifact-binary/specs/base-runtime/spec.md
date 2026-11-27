## ADDED Requirements

### Requirement: Sandbox runtime links resolve to an immutable artifact object
`up` SHALL materialize the embedded runtime binary (and any companion runtime library) into a read-only content-addressed object under `~/.local/share/devcroft-artifacts/runtime/<id>/` keyed by generation, target and binary SHA-256, record `runtime_object_id` in `Meta` under a store lease, and point `<project>/.devcroft/<name>/runtime/*` at that immutable object rather than the mutable host `devcroft` executable path. Sweeps during `up` and `rm` SHALL remove a runtime object only when no recorded sandbox `Meta` references it.

#### Scenario: Replacing the host devcroft binary does not mutate a running sandbox's runtime
- **WHEN** a sandbox is brought up, the host `devcroft` executable path it was started from is atomically replaced on disk by another binary, and a session inside the sandbox runs `sh` and `ls`
- **THEN** both resolve to and execute the immutable runtime object recorded at `up`, not the newly written host file

#### Scenario: Two sandboxes share one runtime object until the last is removed
- **WHEN** two sandboxes are brought up by the same `devcroft` binary and the first sandbox is removed with `devcroft rm`
- **THEN** both share one runtime object, the first `rm` keeps it, and removing the second sandbox sweeps it
