## 1. Connect one editor

- [ ] 1.1 Implement host `agent acp [<recipe>]` and `--print-config` for one editor, using an absolute executable path. Reuse the interactive target resolver and selection checks, including the nested-repository refusal. Refuse a missing selection, `acp = "none"`, a down sandbox and one in preparation immediately on `stderr` with an exact command, starting nothing.
- [ ] 1.2 Decide “running” from the keeper’s answer for the generation recorded in `Meta`, not from an accepted connection. Test a socket that accepts before the keeper admits sessions, an `up` holding the lifecycle lock, a stopped named sandbox, and an outside or nested `cwd`.
- [ ] 1.3 Confirm `agent acp` never runs the credential command from `add-agent-packages`, and that a GUI-launched editor with no key in its environment attaches to a sandbox brought up with that command.
- [ ] 1.4 Implement piped sessions, EOF grace and process-group cleanup. Verify clean `stdout` (`T11`), byte-transparent cancellation, two concurrent conversations, and preservation of services after disconnect. Add fixture children using `setsid`, `setpgid`, double fork and inherited stdio, and confirm the published guarantee matches what they show. Implement `doctor --probe-acp` on a running sandbox, stating that `initialize` does not prove authentication.
- [ ] 1.5 Add Testy at a pinned revision as a git dev-dependency and run the protocol cases against `agent acp` with a fixture agent. Confirm `cargo package` drops the dependency. Assert effects (denials, cleanup, credentials) through `devcroft-test-agent` and OS probes, not through Testy’s messages.

## 2. Qualify the editor workflow

- [ ] 2.1 Bring the sandbox up from a terminal, then launch the chosen editor from the OS GUI without a key in its environment. Use the printed configuration to edit a file and run its test in the intended project. Measure warm `initialize` time against the editor’s timeout. Confirm a package upgrade preserves a running session and that the printed `--agent-upgrade` command works verbatim.
- [ ] 2.2 Repeat with two worktrees and a session in each. Confirm that a call the policy denies stays denied when the client approves it, or when the harness runs without its own permission prompts. Record one sanitized ACP transcript of the qualified recipe for regression.
- [ ] 2.3 Publish the tested editor version, `cwd` convention, the `up`-first workflow, auth setup, initialize time, diagnostic command and disconnect behavior, including whether the harness leaves processes outside the session’s group.
