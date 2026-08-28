# Tasks — Sandbox Provider Resolution

## 0. Spike first — this sizes the whole change

- [ ] Measure what `flox activate` actually needs when confined: run it under a
      deny-by-default profile against a real project and add grants until it
      works. Record the minimal set. Candidates to check: the provider's config
      and data directories, the nix daemon socket, `TMPDIR`, the terminal.
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
