## ADDED Requirements

### Requirement: Attribute login destinations to the active recipe

Only a selected login mode SHALL add the recipe's qualified login hosts with the recipe's origin. The API-key route SHALL retain the existing broker policy.

#### Scenario: Login hosts rendered only under login auth
- **WHEN** `claude-code` is brought up with `auth = "login"` vs `auth = "api-key"`
- **THEN** the recipe's login hosts appear with origin `agent:claude-code` under `auth = "login"` and are absent under `auth = "api-key"`
