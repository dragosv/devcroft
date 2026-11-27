## 1. Measure the enforcement

- [ ] 1.1 On Linux, build a fixture view with the workspace, `HOME` and `/tmp` mounted `noexec` and only a fixture object executable. Confirm that `execve` of a workspace ELF and of a shebang script fails, that `mmap(PROT_EXEC)` of a workspace `.so` fails, and whether `vm.memfd_noexec = 2` can be set in the sandbox’s PID namespace on the reference kernels.
- [ ] 1.2 On macOS, emit Seatbelt rules that deny `process-exec*` and `file-map-executable` outside the object, and confirm the same three cases. Record both platforms’ results before implementing the profile.

## 2. Select and record the profile

- [ ] 2.1 Implement `--file-only` on `up` and `agent run`: root discovery by name only (`devcroft.toml` or `.git`), the default name, no manifest read, `Meta.profile`, restart in the same profile, and refusal of a profile conflict naming `--name` and `rm`. Add the flag to `USAGE`, help and `tests/cli_help_and_version.rs`.
- [ ] 2.2 Require a selected recipe qualified for file-only on this platform, and add the per-platform `file_only` qualification to the recipe schema.

## 3. Build the sandbox

- [ ] 3.1 Compile the file-only policy: workspace read-write with origin `workspace`, `.git` read-only, a worktree’s gitdir ungranted, manifest protection, and the agent’s object and declared libraries as the only executable set. Leave out the runtime directory, the tooling layer and any provider or store. Refuse at layer `backend` on a host that cannot enforce it.
- [ ] 3.2 Deny default egress except the recipe’s hosts through the proxy and broker. Withhold and name the user’s credentials, and refuse `loopback_any` recipes.
- [ ] 3.3 Admit only the recipe’s entrypoints in the keeper. Refuse `shell`, `exec`, `ssh` and `proxy` at layer `config` naming the profile, and print the `stderr` line on `agent run`.

## 4. Qualify one recipe

- [ ] 4.1 Build an adversarial fixture project with:
  - a committed executable and a shebang script;
  - a harness plugin and a stdio MCP entry that each write a marker;
  - a hook in the harness’s project settings;
  - a script an interpreter switch could run;
  - attempted writes to `.git/hooks` and `.git/config`.
- [ ] 4.2 Qualify OpenCode, or the recipe `add-agent-packages` ships, per platform against D8. Start with its project plugin directory, ripgrep and `BUN_BE_BUN`. Record each criterion’s result and the features that fail closed. Mark `file_only` only where every criterion passed.
- [ ] 4.3 Complete a real analysis-and-edit task on a project with no provider, from the terminal and from the qualified editor. Confirm the evidence separates static analysis from execution.

## 5. Revise the decision and document

- [ ] 5.1 Revise `docs/decisions.md` and the framing rules in `AGENTS.md` and `docs/invariants.md` from “no non-reproducible mode” to “no non-reproducible execution”, keeping `host` and `none` rejected as providers.
- [ ] 5.2 Add a file-only column to `docs/agents.md`. Keep the `docs/known-gaps.md` entry on `.git` in trusted managed checkouts current. Describe the workflow in the README and in `docs/roadmap.md`’s 1.0 section.
