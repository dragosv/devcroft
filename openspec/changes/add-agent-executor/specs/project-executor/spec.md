## ADDED Requirements

### Requirement: Record a real project execution and credential separation result

The experiment SHALL use one fixture and one qualified real agent to determine whether an agent-requested command uses the project’s frozen environment and whether project processes can read the agent’s synthetic credential or use its route. The record SHALL name the platform, agent version, authentication mode, observations, limits and decision about a follow-up.

#### Scenario: Agent requests a project test
- **WHEN** the real harness edits a file and requests a test through its configured tool
- **THEN** the record shows which process tree and toolchain executed the test, the observed exit result and which settings could change routing

#### Scenario: Separate credential custody
- **WHEN** the experiment runs a project command alongside an agent holding a synthetic credential, including attempts to forward that variable in `[env] forward` or cover the agent `HOME` / socket root in `filesystem.read`
- **THEN** colliding `[env] forward` and overlapping canonical `Read`/`ReadWrite` grants are refused, and the record states whether the project could obtain the value or its credential route

#### Scenario: Qualification is incomplete or platform-gated
- **WHEN** any of these has not been demonstrated:
  - routing, credential separation or cleanup;
  - `network.default = "deny"` on the final topology;
  - cross-tree inspection and attachment denial in every auth mode;
  - helper environment clearing or crash-atomic startup;
  - control-socket absence;
  - refusal when the routing tool is missing from a session;
  - an acceptance case listed in the design
- **THEN** record the failed or unmeasured condition and withhold a product support claim for that combination

#### Scenario: Harness started without the routing tool
- **WHEN** a session of the real harness starts but the qualified routing tool is not offered to the model
- **THEN** the record shows that the session was refused rather than continued with local execution, or that the harness fails this gate

#### Scenario: Platform outside the experiment
- **WHEN** a result is requested for macOS
- **THEN** the record states that macOS was not measured and claims nothing for it
