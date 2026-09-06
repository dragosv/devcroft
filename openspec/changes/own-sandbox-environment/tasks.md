# Tasks — Own the Sandbox Environment

## 0. Establish the baseline before changing it

> The measurement that motivated this change was taken from one host, one
> provider, one shell. Before removing 101 variables from every sandbox, know
> whether that number is representative.

- [ ] 0.1 Record what reaches a sandbox on each provider row (flox, nix,
      devbox), not just flox: how many variables, how many are byte-identical to
      the invoking shell, and what the provider genuinely contributes.
- [ ] 0.2 Do the same from a *clean* shell — `env -i` plus the minimum — so the
      101 is separated into "what any shell leaks" and "what this developer's
      shell happens to hold". The first number is the one that generalises.
- [ ] 0.3 Enumerate what breaks. Run the existing suite with the subtraction
      applied and record every failure **before** fixing any of them; the list
      is the migration story, and discovering it one test at a time turns a
      design question into a series of patches.

## 1. Subtract the ambient environment

- [ ] 1.1 Capture devcroft's own ambient environment at `up`, before provider
      resolution, so the comparison is against what devcroft actually inherited
      rather than against whatever the process has by then.
- [ ] 1.2 Remove every variable matching that ambient set by **name and value**
      (D1), through the existing `Resolution::unset` channel — `.envs()` can
      only add or override, which is the mistake `adopt-nono-proxy` already made
      once and had to correct.
- [ ] 1.3 Keep the essential set (D2) as one constant with a comment per entry
      saying what breaks without it. A set that grows by convenience is how this
      becomes a denylist again.
- [ ] 1.4 Confirm the provider's own contribution is untouched: a variable
      activation *set*, one it *modified*, and one it *unset* must each behave
      as before.

## 2. `[env] forward`

- [ ] 2.1 Manifest key, names only, schema-checked.
- [ ] 2.2 A forwarded variable the host does not set **warns and continues**
      (D3) — deliberately unlike a brokered route's missing credential, which
      fails `up`. Record the contrast where the code is, since two adjacent
      features answering the same question differently is exactly what gets
      "fixed" into consistency later.
- [ ] 2.3 Forwarded variables survive the subtraction, whatever their value.

## 3. `HOME`

- [ ] 3.1 `HOME=<project>/.devcroft/<name>/home`, created at `up`.
- [ ] 3.2 Confirm the artifact directory's existing lifetime is right for this:
      ignored by `init`, removed by `rm`, surviving `down`.
- [ ] 3.3 Check what assumes the old `HOME`. `policy::compile` writes
      credential denials in `~/...` shorthand and `normalize_path_for_policy`
      rewrites `$HOME`-relative paths — if either now resolves against the
      sandbox-local home, the baseline denials it produces are nonsense.
      **This is the most likely place this change breaks something quietly.**

## 4. `ssh.forward_agent`

- [ ] 4.1 **Measure first**: with `SSH_AUTH_SOCK` present and the socket path
      granted, is the host agent actually reachable from inside? macOS treats
      unix-socket `connect` as a network operation, so `network.default = "deny"`
      may refuse it regardless. The answer decides whether 4.2 is implementable.
- [ ] 4.2 If reachable: implement the key — variable plus socket grant, in one
      place. If not: remove the key and the `add-mvp-core` scenario together,
      and say so in `docs/known-gaps.md`.
- [ ] 4.3 Either way, assert the *off* case, which is the one that is a security
      property rather than a feature.

## 5. Make the removal diagnosable

- [ ] 5.1 Report the removed set somewhere a user hits at the moment they need
      it (design Open Question 1 — `status` risks burying it).
- [ ] 5.2 The report names the manifest key that restores a variable. A
      diagnosis without a remedy just relocates the confusion.

## 6. Tests

- [ ] 6.1 A shell variable no provider sets does not reach the sandbox.
- [ ] 6.2 The control: a variable the provider *does* set arrives with the
      provider's value. Without this, an implementation that removed everything
      would pass 6.1.
- [ ] 6.3 `PATH` still contains the closure — the specific case a name-only
      comparison would have broken.
- [ ] 6.4 A `forward`ed variable arrives; an unset one warns without failing.
- [ ] 6.5 A session writes under `$HOME`, the write succeeds, and nothing
      appears in the host user's home.
- [ ] 6.6 Teeth-check the subtraction: disable it and confirm 6.1 fails.

## 7. Say what changed

- [ ] 7.1 `docs/known-gaps.md`: the gap this closes, stated as it was —
      **no environment filtering existed at all**, so every exported credential
      was inside every sandbox while the baseline denied the directories those
      same credentials live in. A closed gap is still worth publishing when it
      was open in a shipped version.
- [ ] 7.2 README/migration: a project may need one `forward` line. The failure
      is a tool not finding a variable, far from its cause.
- [ ] 7.3 `docs/decisions.md`: the secret-injection position no longer needs the
      retraction `add-agent-workload` task 7.1 drafted. `forward` is the honest
      simple answer and brokering is the strong one; neither is "never via env
      vars", and the entry should say what is true rather than what was hoped.
- [ ] 7.4 Record in `add-agent-workload` that `[hooks] post_create` running an
      agent's **official installer** is now possible, since `HOME` is writable —
      a much cheaper answer to its tooling problem than devcroft shipping
      runtimes, and one that keeps the vendor's own install path rather than
      devcroft reimplementing it.
