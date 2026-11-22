## ADDED Requirements

### Requirement: Two restricted trees under an unrestricted parent
For a sandbox in mode 2, `up` SHALL start an unrestricted host-side parent.
The parent SHALL create every listener and fd before restriction, then start
the project executor and the agent as separate children, each applying its
own profile to itself. The parent SHALL run no session itself. Sandboxes
without an agent, or in mode 1, SHALL keep the single self-restricting keeper
unchanged.

#### Scenario: Agent-less sandbox unchanged
- **WHEN** a sandbox without an agent is brought up
- **THEN** its process topology and compiled profile are identical to those before this change

#### Scenario: Down tears down both trees
- **WHEN** `down` runs on a mode 2 sandbox
- **THEN** the parent, the executor, the agent and every descendant have exited
