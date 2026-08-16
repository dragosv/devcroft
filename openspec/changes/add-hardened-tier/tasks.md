## 1. Config: isolation tier key

- [x] 1.1 Add an `isolation` field to `Sandbox` in `src/config/mod.rs`
      (`"process"` default | `"hardened"`), parsed the same way
      neighboring `[sandbox]` fields are
- [x] 1.2 Reject unrecognized `isolation` values through the existing
      `ConfigError` enum, consistent with other manifest validation
      errors
- [x] 1.3 Unit tests: omitted key defaults to `process`; explicit
      `hardened` parses; an invalid value produces `ConfigError`

## 2. Keeper: session backend abstraction

- [x] 2.1 Extract the body of `keeper::session::spawn()` into a
      `SessionBackend` trait (`fn spawn(&self, req: &SpawnRequest) ->
      io::Result<SpawnedSession>`); move today's local fork/exec
      implementation into a `LocalSessionBackend` behind it
- [x] 2.2 `Keeper::new` takes `Arc<dyn SessionBackend>` instead of
      assuming local spawn; `connection.rs`/`pty.rs`/`protocol.rs`/
      `registry.rs` stay unchanged — they never touch the spawn
      mechanism directly
- [x] 2.3 Route the ssh server's own session spawning through the same
      trait (`src/ssh/server.rs` calls `session::spawn` directly as a
      second call site — missing it would leave SSH sessions fork/exec'ing
      on the host while `exec`/`shell` dispatched into the sandbox)
- [x] 2.4 Existing keeper/session integration tests still pass unmodified
      against `LocalSessionBackend`, confirming the process tier's
      behavior is bit-for-bit unchanged by the refactor

## 3. Lifecycle: tier resolution and dispatch

- [x] 3.1 Resolve the isolation tier and concrete backend at `up` from
      `manifest.sandbox.isolation`; on macOS, `hardened` is a hard
      failure at layer `backend` naming the platform limitation, never a
      silent downgrade to `process`
- [x] 3.2 `process` tier path: today's code, unchanged — nono profile,
      fd-inheritance, `SessionBackend = Local`
- [x] 3.3 `hardened` tier path: delegate bundle synthesis and sandbox
      start to the resolved backend (gVisor's implementation lives in
      `add-gvisor-backend`'s own tasks); start the SSH/control listener
      host-side directly — no fd-inheritance/self-restriction dance,
      since the host-side control process is not the trust boundary at
      this tier (the backend's own sandboxing is)
- [x] 3.4 `src/lifecycle/status.rs`: add a tier/backend field to
      `SandboxStatus` so `status` prints `isolation: process` or
      `isolation: hardened (<backend>/<platform>)`
- [x] 3.5 `doctor` in `src/bin/devcroft.rs`: add a generic
      "hardened-tier availability" line (`[WARN]` if no hardened backend
      is available, noting it's Linux-only), separate from any
      backend-specific probe a concrete backend change adds

## 4. Policy: tier-target dispatch

- [x] 4.1 Confirm `policy::compile`, `render`, and `why` remain fully
      tier-agnostic (already true today — they operate on
      `CompiledPolicy` before any backend-specific projection); add a
      regression test asserting `render`/`why` output for a given
      manifest is identical regardless of which tier compiled it
- [x] 4.2 Add the per-tier projection dispatch point in `up` (nono
      profile for `process`; a backend-specific projection for
      `hardened`, implemented by the concrete backend change) — this
      task only wires the dispatch, not a projection itself

## 5. Docs

- [x] 5.1 Amend CLAUDE.md's "SSH lives inside the boundary" invariant to
      be explicit about the tier split: `process` tier — the keeper
      embeds russh inside the restricted process tree, as today;
      `hardened` tier — the SSH/control server runs host-side and
      dispatches sessions through the backend's exec-into primitive,
      still only a 0600 unix socket in a 0700 dir, still never binding
      TCP. The access boundary was always the socket's filesystem
      permissions, not the process's physical location.
- [x] 5.2 Cross-tier SSH parity test backing the `ssh` delta's "Client
      cannot tell the difference" scenario — the same client workflow
      against both tiers, asserted identical
      (`tests/hardened_tier_ssh_parity.rs`): an `exec` round trip plus an
      SSH client-key handshake through `devcroft proxy`, run once against
      a real `process`-tier sandbox and once against a real
      `hardened`-tier sandbox, with the results (`stdout`, exit code, SSH
      auth success) asserted `==`. Gated on both tiers' real tooling
      (self-skips otherwise, same convention as `add-gvisor-backend`
      task 4.3); confirmed live to self-skip in this devcontainer for the
      same `runsc` userns reason task 4.3's own test does — see
      `add-gvisor-backend` task 10.3
- [x] 5.3 `openspec validate --all` passes with this change's `tasks.md`
      added (currently 5/5 passing)
