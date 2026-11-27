## 1. Say what 1.0 promises

- [ ] 1.1 Rewrite the 1.0 section of `docs/roadmap.md` to the criterion accepted on 2026-11-27. List fleet, microVM and Mode 2 as non-blocking.
- [ ] 1.2 Write `docs/stability.md` with the stable surface: commands, flags, exit codes, error layers, configuration keys and listed behaviors. State that message wording and Rust `pub` items aren't stable.
- [ ] 1.3 With the owner, set up the private security channel (GitHub private vulnerability reporting by default). Write `SECURITY.md`, and point the README’s security section at it.

## 2. Prove the matrix and the artifact

- [ ] 2.1 Add a release workflow that joins the agent qualification records, the provider rows and the backend checks into the stable set. Confirm that a skipped mandatory probe keeps its combination out, and that the release notes name it.
- [ ] 2.2 In CI, install devcroft from the package being published and bring up a sandbox with it.
- [ ] 2.3 Declare the oldest readable `Meta` and configuration versions. Test the new build against the previous release’s sandboxes: each runs unchanged, or is refused naming `up --recreate`.
- [ ] 2.4 Run two worktrees of one project in parallel on the published build, each with an agent completing an edit and a test.

## 3. Clean up after a crash

- [ ] 3.1 Kill each component (keeper, proxy, relay, agent) of a running sandbox on Linux and on macOS. Record what survives.
- [ ] 3.2 End every surviving process with the sandbox’s authority, through the PID namespace and `fleet::lifeline` on Linux and a measured lifeline on macOS. Report the crash in `status` and recover at the next `up`. Record what a platform can’t guarantee in `docs/known-gaps.md`.
