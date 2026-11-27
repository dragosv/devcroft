## 1. Build a disposable experiment

- [ ] 1.1 On a fresh Linux sandbox (`network.default = "deny"`, mount isolation), select one terminal-qualified recipe/version and one auth method, and use a synthetic credential. Record the platform. Establish the topology from `design.md`:
  - the project keeper with a typed executor ingress;
  - an agent keeper starting one harness per session;
  - a minimal supervisor;
  - separate environments, homes, mount plans and proxy routes;
  - every host-side helper started from a cleared environment.

  Bind the pair by root, sandbox name and generation, and report `Ready` only when both keepers and the executor answer for it.
- [ ] 1.2 Run the acceptance cases `R1`–`R7`, `W1`–`W4`, `L1`, `S2` and `N1` from `design.md` on the fixture, including descendants that use `setsid` and double fork. Report any case that did not run as not run. Also verify:
  - `[env] forward` credential collisions are refused;
  - canonical `filesystem.read` / `read_write` overlaps on the agent `HOME` or executor socket directory are refused, in the new sandbox and in a live one;
  - neither tree can inspect, attach to or signal the other;
  - `svc.sock` and the other control sockets are absent from the tree that must not reach them.

## 2. Decide from one real harness

- [ ] 2.1 Route one real agent’s test command through the prototype. Compare ACP terminal capabilities, Codex App Server client-declared tools and an MCP tool where the harness offers them, and qualify each `(component, platform, launch path, routing mechanism)` separately. Record:
  - whether its native tool can be disabled;
  - whether a session detects a missing routing tool and refuses;
  - which commands remain local and which project settings affect the agent;
  - what happens to the project when the agent or a helper exits.

  Rerun the cases from 1.2 that apply to the real harness.
- [ ] 2.2 Record reproducible observations, unmeasured cases and a proceed/defer decision. Carry the nine mandatory Mode 2 gates in `design.md` into any follow-up production proposal. Leave public commands and existing sandbox state untouched throughout the experiment.
