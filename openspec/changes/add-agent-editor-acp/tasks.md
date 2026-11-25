## 1. Connect one editor

- [ ] 1.1 Implement host `agent acp [<recipe>]` and `--print-config` for one editor, using an absolute executable path. Reuse the interactive target and selection checks; propagate `--agent <recipe>` to detached `up` on a brand-new sandbox, refuse missing selection/ACP/objects immediately on `stderr` with an actionable command.
- [ ] 1.2 Implement the user-only credential command for GUI sessions when the host key variable is unset or empty (`""`). Prove it runs once inside `up`, never during preflight or doctor, fails at layer `config` on non-zero exit, timeout or empty output, and keeps the value absent from the sandbox, logs and metadata for the brokered mode.
- [ ] 1.3 Measure the editor timeout and implement bounded startup, serialized background preparation, log reporting and retry. Exercise a slow first checkout, a failed prepare, a retry during prepare, an outside or nested cwd, and a stopped named sandbox.
- [ ] 1.4 Implement piped sessions, EOF grace and process-group cleanup. Verify clean stdout including during startup, byte-transparent cancellation, two concurrent conversations, and preservation of services after disconnect. Implement `doctor --probe-acp` on a running sandbox.

## 2. Qualify the editor workflow

- [ ] 2.1 Launch the chosen editor from the OS GUI without a key in the environment. Use the printed configuration to edit a file and run its test in the intended project. Confirm a package upgrade preserves a running session and the suggested recreate command works verbatim.
- [ ] 2.2 Publish the tested editor version, cwd convention, auth setup, startup deadline, diagnostic command and disconnect behavior.
