## 1. Build a disposable experiment

- [ ] 1.1 Select one terminal-qualified recipe/version and one auth method on a fresh Linux `network.default = "deny"` sandbox with mount isolation. Record the platform and use a synthetic credential. Establish separate project and agent processes, environments, homes, mount plans and proxy routes, with every host-side helper started from a cleared environment. Bind the pair by root, sandbox name and generation.
- [ ] 1.2 Trace direct and shell-launched project commands, working directory, descendants (including `setsid` and double fork), cancellation and cleanup. Record the project toolchain and the agent runtime. Verify the following, and confirm that a project command can neither read the synthetic credential nor use the agent’s route:
  - `[env] forward` credential collisions are refused;
  - canonical `filesystem.read` / `read_write` overlaps on the agent `HOME` or executor socket directory are refused, in the new sandbox and in a live one;
  - neither tree can inspect, attach to or signal the other;
  - `svc.sock` and the other control sockets are absent from the tree that must not reach them.

## 2. Decide from one real harness

- [ ] 2.1 Route one real agent’s test command through the prototype, comparing ACP terminal capabilities, Codex App Server client-declared tools and an MCP tool where the harness offers them. Record whether its native tool can be disabled, which commands remain local, which project settings affect the agent, and what happens to the project when the agent or a helper exits.
- [ ] 2.2 Record reproducible observations, unmeasured cases and a proceed/defer decision. Carry the eight mandatory Mode 2 gates in `design.md` into any follow-up production proposal. Leave public commands and existing sandbox state untouched throughout the experiment.
