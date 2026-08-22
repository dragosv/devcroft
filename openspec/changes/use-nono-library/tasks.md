## 1. Wire the dependency

- [x] 1.1 Add `nono` to `Cargo.toml` (the library crate, not `nono-cli`) —
      pinned `"0.74.0"`, the top of own-policy-baseline's verified range
- [x] 1.2 Confirm the build accepts the trust-dependency surface as
      measured in design.md (141 net-new crates) — no feature flags
      needed since Decision 4 accepted it whole, but record the actual
      `cargo tree` diff against what design.md predicted (measured:
      devcroft's own tree grew from 219 to 472 unique normal-edge crates,
      254 net new — devcroft's baseline has grown since design.md's
      isolated 158-crate measurement, e.g. from `own-policy-baseline`;
      the shape — Sigstore/TUF + a second TLS stack — matches what was
      predicted, builds and compiles clean)

## 2. Project CompiledPolicy into a CapabilitySet

> Ended up as two types, not one: `CompiledPolicy` (origin-tracking,
> devcroft-internal) → `CapabilityPlan` (plain-value, `Serialize`/
> `Deserialize`, the actual wire format handed to the keeper) →
> `nono::CapabilitySet` (the library's own type). `Origin` carries
> `&'static str`, which has no `Deserialize` impl, and the keeper needs a
> value it can reconstruct across an exec boundary — see
> `capability_set.rs`'s module doc.

- [x] 2.1 `CapabilityPlan::to_capability_set(project_root) ->
      Result<nono::CapabilitySet, CapabilitySetError>` — `filesystem_allow`/
      `filesystem_read` → `allow_path`/`allow_file` (`ReadWrite`/`Read`).
      Empirically confirmed `filesystem_deny` has no direct primitive:
      Landlock is purely additive, and live-tested that `nono-cli` itself
      refuses to start on a deny nested inside a broader allow ("Landlock
      deny-overlap is not enforceable on Linux"). So deny entries are never
      passed to the library at all — nothing grants them, Landlock denies
      by default — and `to_capability_set` instead detects the overlap
      case at compile time and returns `CapabilitySetError::DenyOverlapsAllow`
      rather than silently producing an unenforceable sandbox
- [x] 2.2 `network_block`/`network_ports` → `NetworkMode::Blocked`/
      `AllowAll` + `allow_localhost_port`; `network_allow_domain` grants
      no capability (design.md Non-Goal: `network.allow` compiles to
      `NetworkMode::Blocked`, matching today's actual behavior — see the
      finding below)
- [x] 2.3 `signal_mode` → `set_signal_mode(SignalMode::Isolated)`
- [x] 2.4 Test: for representative manifests, the capability set grants
      exactly what `CompiledPolicy`/`CapabilityPlan` record
      (`policy::capability_set::tests::grants_the_project_root_read_write`,
      `nonexistent_grant_is_skipped_not_an_error`, `plan_round_trips_through_json`)
- [x] 2.5 Test: `DEVCROFT_DATA_DIR`/a manifest deny nested under a broader
      allow is rejected at compile time
      (`deny_nested_inside_a_broader_allow_is_a_compile_error`) — real
      overlap live-verified against `nono-cli`'s own refusal first (see
      2.1), then pinned down as a devcroft-level test

**Finding, not originally scoped:** `allow_path`/`allow_file` require an
absolute, *existing*, canonicalizable path (`FsCapability::new_dir`
canonicalizes before granting) — nono-cli's profile reader tolerated
`~`/project-relative forms and silently dropped nonexistent grants (the
multiarch `KEEPER_SYSTEM_READ` entries depend on this: only one of
`/lib/x86_64-linux-gnu`/`/lib/aarch64-linux-gnu` exists per host).
`capability_set.rs`'s `resolve`/`grant` replicate both behaviors: resolve
against `project_root`/`$HOME`, skip (never error on) a grant that
doesn't exist.

## 3. Remove the group-injection and JSON-profile machinery entirely

> Confirmed with the project owner: the process tier stops enforcing
> nono-cli's ~100-path group catalog (browser data, keychains, shell
> history/configs). `SENSITIVE_PATHS` + `DEVCROFT_DATA_DIR` — unchanged
> from today — is what the process tier denies going forward.

- [x] 3.1 Delete `render_backend_enforced`, `group_paths`, and the
      `BACKEND_ENFORCED_GROUPS` constant
- [x] 3.2 Delete `Origin::BackendEnforced` and its `Display` arm
- [x] 3.3 Delete `to_nono_profile` and the `NonoProfile`/`NonoMeta`/
      `NonoFilesystem`/`NonoNetwork`/`NonoSecurity`/`NonoGroups` types,
      `GROUPS_EXCLUDE`, `NONO_SCHEMA_URI`, `NONO_BASELINE_PROFILE`
- [x] 3.4 Delete `why`'s `backend_group` extraction and the
      `QueryProfile`/`-p <file>` machinery — `why_path` is now a pure,
      infallible function of `CompiledPolicy` (signature changed from
      `Result<Explanation, WhyError>` to `Explanation`; `WhyError` deleted
      entirely, `cli_why` updated)
- [x] 3.5 Test: `why.rs` shells out to nothing — structural, by
      inspection (no `std::process::Command` anywhere in the rewritten
      file) rather than a runtime `PATH=/nonexistent` test, since there is
      no subprocess call left to disable
- [x] 3.6 `cli_policy` collapses back to `render()` alone
- [x] 3.7 Rewrote the tests that exercised `to_nono_profile()`:
      `compilation_is_deterministic` now compares two `CapabilityPlan`s
      directly (derives `PartialEq`, no JSON round-trip needed);
      `network_ports_compile_to_open_port_alongside_a_deny_default` keeps
      its `CompiledPolicy`-level assertions, drops the JSON-shape ones.
      `nono_profile_json_matches_expected_shape` and
      `compiled_profile_validates_and_executes_under_real_nono` are
      **deleted, not replaced in kind** — see the note below
- [x] 3.8 Delete the now-obsolete tests
      (`render_backend_enforced_shows_required_and_optional_groups`,
      `every_resolved_group_is_accounted_for_by_render`,
      `why_path_attributes_backend_enforced_denial_to_a_backend_group`)

**Why 3.7 deletes rather than replaces two tests:** `Sandbox::apply_auto`
is irreversible and process-wide — calling it inside a `cargo test`
process would restrict every other test sharing that process, so there is
no unit-test equivalent of "compiled profile validates and executes under
real nono" for the library. `capability_set.rs`'s own tests cover the
shape (a `CapabilitySet` contains the right grants); the real functional
proof ("self-restriction actually works") now lives entirely in the
integration suite (task group 4), which spawns the real binary as its own
process.

## 4. The keeper self-restricts

> The highest-consequence risk (design.md Risks), and it's real: verified
> live, not just by inspection.

- [x] 4.1 `keeper_main` calls `self_restrict()` (which calls
      `Sandbox::apply_auto(&caps)`) as its first action — before
      reconstructing the listener fds, before `install_shutdown_handler`,
      before the SSH server starts, before `start_services_if_requested`
- [x] 4.2 The keeper receives a `CapabilityPlan` (not a `CompiledPolicy` —
      see task group 2's note) serialized to JSON in the
      `DEVCROFT_CAPABILITY_PLAN` env var, built once by `up_process` and
      also used there to validate the projection *before* creating any
      listener or spawning anything (fails fast at the `config` layer,
      new `UpError::Policy` variant, exit code 2)
- [x] 4.3 `spawn_keeper` drops the `nono wrap -p <profile> --` prefix —
      `Command::new(exe)` directly
- [x] 4.4 `up_process` still writes `paths.profile`, for inspection/
      debugging parity with the existing "`down` keeps the compiled
      policy" guarantee (`lifecycle::terminate`/`state` tests assert
      `paths.profile.exists()` after `down`) — content is now the
      serialized `CapabilityPlan`, not a nono-cli profile; nothing reads
      it back, it exists for a human to inspect
- [x] 4.5 Verified live (`tests/lifecycle_up.rs`,
      `up_spawns_a_working_keeper_and_down_tears_it_back_down`, already
      exercises exactly this): the control socket is reachable and a
      client connects successfully after self-restriction — full
      integration suite result recorded below
- [x] 4.6 Verified live via `tests/lifecycle_hooks.rs`'s existing suite
      (hooks dispatch only after `wait_until_responsive`, which only
      succeeds once the keeper is serving — i.e., after self-restriction)
- [x] 4.7 Verified live: `ps` during a running sandbox shows
      `/workspaces/devcroft/target/debug/devcroft __keeper <fd> <fd>` as
      a direct child of `up`, no `nono` process anywhere in the tree
- [x] 4.9 Fixed two real gaps `KEEPER_SYSTEM_READ` (read-only) missed —
      `/dev/pts` and `/dev/null` both need read+write, not read-only, for
      pty sessions and `Stdio::null()` respectively. Both were masked by
      own-policy-baseline's still-active `system_write_linux` group and
      surfaced live as an opaque keeper-spawn failure; see design.md
      Decision 1's "what actually went wrong" addendum for how they were
      isolated. New `KEEPER_SYSTEM_READWRITE` constant carries both
- [x] 4.8 Full real-`nono`-crate integration suite: 199 lib tests plus
      every `tests/*.rs` integration file, 100% pass, zero failures
      (confirmed with two full back-to-back runs after the `cargo fmt`/
      `clippy` cleanup pass; one incidental failure on an unrelated
      parallel run of `post_create_does_not_rerun_on_recovery_but_post_start_does`
      did not reproduce in isolation or on a clean re-run — pre-existing
      test-parallelism flakiness, not a regression from this change)

## 5. doctor and degraded-capability detection move to the library

- [x] 5.1 `doctor_backend` reports `nono::Sandbox::support_info()`
      (platform + kernel capability) — no `PATH` lookup, no version
      number
- [x] 5.2 Deleted `doctor_backend_profile_compatibility` and the
      `ScopedFileRemove` helper it used — no schema to validate and no
      group semantics to check once nothing is emitted as JSON for an
      external binary to parse
- [x] 5.3 `policy::degraded::detect`/`HostCapabilities` left unchanged —
      verified (not assumed) that its domain-filtering claim is still
      accurate: `network.allow` grants no capability under the library
      either (task 2.2), so "not enforced" stays true on both platforms,
      exactly what it already reported
- [x] 5.4 Verified live: `devcroft doctor`'s backend line passes with
      `nono` absent from `PATH` entirely (nothing left to look for there)

## 6. Publish what changed

- [x] 6.1 README Status section: the process tier no longer requires
      installing `nono` — a user-visible simplification, and the
      strongest practical argument for the change per the proposal
- [x] 6.2 README: recorded the security-relevant scope decision from
      design.md Decision 5 — the process tier's credential/privacy
      protection is `SENSITIVE_PATHS`, not nono-cli's broader group
      catalog, and always has been for the paths that matter to
      devcroft's own threat model. (`docs/decisions.md` needed no
      corresponding edit — it documents rejections/gaps, and this isn't
      one; the README Status section is the right home, matching how
      own-policy-baseline's own record lives there too)
- [x] 6.3 `.devcontainer/Dockerfile`: decided — **kept**, with an updated
      comment. No longer a runtime dependency of the process tier, but
      this is a development image, not an end-user install, and the
      binary stays useful for manual `nono profile`/`why`/`wrap`
      comparisons (how this change and own-policy-baseline were actually
      investigated) and `add-gvisor-backend`-adjacent debugging
- [ ] 6.4 File the upstream ask from design.md Decision 3 (gate the
      trust module behind a Cargo feature) as an issue against
      `nolabs-ai/nono` — **not done by this session**: filing an issue on
      a third-party repo is a visible external action outside what an
      agent should do unprompted; left for the project owner to send (or
      ask for explicitly) when ready
