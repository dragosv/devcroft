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
- **WHEN** routing, credential separation, cleanup, `network.default = "deny"` / socket isolation, `Meta.mode1_recipes` quarantine, or (on macOS for `auth = "env"`) `KERN_PROCARGS2` environment isolation has not been demonstrated
- **THEN** record the failed or unmeasured condition and withhold a product support claim for that combination
