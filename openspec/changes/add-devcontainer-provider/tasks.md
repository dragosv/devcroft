## 0. Measure before writing provider code

Every item here is a probe with a number or a yes/no, recorded in
`design.md` (or a `docs/` note it links) before task 1 starts. A
measurement that contradicts a decision changes the decision, not the
measurement.

- [ ] 0.1 Materialization: `docker create` + `docker export` of
      `debian:stable-slim@sha256:…` and `alpine@sha256:…` into a user-owned
      directory. Record wall time, on-disk size, file count, and what the
      tar contains that must be skipped (device nodes, setuid bits,
      `/proc`/`/sys` placeholders). Confirm no process from the image ran
      (an image whose entrypoint writes a marker).
- [ ] 0.2 Same with `podman`, rootless. Record the verb differences and
      whether the exported tree is identical.
- [ ] 0.3 **The decision D3 rests on:** in a probe binary, grant
      `<rootfs>` read-only with nono in a parent, then in a child
      `unshare(CLONE_NEWNS)` + pivot into a view whose root is `<rootfs>`,
      and `execve("/bin/ls")`. It must run; `/bin/ls` of the *host*
      (visible before the pivot) must be refused. Then the inverse: the
      grant made *after* the pivot by path (`/`) must not be needed. Record
      the result for both images.
- [ ] 0.4 The keeper's own libc: exec the host-built `devcroft` binary
      from inside a view whose `/lib` is alpine's. Expected: loader
      failure. Record the exact error — it is the sentence D3's "cannot be
      `/` for the keeper" cites.
- [ ] 0.5 `up` latency on a 1–2 GB rootfs versus a flox sandbox on the
      same host: one bind, one rule, or something scales with file count?
      Record both numbers.
- [ ] 0.6 `docker inspect` `Config.Env` of the two images and of
      `mcr.microsoft.com/devcontainers/rust`: what `PATH` is, whether
      `shell.rs`'s resolver finds `/bin/sh` inside the rootfs under the
      declared-grant guard, and what `Config.User` is for each (the
      `devcontainers/*` images say `vscode` — this is where D4's refusal
      bites, and the measurement decides whether "refuse" or "run as
      root-in-image" is the right first cut).
- [ ] 0.7 The reference `devcontainer` CLI with `--skip-post-create`:
      what it runs on the host, for the open question on `features`.
      Record; do not act.

## 1. Provider

- [ ] 1.1 `src/provider/devcontainer.rs`: locate the file, parse it with
      an unknown-field-tolerant JSON reader, apply D6's honored/ignored/
      refused table, each refusal a `ProviderError` naming the field.
- [ ] 1.2 Digest resolution: `image@sha256:…` used as is; a bare tag
      resolved once via the runtime and recorded in
      `.devcontainer/devcroft.lock`; the lock wins thereafter until
      `--recreate`.
- [ ] 1.3 Materialization into `$XDG_CACHE_HOME/devcroft/rootfs/<digest>/`
      (D2), skipping what 0.1 found, then made read-only; idempotent on a
      present digest; a partial extraction (crash mid-way) is detected by
      a completion marker and redone.
- [ ] 1.4 Environment from `Config.Env` as the diff against
      `canonical_base_env`; `Resolution.read_only_grants` is the one
      rootfs directory; `services: Unsupported` (no service concept, like
      nix).
- [ ] 1.5 `ProviderKind::Devcontainer`, `validate.rs` list, fingerprint =
      hash(devcontainer.json) + recorded digest, `host_can_materialize_images()`
      probe alongside `host_can_build_nix_closures()`.
- [ ] 1.6 Platform gate: `resolve` on non-Linux fails at layer `provider`
      naming the VM route.
- [ ] 1.7 Tier `image` — the third variant of the `Tier` type
      `add-swift-provider` introduces, or the type itself if this lands
      first; printed at `up` and in `status`.
- [ ] 1.8 `postCreateCommand` → `post_create`, `postStartCommand` →
      `post_start` (design D8): parse string/array/object forms into the
      hook list the keeper already runs (`lifecycle::hooks`), so the
      existing once-per-creation/once-per-start machinery and the
      existing hook failure path are reused unchanged; the keeper-layer
      failure message gains the rootfs-shared sentence when the failing
      write is under the rootfs grant.

## 2. Session view

- [ ] 2.1 `fleet::mount`: a second constructor, `construct_session_view(rootfs, project_root, …)`,
      whose root is the rootfs (read-only bind) with the project root,
      private `/tmp`, `/proc`, `/dev` (private devpts included) and the
      control/proxy sockets bound in — reusing every helper
      `construct_view` has, adding none that binds a host path.
- [ ] 2.2 The keeper spawns each session of a `devcontainer` sandbox
      through `unshare(CLONE_NEWNS)` + the session view in `pre_exec`,
      *after* its own Landlock restriction (inherited), so 0.3's property
      is what makes the session's `/usr` allowed.
- [ ] 2.3 `shell.rs`: the resolved shell is inside the rootfs grant and
      recorded in `Meta.shell` by its **rootfs-relative** path (`/bin/sh`),
      which is what the session sees; document that this is the first
      provider where `Meta.shell` and the host path differ.
- [ ] 2.4 `policy --render` unchanged in mechanism: the rootfs grant
      renders by its host path; `status` gains one line saying sessions
      see it as `/`.

## 3. CLI

- [ ] 3.1 `init` detection and ranking (cli delta); the advice line names
      the tier and the closure alternative.
- [ ] 3.2 `doctor`: runtime present/usable/which, rootfs store writable,
      digest materialized; fixes named (cli delta's docker-group
      scenario).
- [ ] 3.3 `USAGE` unchanged (no new command); `tests/cli_help_and_version.rs`
      still passes.

## 4. Tests

- [ ] 4.1 Unit: the field table (each refused field → its message; each
      ignored field → silence; unknown → silence), digest parsing, lock
      precedence, fingerprint sensitivity.
- [ ] 4.2 Linux e2e, self-skipping without a usable runtime, against the
      two pinned images: `up`; `exec -- /usr/bin/env` shows the image's
      `PATH`; a rootfs binary runs; the host's `/usr/bin/gcc` is refused;
      `shell` gets a prompt from the rootfs's `sh`; `status` stale after
      editing the lock; second project on the same digest materializes
      nothing (assert on the store's mtime).
- [ ] 4.3 The entrypoint-marker test: an image built for the test whose
      entrypoint writes to a bind-mounted host path; after `up` the path
      is untouched.
- [ ] 4.4 Hooks: a `postCreateCommand` that writes a marker into the
      project ran exactly once inside (marker present after `up`, not
      duplicated by a second `up`); one that writes under `/usr` fails
      `up` at layer `keeper` with the sharing sentence and leaves the
      rootfs byte-identical (assert on a checksum of the store dir); one
      that dials an unallowed host fails naming `network.allow`.
- [ ] 4.5 macOS: `up` refuses with the platform message; `init` still
      detects and writes the manifest (a Linux teammate will run it).
- [ ] 4.6 CI: `e2e (devcontainer)` leg on `ubuntu-latest` (docker is
      preinstalled there), `DEVCROFT_TEST_PROVIDER=nix` for the fixture
      rows as the devenv leg does, blocking once green.

## 5. Documents

- [ ] 5.1 `docs/decisions.md` §1: the `image` tier defined beside the
      other two; the qualification test answered per criterion for this
      provider (3: the rootfs store; 4: `create`+`export`+`inspect`, no
      execution; 6: runtime present, digest resolvable). §2: "Features →
      run at build, in the container (next cut)"; the `remoteUser`
      rejection extended to `Config.User`.
- [ ] 5.2 `docs/known-gaps.md`: no GC of rootfs directories; `features`
      unsupported; Linux-only; `dockerComposeFile` deferred, with D6's
      mapping (sibling services as materialized rootfs + supervised
      process, names on loopback, named volumes as grants) recorded as
      the follow-up change's starting point. `docs/roadmap.md`: that
      follow-up placed after this change.
- [ ] 5.3 `CLAUDE.md`: provider count, the two-level view in the
      architecture invariants (one paragraph: keeper on host libc,
      sessions on the image's, Landlock by inode is what makes it one
      grant), and the rootfs store's location and why it is not under the
      data dir.
- [ ] 5.4 `README.md`: one row in the providers table; the tier's
      one-sentence definition; `samples/devcontainer-sample/` with an
      unmodified `devcontainer.json` from a real template.
- [ ] 5.5 `docs/roadmap.md`: where this sits relative to fleet — it is the
      adoption half; PID isolation and cgroups remain fleet's.
