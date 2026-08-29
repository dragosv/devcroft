## 0. Establish scope before building anything

> Added after review found the proposal's central premise contradicted by
> the project's own code. Do this group first: it decides how much of the
> rest applies.
>
> **Reduced to one task by `remove-gvisor-backend`.** 0.1 and 0.3 asked
> about the `hardened` tier's own network namespace, which came from an
> OCI spec (`src/gvisor/oci_spec.rs`) that no longer exists — 0.1 would
> have had a deleted file to confirm against, and 0.3 a tier `config`
> now rejects outright. Both are struck rather than reworded: the case
> they covered has no live instance, and the case that replaces it
> (fleet's per-agent netns) belongs to `add-linux-agent-fleet`, which
> owns the namespace and the host-side mapping alike. 0.2's principle is
> what survives, with one live branch instead of three.

- [x] ~~0.1 Confirm against `src/gvisor/oci_spec.rs`...~~ **Struck:**
      the tier, its OCI spec, and the test named here were deleted by
      `remove-gvisor-backend`. Nothing to confirm.
- [ ] 0.2 Gate allocation on whether the sandbox **has its own network
      namespace**, not on which sandbox implementation it uses. Today
      that is always false, so allocation always applies; keep the gate
      rather than hardcoding "always" so fleet has the seam it needs
- [x] ~~0.3 Test: a `hardened` deny-default sandbox...~~ **Struck:** a
      `hardened` manifest now fails at layer `config` before any of this
      runs. The equivalent test belongs to fleet, against a real netns

## 1. Config surface

- [ ] 1.1 Allocation request in `[network]`, keyed by **service *and*
      variable** (design.md decision 1's addendum) — not a flat list of
      variable names, which cannot express "fail naming the service" and
      makes the 4.4 check unimplementable. Unknown keys rejected with the
      full key path like every neighbouring section
- [ ] 1.2 Reject at parse time, not at `up`: a malformed request, an
      empty or non-identifier variable name, and a request naming a
      variable with no service (unless explicitly session-scoped)
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
- [ ] 2.3 **Make `write_meta` read-modify-write first — this is the hard
      part of 2.2/2.3, and it is not "add a field".** `up` today builds a
      fresh `Meta` literal on every call (`up.rs`) and `write_meta`
      replaces the whole file, so every existing field survives only
      because it is re-derived each time. A recorded allocation is the
      first field that *cannot* be re-derived: added naively it resets on
      the very next `up`, silently defeating decision 2. Read the
      previous `Meta` back before writing, and test that a second `up`
      preserves the recorded port
- [ ] 2.4 Fall back to choosing a new port when a recorded one can no
      longer be **bound** (not "granted" — the policy grants whatever it
      is asked; only a bind attempt can discover unavailability, which is
      decision 4's race run a second time)
- [ ] 2.5 Report the fallback at `up`, naming the old and new port. A
      silent swap invalidates exactly the connection string stickiness
      exists to protect — the mechanism-without-visibility mistake
      `add-flox-services` decision 7 already made once
- [ ] 2.6 Test: same port across a restart cycle; a fresh port after `rm`
      then recreate; a repeated `up` does not reset the record; a
      reallocation is announced

## 3. Policy

- [ ] 3.1 Compile allocated ports into the same backend rule
      `network.ports` produces (today `allow_localhost_port` via
      `policy::CapabilityPlan`, since `use-nono-library`), with an origin
      marking them allocated
- [ ] 3.2 `policy --render` shows them with that origin — this matters
      more than for manifest rules, since the user did not choose the
      value and cannot predict it
- [ ] 3.3 Render the request as *pending* when nothing is recorded yet
      (before the first `up`, or after `rm`): never omitted, never
      invented, never a failure. Same precedent provider grants already
      set, which render as none for a project that has never been up
- [ ] 3.4 Test: a sandbox with one allocated and one fixed port renders
      both, distinguishably; compilation stays deterministic given the
      same recorded allocation; rendering with no recorded allocation
      grants no port

## 4. Reaching the processes

- [ ] 4.1 Inject the allocated variable into the sandbox environment, so
      sessions (`exec`/`shell`) can read it
- [ ] 4.2 Substitute it into the generated process-compose config,
      overriding whatever the provider's `vars` declared for that
      variable — without touching the provider's manifest on disk
- [ ] 4.3 Leave every other declared variable exactly as the provider
      set it
- [ ] 4.4 Fail `up` naming the service when the service **the request
      names** hardcodes its port and never references the allocated
      variable. **Scope the check to that one service**: quantified over
      all declared services it is unimplementable — in a project with
      `db`/`worker`/`migrate` the services that legitimately never
      reference `DB_PORT` are the majority, so "any service missing it"
      fails every real project while "no service has it" passes the case
      the check exists to catch. Detect by looking for the variable
      reference, never by parsing a port out of arbitrary shell
- [ ] 4.5 Fail `up` naming the service when a request names a service the
      provider did not declare — unsubstitutable by construction, so a
      manifest error rather than a no-op

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
      up, neither collides, each reports its own port, and **a client
      inside each sandbox** reaches its own service on its own port.
      Asserted from inside deliberately — where sandboxes share the
      host's loopback, distinctness is all allocation provides (not
      isolation), and where a sandbox has its own namespace a host-side
      client cannot reach it at all.
      **Note the dependency:** without `add-agent-workload`'s per-root
      sandbox identity, the two worktrees share a sandbox name and never
      get as far as needing two ports — so this test either follows that
      change or uses explicit distinct names and says so
- [ ] 6.2 Two sandboxes sharing **one project root** under distinct
      names, the alternative to worktrees this change's own scope
      contemplates. Service artifacts are keyed per-sandbox
      (`services::artifact_dir`), so verify the two do not share a
      generated config or supervisor socket once each also has its own
      port

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
- [ ] 8.4 Settle the ephemeral-range open question before calling
      decision 2 done: drawing from the ephemeral range means a recorded
      port can be handed to an unrelated outbound connection while the
      sandbox is down, so stickiness is weakest on exactly the busy
      many-sandbox hosts that motivate this change
