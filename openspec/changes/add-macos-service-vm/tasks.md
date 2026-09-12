# Tasks — add-macos-service-vm

## 0. Measure before building any of it

> Everything below group 0 is written from a spike that has not run. The
> group exists because the last three macOS assumptions in this project
> were each wrong in a way only a measurement caught.

- [x] 0.1 **Does `fleet::netns` work unchanged inside a Lima guest?**
      **Yes — the gate passes, and the rest of these tasks are live.**
      Measured on Lima 2.1.0, Ubuntu 25.10, kernel 6.17.0-14-generic,
      aarch64, 4 CPUs / 8 GiB.

      Confirmed twice, deliberately, because the first way could have
      been a property of `unshare(1)` rather than of the call devcroft
      makes:

      1. `unshare(CLONE_NEWUSER | CLONE_NEWNET)` **in-process**, from a C
         probe compiled in the guest — the exact syscall
         `fleet::netns::enter_network_namespace` issues. Returns 0.
      2. devcroft's own probe, through `doctor`:
         *"namespaces: available on this host — sandboxes with
         `network.default = "deny"`, no `network.allow`, and services or
         `network.ports` each get their own port table, and every sandbox
         gets its own filesystem view (mount isolation)."*

      **The reason to insist on the second reading**: Ubuntu has
      restricted unprivileged user namespaces since 24.04, and the
      restriction is *on* in this guest —
      `kernel.apparmor_restrict_unprivileged_userns = 1`, with
      `kernel.unprivileged_userns_clone = 1` and
      `user.max_user_namespaces = 31183`. It permits the call anyway,
      because the gate is an AppArmor profile applied per binary rather
      than a global switch. So this result is **Ubuntu-25.10-specific and
      must not be generalized**: a distro or a policy that confines the
      calling binary differently can still refuse, and the guest image is
      therefore part of the design, not an implementation detail.

      Recorded alongside, because it bears on `add-mount-isolation` and
      `add-macos-unix-socket-scoping` rather than on this change:
      **Landlock V6** is available in the guest, with *Basic filesystem
      access control, Refer, Truncate, TCP network filtering, Device
      ioctl filtering, and Signal and abstract UNIX socket scoping*.

      Also established, and worth keeping because it is the expensive
      part of any future guest: the provisioning sequence that makes a
      guest a *usable* devcroft Linux. `nix` alone is not enough —
      `doctor` correctly reported `[FAIL] provider: nix found but flake
      commands are rejected` until `experimental-features = nix-command
      flakes` was added to `/etc/nix/nix.conf`. With that, all four
      providers report `[PASS]`: flox 1.16.0, nix 2.31.5 (flox's own,
      which shadows a separately installed 2.35.2), devbox 0.18.0,
      devenv 2.2.2. Note devbox is **0.18.0** in the guest against
      0.16.0 on the macOS host — a version skew to control for before
      comparing any devbox result across the two.

      **Not measured: the test suite on Linux.** The run was started and
      the host ran out of disk before its output could be read; its
      exit code is not evidence of anything. The guest was deleted to
      recover space. Tasks 0.2 and 0.3 still need a guest.
- [ ] 0.2 Two sandboxes, two namespaces, same declared port, both serving —
      end to end in the guest. This is the property the change exists for;
      measure it before designing around it.
- [ ] 0.3 **Price the second resolution.** Resolve one project's services for
      `aarch64-linux` in the guest and record: wall-clock for a cold store,
      wall-clock warm, and store size. D4 assumes a shared store makes the
      second and later worktrees cheap — verify.
- [ ] 0.4 Measure the relay's cost on a real client: a `psql` round-trip over
      the unix-socket path versus a direct connection, so the endpoint story
      is quoted with a number rather than an adjective.
- [ ] 0.5 Confirm the guest survives what a laptop does to it — sleep, wake,
      network change — and what `status` reports across each.

## 1. The seam

- [ ] 1.1 A devcroft-owned trait for *where* services run, distinct from
      `Supervisor`, which is about *which* supervisor (D6). Name the
      environment each refers to, so `Supervisor::binary()` stops being
      ambiguous once two environments exist.
- [ ] 1.2 Lima behind that seam; no Lima vocabulary above it.
- [ ] 1.3 Record the keeper-parentage argument in the code, not only here:
      Lima runs services, the keeper stays a direct child of `up` (D7).

## 2. The guest

- [ ] 2.1 Lazy start, shared across sandboxes, owned by devcroft.
- [ ] 2.2 Nix + SSH only — no flox, devenv or devbox in the image (D3).
- [ ] 2.3 Shared Linux Nix store.
- [ ] 2.4 **No project-source mount** (D2), and a test that asserts the
      absence, since a later "just mount it" change would otherwise pass
      silently.

## 3. Per-sandbox isolation in the guest

- [ ] 3.1 A network namespace per sandbox, reusing `fleet::netns`.
- [ ] 3.2 A supervisor instance per sandbox inside its namespace.
- [ ] 3.3 A data volume per sandbox.
- [ ] 3.4 The negative that makes the rest mean something: two sandboxes must
      not share a namespace, and the attempt must fail loudly.

## 4. Reaching services from the host

- [ ] 4.1 Per-sandbox endpoint: unix socket where the client supports one,
      allocated loopback port otherwise (D5).
- [ ] 4.2 Inject the endpoint into the sandbox environment; show it in
      `status`.
- [ ] 4.3 Endpoint lifetime is the sandbox's — created at `up`, gone at
      `down`, never left behind after a crash.

## 5. Environment resolution

- [ ] 5.1 Resolve services for the guest platform, recorded as a **second**
      resolution with its own fingerprint (D4).
- [ ] 5.2 Staleness answers for both independently.
- [ ] 5.3 A service that cannot resolve for Linux fails at `up` naming the
      service — never starts a partial set.

## 6. Tests

- [ ] 6.1 Two worktrees, same declared port, both serving. Self-skips where
      no guest runtime exists.
- [ ] 6.2 The control: with services on the host (Linux, or macOS without
      the guest), behaviour is unchanged.
- [ ] 6.3 Data volumes survive `down` and are deleted by `rm`.
- [ ] 6.4 A stopped guest reports services unavailable rather than healthy.
- [ ] 6.5 No sandbox's sources are reachable from another sandbox's service.

## 7. Say what changed, and what did not

- [ ] 7.1 README: services may run in a shared Linux guest on macOS; the
      sandbox's own code does not, and gains no boundary from it.
- [ ] 7.2 `docs/known-gaps.md`: update the port-collision entry — the macOS
      half stops being "no answer" and becomes "an answer with a named
      cost", with `forward = ["PORT"]` still the zero-cost path for services
      that read an env var.
- [ ] 7.3 `docs/decisions.md`: record the four rejected mechanisms with their
      measurements (aliases, `SO_REUSEPORT`, `pf`, NetworkExtension) so none
      is proposed again.
- [ ] 7.4 Close `add-port-allocation`, whose surviving scope this replaces.
- [ ] 7.5 State the platform drift honestly: code builds on Darwin, services
      run on Linux. For Postgres and Redis that is the point; for a service
      that is the project's own code it is a complication the project owns.
