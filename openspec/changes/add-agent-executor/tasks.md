## 1. Build a disposable experiment

- [ ] 1.1 Select one terminal-qualified recipe/version and one auth method on a Linux `network.default = "deny"` sandbox with mount isolation. Record the platform and use a synthetic credential. Establish separate project and agent processes, environments, homes and proxy routes in a fresh fixture.
- [ ] 1.2 Trace direct and shell-launched project commands, working directory, descendants, cancellation and cleanup. Record the project toolchain and the agent runtime. Verify that `[env] forward` credential collisions and canonical `filesystem.read` / `read_write` overlaps on the agent `HOME` or executor socket directory are refused, and that a project command cannot read the synthetic credential or use the agent’s route.

## 2. Decide from one real harness

- [ ] 2.1 Route one real agent's test command through the prototype. Record whether its native tool can be disabled, which commands remain local, which project settings affect the agent, and what happens to the project when the agent exits.
- [ ] 2.2 Record reproducible observations, unmeasured cases and a proceed/defer decision. Carry the five mandatory Mode 2 gates in `design.md` (`network.default = "deny"`, macOS `auth = "env"` ineligibility until `KERN_PROCARGS2` is measured, `[env] forward` collision refusal, canonical `Read`+`ReadWrite` overlap refusal, and `Meta.mode1_recipes` quarantine) into any follow-up production proposal. Leave public commands and existing sandbox state untouched throughout the experiment.
