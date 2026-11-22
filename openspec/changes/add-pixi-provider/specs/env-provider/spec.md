## ADDED Requirements

### Requirement: Strict Pixi binary materialization
The Pixi provider SHALL require an existing, current lock in a format version
it covers, with a solution for the selected environment and platform, and
SHALL check the lock's recorded system requirements against the host. It SHALL
install with strict lock checking, SHALL leave manifest and lock bytes
unchanged, and SHALL refuse before any installer runs: source distributions,
editable, path and VCS dependencies, and build backends. Failures SHALL be at
layer `provider`, exit code 3, naming the package and the reason.

#### Scenario: Clean environment restore
- **WHEN** a supported locked project starts with an empty artifact store
- **THEN** the selected environment is materialized from the exact locked packages
- **AND** neither manifest nor lock changes

#### Scenario: Stale or wrong-platform lock
- **WHEN** the lock is missing, stale or lacks the selected platform or environment
- **THEN** `up` fails with a corrective hint and no relock or substitute environment

#### Scenario: Default pyproject layout
- **WHEN** a `pyproject.toml` workspace installs the project itself as an editable dependency
- **THEN** it is recognized as Pixi and refused, naming the dependency and that installing it would run its build backend

#### Scenario: Unknown lock version
- **WHEN** the lock declares a format version the adapter has no fixtures for
- **THEN** `up` fails naming the version instead of guessing at its fields

### Requirement: Host-side Pixi runs isolated and executes nothing it downloaded
Host-side pixi SHALL run with an environment constructed from an explicit
allowlist, an isolated home, and no user or system configuration. The
project's own Pixi configuration, which pixi always merges, SHALL be parsed
and any key outside the admitted set refused by dotted name. An environment
shape for which locked installation executes downloaded code on the host, as
measured for the qualified version, SHALL be refused by name; for the
qualified version that is every environment with a PyPI dependency. Every
package URL in the selected environment's lock SHALL be on a qualified public
host (`conda.anaconda.org`, `repo.prefix.dev`), checked before any pixi
subprocess, so that no stored credential of the caller can be used. `policy
--render` SHALL state what resolution runs host-side.

#### Scenario: User configuration does not change the result
- **WHEN** two callers differ in user-level Pixi configuration, mirrors and stored credentials
- **THEN** the same project inputs produce the same object and environment

#### Scenario: Project enables installation code
- **WHEN** the project's Pixi configuration opts in to post-link scripts or disables TLS verification
- **THEN** `up` fails at layer `provider`, exit code 3, naming the key, before installation

#### Scenario: Installation would run the environment's interpreter
- **WHEN** the selected environment's lock contains a `pypi:` package
- **THEN** `up` fails at layer `provider`, exit code 3, naming the package and that installing it runs the environment's interpreter on the host
- **AND** Conda-only environments remain supported

#### Scenario: Channel outside the qualified hosts
- **WHEN** a locked package URL names a host other than `conda.anaconda.org` or `repo.prefix.dev`
- **THEN** `up` fails before any pixi subprocess, naming the host and stating that authenticated and private channels are not supported
- **AND** no stored credential of the caller, including one in the macOS keychain, is consulted

### Requirement: Pixi activation is captured inertly and executed confined
The system SHALL obtain activation as non-executed text after installation
and SHALL NOT use the JSON form or evaluate activation on the host. The order
of package, workspace, feature and target activation SHALL be the order pixi
emits. Before it runs, every project path in that text SHALL name the real
project root and never the installation's staging directory. The text SHALL
run once in confined provisioning, with the runtime filesystem and only the
provisioning network policy, and a failure SHALL prevent keeper startup.

#### Scenario: Activation sentinels
- **WHEN** workspace and package activation scripts write markers and export variables
- **THEN** host-side capture leaves the markers untouched
- **AND** confined provisioning runs them in pixi's order and sessions inherit the result

#### Scenario: Project paths
- **WHEN** a project activation script reads a relative file or `PIXI_PROJECT_ROOT`
- **THEN** it sees the real workspace

#### Scenario: Feature and target scripts
- **WHEN** the selected environment composes activation from several features and a target
- **THEN** the scripts run in the order pixi itself would run them

#### Scenario: Runtime cannot inherit provisioning access
- **WHEN** activation uses an endpoint allowed only by the provisioning network policy
- **THEN** activation reaches it and later sessions do not

#### Scenario: Emission is not inert at a candidate version
- **WHEN** qualification finds the text form executing an activation sentinel
- **THEN** that version is not admitted and no host evaluation is used instead

### Requirement: Pixi objects are immutable, shared by recipe and reclaimed
The provider SHALL materialize each environment at its final path under
`~/.local/share/devcroft-artifacts/pixi/`, under an exclusive per-object lock,
removing any directory that lacks a completion record before installing and
writing that record atomically only after verification. Object identity SHALL
cover the manifest, lock, admitted configuration, environment, target, pixi
version and adapter version. A sandbox's recorded state SHALL be its reference
to an object; `rm` and a recreation that changes the object SHALL remove every
object no sandbox references, and SHALL NOT remove one a concurrent `up` is
acquiring. `down` SHALL remove nothing. The user's own `.pixi` directory SHALL
NOT be read, written or removed.

#### Scenario: Two sandboxes of one checkout
- **WHEN** two sandboxes of the same project resolve the same recipe
- **THEN** both execute one object and neither can modify it

#### Scenario: Interrupted installation
- **WHEN** installation dies before the completion record is written
- **THEN** a later `up` removes that directory under the object lock and installs again

#### Scenario: Last reference removed
- **WHEN** one of two sandboxes sharing an object is removed, then the other
- **THEN** the object survives the first `rm` and is removed by the second

### Requirement: Pixi staleness is complete
The system SHALL fingerprint the selected manifest, the lock, the environment
name, the project's Pixi configuration, each project activation script and
the adapter version, treating an absent referenced file as a distinct input
rather than an error. A changed or missing input SHALL mark the environment
stale without altering live sessions.

#### Scenario: Activation script changes without a lock edit
- **WHEN** a project activation script changes or disappears after `up`
- **THEN** `status` reports `env: stale`, naming the Pixi manifest, and not `unknown`

### Requirement: Pixi tasks do not imply managed services
The provider SHALL report that it has no service concept and SHALL NOT infer
services from task names, task graphs or command syntax. Resolution SHALL NOT
execute Pixi tasks. A `[network.services.<name>]` entry under
`env.provider = "pixi"` SHALL fail at layer `config`, exit code 2. The
existing cross-provider refusal SHALL apply unchanged.

#### Scenario: Task named serve
- **WHEN** a Pixi manifest has a task called `serve` that starts a long-lived command
- **THEN** resolution neither executes it nor registers it as a service

#### Scenario: Service port under Pixi
- **WHEN** `devcroft.toml` declares `[network.services.web]` under `env.provider = "pixi"`
- **THEN** `up` fails at layer `config`, exit code 2

#### Scenario: Flox services beside a Pixi selection
- **WHEN** the project also carries a flox manifest that declares services
- **THEN** `up` fails at layer `provider`, exit code 3, naming them, unless `--skip-hooks` is passed
