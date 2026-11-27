## ADDED Requirements

### Requirement: Editor setup and diagnostics

The host CLI SHALL list `agent acp` in help, print the qualified editor's configuration with `agent acp --print-config [<editor>]` and offer `doctor --probe-acp` on a running sandbox. The probe SHALL verify a clean initialize response without installing a package, starting a keeper or acquiring a credential, and SHALL say that a clean `initialize` proves the transport, not authentication or model access.

#### Scenario: Print-config emits GUI-safe absolute executable path
- **WHEN** `devcroft agent acp --print-config` runs
- **THEN** it prints a configuration snippet referencing the absolute path of `devcroft`

#### Scenario: Probe on a stopped sandbox
- **WHEN** the user asks doctor to probe an editor connection to a stopped sandbox
- **THEN** doctor reports the preparation needed and leaves the sandbox stopped
