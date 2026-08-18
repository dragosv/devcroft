## 1. Config surface

- [ ] 1.1 Allocation request in `[network]`, naming the environment
      variables to allocate; unknown keys rejected with the full key path
      like every neighbouring section
- [ ] 1.2 Reject at parse time, not at `up`: a malformed request, and an
      empty or non-identifier variable name
- [ ] 1.3 Regression test: a manifest requesting no allocation produces a
      byte-identical `policy --render` and a byte-identical generated
      service config to before this change — the migration plan's stated
      invariant

## 2. Choosing and recording the port

- [ ] 2.1 Port chooser: bind `127.0.0.1:0`, read the assigned port,
      close. Host-side at `up`, in the trusted phase
- [ ] 2.2 Record the chosen port in `meta.json`, reading as empty for
      sandboxes recorded before the field existed (same posture the
      isolation-tier field used)
- [ ] 2.3 Reuse the recorded port across `down`/`up` (design.md decision
      2): a connection string stays valid for the sandbox's life
- [ ] 2.4 Fall back to choosing a new port when a recorded one can no
      longer be granted — the record is a preference, not a contract with
      the rest of the host
- [ ] 2.5 Test: same port across a restart cycle; a fresh port after `rm`
      then recreate

## 3. Policy

- [ ] 3.1 Compile allocated ports into the same backend rule
      `network.ports` produces, with an origin marking them allocated
- [ ] 3.2 `policy --render` shows them with that origin — this matters
      more than for manifest rules, since the user did not choose the
      value and cannot predict it
- [ ] 3.3 Test: a sandbox with one allocated and one fixed port renders
      both, distinguishably; compilation stays deterministic given the
      same recorded allocation

## 4. Reaching the processes

- [ ] 4.1 Inject the allocated variable into the sandbox environment, so
      sessions (`exec`/`shell`) can read it
- [ ] 4.2 Substitute it into the generated process-compose config,
      overriding whatever the provider's `vars` declared for that
      variable — without touching the provider's manifest on disk
- [ ] 4.3 Leave every other declared variable exactly as the provider
      set it
- [ ] 4.4 Fail `up` naming the service when allocation is requested for a
      service whose command hardcodes its port and never references the
      allocated variable (design.md decision 1's stated limitation).
      **Detect by looking for the variable reference**, not by trying to
      parse the port out of arbitrary shell

## 5. Discovery

- [ ] 5.1 `status` reports each allocated port with the variable carrying
      it. Part of this change, not a follow-up — an allocated port nobody
      can find is as useless as one that collides, and shipping the
      mechanism before the visibility is the mistake `add-flox-services`
      already made once with services (its design decision 7)
- [ ] 5.2 Decide and implement what `status` shows for a *stopped*
      sandbox: the recorded port is still meaningful, but printing it
      unqualified reads as though something is listening

## 6. The test that justifies the change

- [ ] 6.1 Two sandboxes from two real `git worktree` checkouts of one
      repo, identical committed manifest, same declared service: both come
      up, neither collides, each reports its own port, and connecting to
      each reaches *that* sandbox's service and not the other's.
      **Note the dependency:** without `add-agent-workload`'s per-root
      sandbox identity, the two worktrees share a sandbox name and never
      get as far as needing two ports — so this test either follows that
      change or uses explicit distinct names and says so

## 7. Docs

- [ ] 7.1 README known gaps: remove the port-collision limitation, or
      narrow it to what remains (hardcoded-port services, and the
      allocate-then-bind race)
- [ ] 7.2 `docs/decisions.md`: record why commands are not rewritten and
      why ports are not offset — both were considered and both fail on a
      named property, per that file's convention
- [ ] 7.3 A sample demonstrating two sandboxes of one repo running the
      same service simultaneously, which is the claim this change exists
      to make true

## 8. Verification

- [ ] 8.1 `cargo build`, `cargo clippy`, `cargo fmt` clean
- [ ] 8.2 `openspec validate --all` passes
- [ ] 8.3 Report honestly whether the allocate-then-bind race was
      observed in practice during testing, rather than only reasoned
      about — design.md decision 4 accepts it on the assumption it is
      rare, and that assumption is worth checking
