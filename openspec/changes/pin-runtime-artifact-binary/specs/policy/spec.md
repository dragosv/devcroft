## ADDED Requirements

### Requirement: Runtime object grant carries runtime generation origin
The read-only grant for `~/.local/share/devcroft-artifacts/runtime/<id>` (and any minimal host loader/libc paths required by that binary) SHALL carry origin `runtime:gen-<N>` in `policy --render`, and any manifest write grant overlapping `~/.local/share/devcroft-artifacts/runtime` SHALL fail at layer `config`.

#### Scenario: Policy render attributes the immutable runtime object
- **WHEN** `devcroft policy --render` runs on a sandbox
- **THEN** the runtime store object path appears read-only with origin `runtime:gen-<N>`
