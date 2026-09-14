## 1. The decision, in `up`

- [ ] 1.1 `UpOptions` gains `allow_degraded_isolation: bool` (default
      false); `Meta` gains `isolation_fallback: Option<String>` with
      `#[serde(default)]`, and `read_meta` of a pre-field `meta.json`
      yields `None` (unit test on the JSON, not on a file).
- [ ] 1.2 Rewrite the `(true, false)` arm of the isolation `match` in
      `src/lifecycle/up.rs` as design D2: deny-default → refuse naming the
      claim and both ways out; no flag → refuse naming the sysctl and the
      flag; flag + allow-default → one warning (D6 wording, reusing the
      macOS arm's shape), `isolate_filesystem = false`, and the fallback
      written into `Meta` before the keeper spawns. The `deny_default`
      predicate is computed once and shared with the macOS arm.
- [ ] 1.3 Make sure `isolate_filesystem = false` on Linux takes exactly
      the pre-`add-mount-isolation` path in `spawn_keeper`'s `pre_exec`
      (no `enter_mount_namespace`, no `construct_view`, no
      `set_current_dir` into a view) and that `isolate_network` is forced
      false with it — the same coupling the macOS comment there already
      describes. Read the comment; if it says "nothing here runs at all",
      make that literally true for this arm too.

## 2. Surfacing

- [ ] 2.1 `status`: a `isolation:` line rendered from `Meta.isolation_fallback`
      (design D3), present only when set; text per the lifecycle delta's
      "Status reports the isolation actually applied". `SandboxStatus`
      carries the field for the JSON shape too.
- [ ] 2.2 `backend_capabilities`: `pathname-unix-sockets` Linux entry
      becomes probe-dependent (design D5) — `enforced` when
      `mount_namespace_available()`, otherwise
      `EnforcedWithNamedDegradation` with the opt-in named. Check the
      `Status` vocabulary's own rules (`add-not-applicable-status`) before
      choosing the word; `degraded` is the existing one for "enforced
      elsewhere, not here".
- [ ] 2.3 `doctor`'s mount-isolation line names the flag and its cost on
      a host where the probe fails (filesystem-view delta, "Diagnosis
      before the attempt").

## 3. CLI surface

- [ ] 3.1 `cli_up` parses `--allow-degraded-isolation`; `USAGE` and
      `cli_up`'s own usage string list it; `tests/cli_help_and_version.rs`
      is what fails if `USAGE` is forgotten — confirm it does by removing
      the line once.
- [ ] 3.2 `maybe_auto_up` (exec/shell) passes default options unchanged;
      when `up` fails with the namespace error, the CLI's message appends
      the remedy naming `devcroft up --allow-degraded-isolation` (exec
      delta). Exit code stays 4 (`backend`).
- [ ] 3.3 Assert the flag is not ambient: no environment variable and no
      manifest key is read for it (cli delta, "The flag is not ambient").
      A test that sets a plausible env var and confirms `up` still
      refuses.

## 4. Tests

- [ ] 4.1 A test module that self-skips unless the host *cannot* create
      the namespace (`mount::probe` false) — the inverse of every other
      guard in `tests/`, printed as such — covering: no flag → refused
      naming both remedies; flag + `network.default = "allow"` → started,
      one warning, `status` shows the fallback, `Meta` has it; flag +
      `network.default = "deny"` → refused naming the claim; auto-up via
      `exec` → refused naming the flag.
- [ ] 4.2 The existing fail-closed test (`tests/mount_view_e2e.rs` or
      wherever "cannot create unprivileged mount namespaces" is asserted)
      keeps passing unchanged — the default did not move.
- [ ] 4.3 A control on a host *with* the namespace: passing the flag
      changes nothing — the view is built, `Meta.isolation_fallback` is
      `None`, no warning is printed. The flag must be inert where it is
      not needed.
- [ ] 4.4 CI: a dedicated `ubuntu-latest` leg that *omits* the sysctl step
      and runs the 4.1 module (and only it), named for what it is —
      `e2e (landlock-only fallback)`. No other leg passes the flag. The
      leg is blocking once green.

## 5. Documents

- [ ] 5.1 `add-mount-isolation/design.md` M4: a cross-reference paragraph
      saying the fail-closed default is unchanged, the opt-in exists, and
      where its gate is defined — do not rewrite M4's rationale.
- [ ] 5.2 `docs/known-gaps.md`, the AF_UNIX entry ("both halves now
      closed"): the Linux half is closed *where the view exists*; under
      the fallback it is open by declaration, and how a reader tells
      which they have (`status`, `doctor`).
- [ ] 5.3 `docs/threat-model.md`: which use case the fallback backs
      (accident protection on a host the operator controls) and which it
      does not (an agent with credentials in the proxy — refused by the
      gate anyway).
- [ ] 5.4 `README.md` platform-support table: Linux row notes the
      Ubuntu 24.04 default and the two remedies, in one line.
- [ ] 5.5 `CLAUDE.md`: one sentence under the architecture invariants
      that the fallback exists, is opt-in, and is gated on network mode —
      so a future change that adds a view-only guarantee knows to update
      the gate (design risk 2).
