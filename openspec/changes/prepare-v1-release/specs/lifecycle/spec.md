## ADDED Requirements

### Requirement: A crash leaves nothing with the sandbox's authority
When a keeper, egress proxy, relay or agent process of a sandbox dies,
devcroft SHALL end the processes that hold that sandbox's authority:
sessions, services, the proxy and its relays. `status` SHALL report the crash,
naming the component that died, and the next `up` SHALL start from a clean
state. A platform where this can't be guaranteed SHALL say so in
`docs/known-gaps.md`, and its combinations SHALL stay out of the stable set.

#### Scenario: Keeper killed
- **WHEN** the keeper of a running sandbox is killed with `SIGKILL`
- **THEN** no session, service or proxy of that sandbox keeps running, `status` reports the crash, and `up` brings the sandbox back

#### Scenario: Proxy killed
- **WHEN** the egress proxy of a running sandbox is killed
- **THEN** the sandbox can't reach any host until the next `up`, and `status` names the proxy as the component that died
