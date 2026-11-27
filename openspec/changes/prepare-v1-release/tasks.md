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

## 4. Reconcile and record

- [ ] 4.1 Reconcile `add-agent-workload` task 2.4 (a single host credential file, such as `~/.claude/.credentials.json`, granted read-only) with the per-sandbox login of `add-claude-agent-recipe`. Either retire the file-shaped credential or record why both stay, in both changes.
- [ ] 4.2 Reconcile `add-agent-interaction`’s attention channel with OSC 7501 transport (`support-osc7501-terminal`). Say which one reports agent state in 1.0, or how they coexist.
- [ ] 4.3 Fix `up`’s tunnel hint (`src/lifecycle/up.rs`), which suggests `devcroft ssh -L …` although `devcroft ssh` takes only `[--no-up] [name]`. Suggest `ssh -L LOCAL:127.0.0.1:PORT -N NAME.devcroft`, and close the gap in `docs/known-gaps.md`.
- [ ] 4.4 Record in `docs/roadmap.md` as post-1.0 and non-blocking: `status --json` with its own DTO (never `Meta`), a static `up --plan`, task receipts that separate claimed from observed, disposable clones before an unattended run, and metadata-only export.
- [ ] 4.5 Update `openspec/config.yaml`’s `context:` to what 1.0 ships: swift out of the standard build, no host shell for mise, and the file-only workflow’s “no non-reproducible execution”.

