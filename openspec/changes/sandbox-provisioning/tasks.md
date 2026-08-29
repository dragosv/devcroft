# Tasks — Sandbox Provider Resolution

## 0. Spike first — this sizes the whole change

- [ ] **Determine whether Flox can separate materialization from
      `hook.on-activate` at all** — this precedes measuring grants, because
      if it cannot, no grant set is the answer (design.md P2b/P2c, open
      question 1). Check for any public, versioned materialize path, pre-hook
      context, or separate hook runner. `--mode dev`, `--mode run` and
      `--no-start-services` were already measured and all run the hook.
      **A negative result is a valid, expected outcome**, not a reason to
      widen the profile.
- [ ] Measure what `flox activate` actually needs when confined: run it under a
      deny-by-default profile against a real project and add grants until it
      works. Record the minimal set. Candidates to check: the provider's config
      and data directories, `TMPDIR`, the terminal.
      **Note what is deliberately absent from that list:** the nix daemon
      socket. It used to appear here as an ordinary candidate grant, which is
      exactly the conflation P2a rejects — measuring "does it work once I add
      the daemon" would qualify a profile that hands host-global
      materialization authority to project shell. Measure the *resolver's*
      needs separately (below) and keep the hook's profile without it.
- [ ] Measure what a **trusted resolver** needs, separately from the hook: what
      materialization requires when no project code is running. This is the
      half that legitimately holds daemon authority, and the point of measuring
      it apart is to know exactly how much authority the split is protecting.
- [ ] Qualify the hook-free paths for the two eligible providers —
      `nix print-dev-env --json` and `devbox shellenv --pure` — as running
      inside the provisioning worker with no daemon connection.
- [ ] Implement the fail-closed provider-layer diagnostic for the blocked case:
      a Flox environment with `hook.on-activate` under confined provisioning
      fails naming the hook and why, and points at the upstream request rather
      than suggesting a workaround (there isn't a safe one).
- [ ] Repeat for `devbox`.
- [ ] Confirm the environment can be written to a descriptor the supervisor
      holds, across the boundary, without a shell round trip.
- [ ] **If the minimal grant set turns out to be most of `$HOME`, stop and
      reconsider** — the change would then be confinement in name only.

## 1. Provisioning policy

- [ ] Add the provisioning profile to the manifest schema.
- [ ] Define the default profile from the spike's findings.
- [ ] Extend `policy --render` and `why` to cover both profiles with origin
      attribution.
- [ ] Report degradation where the backend cannot enforce a declared aspect.

## 2. Provisioning sandbox

- [ ] Build the provisioning sandbox from the compiled provisioning profile,
      reusing the existing sandbox construction path.
- [ ] Substituted home directory, with declared paths bound in.
- [ ] Capture the environment across the boundary as data.
- [ ] Apply the existing baseline diff, store grants and staleness
      fingerprinting to the captured environment, unchanged.

## 3. Failure attribution

- [ ] Distinguish policy denial from provider failure in `up`'s output.
- [ ] Name the denied path or interface.
- [ ] Test both paths: a hook denied a path it needs, and a hook that is simply
      broken.

## 4. Providers

- [ ] Route flox through the provisioning sandbox.
- [ ] Route devbox through it, keeping the existing lockfile byte-comparison.
- [ ] Give nix a provisioning profile for inspectability; its resolution path is
      unchanged.
- [ ] Replace the host-execution warning with an accurate statement of
      provisioning's reach.

## 5. Validation

- [ ] A project whose hook runs a package install still provisions correctly.
- [ ] A hook writing to an undeclared home path does not affect the real home.
- [ ] A hook attempting to read credentials outside the profile is denied.
- [ ] Every existing sample still provisions.
- [ ] Document the behavioural difference from running activation by hand.

## 6. Deferred decisions (see design.md)

- [ ] Network during provisioning: a real provisioning allowlist, from
      `add-egress-proxy`. Not a binary on/off — see design.md open question 2.
      **No longer deferred-and-unowned: `add-egress-proxy` has shipped, and
      its "Network policy is declared per context" requirement moved into
      this change** (`specs/network/spec.md`). The mechanism exists —
      `CompiledPolicy::network_proxy_port` is per-compilation, so a
      provisioning compilation gets its own proxy and allowlist without new
      machinery. What is missing is only the second context, which is this
      change's own subject.
- [ ] Cache sharing across agents: read-only share plus per-agent overlay.
      **Decide before fleet work**, not after.
- [ ] macOS fidelity statement.
