## 1. Connect one editor

- [ ] 1.1 Implement host `agent acp [<recipe>]` and `--print-config` for one editor, using an absolute executable path. Reuse the interactive target and selection checks. Refuse a missing selection, `acp = "none"`, a down sandbox and one in preparation immediately on `stderr` with an exact command, starting nothing.
- [ ] 1.2 Decide “running” from the keeper’s answer for the generation recorded in `Meta`, not from an accepted connection. Test a socket that accepts before the keeper admits sessions, an `up` holding the lifecycle lock, a stopped named sandbox, and an outside or nested `cwd`.
- [ ] 1.3 Implement the user-only credential command used by `up` when the host key variable is unset or empty (`""`). Prove it runs once inside `up`, never during preflight, doctor or `agent acp`, fails at layer `config` on non-zero exit, timeout or empty output, and keeps the value absent from the sandbox, logs and metadata for the brokered mode.
- [ ] 1.4 Implement piped sessions, EOF grace and process-group cleanup. Verify clean `stdout` (`T11`), byte-transparent cancellation, two concurrent conversations, and preservation of services after disconnect. Add fixture children using `setsid`, `setpgid`, double fork and inherited stdio, and confirm the published guarantee matches what they show. Implement `doctor --probe-acp` on a running sandbox.

## 2. Qualify the editor workflow

- [ ] 2.1 Bring the sandbox up from a terminal, then launch the chosen editor from the OS GUI without a key in its environment. Use the printed configuration to edit a file and run its test in the intended project. Measure warm `initialize` time against the editor’s timeout. Confirm a package upgrade preserves a running session and the suggested recreate command works verbatim.
- [ ] 2.2 Publish the tested editor version, `cwd` convention, the `up`-first workflow, auth setup, initialize time, diagnostic command and disconnect behavior, including whether the harness leaves processes outside the session’s group.
