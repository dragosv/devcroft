# Tasks — add-service-compatibility-matrix

## 0. Measure before writing the harness

> This group exists because the design above is built on six
> measurements, and everything past them is inference. The repo's rule
> holds: no code until the questions are measured, and a measurement that
> contradicts the design changes the design rather than being worked
> around.

- [ ] 0.1 **Does `services.<name>.enable = true` alone yield processes?**
      Run `devenv eval processes` on a minimal project for four services
      chosen to span the range — `redis` (probe, trivial), `postgres`
      (probe, stateful, known-failing), `nginx` (no probe, config-heavy),
      `mailpit` (no probe, trivial). Record how many processes each
      contributes; the mapping is per-process, not per-service, and the
      matrix's row shape depends on which.
- [ ] 0.2 **Does process-compose report readiness in the JSON devcroft
      already parses?** `ServiceState::from_json` reads `status`,
      `is_running`, `exit_code`. Check the same payload for a readiness
      field against a service with a probe. This decides whether the
      `ready` column exists at all (design D4).
- [ ] 0.3 **What does `up` do when a service's closure will not build?**
      Pick a service whose package is unavailable on darwin (if 0.1's
      sample has none, find one) and record the exact failure: which
      layer, which exit code, what text. `not-measured` must be
      detectable from that, and must not be confused with a service that
      built and failed.
- [ ] 0.4 **What does one case cost?** Time a full cycle — generate,
      `up`, observe, `down` — for a warm-store service and a cold-store
      one. Forty-two times the cold number is the run's real budget, and
      if it is hours that belongs in the README's instructions rather
      than being discovered by the first person who runs it.
- [ ] 0.5 **What does a case leak?** After one full cycle, check for:
      processes surviving `down`, `/tmp/devenv-<hash>` directories, nix
      GC roots under the project, and listening sockets. Each one found
      is something the harness must clean between cases, and the `/tmp`
      directory in particular is where this session's postgres failure
      lived.
- [ ] 0.6 **Confirm the socket-path bound.** Compute the longest project
      path the harness will generate plus
      `/.devcroft/<name>/services.sock` and check it against 103 bytes.
      `devenv-services-sample` produced 112 and `up` refused; a generated
      path under a temp root is longer, not shorter.

## 1. Enumerate the catalog

- [ ] 1.1 Read the devenv input's rev and `dir` from the project's
      `devenv.lock`. Fail naming the file if the node is absent rather
      than falling back to a default rev — a matrix measured against an
      unpinned upstream is a matrix about nothing.
- [ ] 1.2 Resolve that rev to a store path with `nix flake prefetch`, and
      list `<store>/src/modules/services/*.nix`. Note the two traps
      measured in design D1: prefetch ignores `?dir=`, and
      `trafficserver` has a directory beside its `.nix` file.
- [ ] 1.3 Assert the count is non-zero and that every name is a plausible
      module name. A prefetch that silently returns an empty or
      restructured tree must stop the run, not produce a 0-row matrix.
- [ ] 1.4 Accept a subset argument, validated against the enumerated
      catalog — an unknown name fails naming it (spec: "A named service
      is not in the catalog").

## 2. Run one case

- [ ] 2.1 Generate a project per service: `devenv.nix` enabling only that
      service plus `pkgs.process-compose`, a `devenv.yaml` and lock
      copied from the enumeration source so every case measures the same
      upstream, and a `devcroft.toml`.
- [ ] 2.2 Derive a sandbox name within 0.6's bound, and assert the bound
      rather than assuming it.
- [ ] 2.3 `up`, capturing stdout, stderr and the exit code — the exit
      code carries the layer (3 = provider) that classification depends
      on.
- [ ] 2.4 Observe the services: names, health, pids, and readiness if 0.2
      says it exists. Poll to a bound rather than sleeping once; a
      database that takes eight seconds to open its socket must not be
      recorded as failed because the harness looked at four.
- [ ] 2.5 `down`, then **verify teardown by absence** — no surviving
      process, no held port. A case that cannot be verified stops the run
      (spec: "A case cannot be torn down").
- [ ] 2.6 Clean whatever 0.5 found leaking, between cases and not at the
      end.

## 3. Classify the outcome

- [ ] 3.1 Implement the six outcomes of design D3, each from a specific
      observable: `refused` from a layer-`provider` failure naming a
      field; `not-measured` from 0.3's build-failure signature; `failed`
      from a non-zero service exit or a probe that never passed;
      `needs-config` from the service's own output asking for something
      absent; `ready`/`started` from 2.4.
- [ ] 3.2 **`needs-config` must not be a catch-all.** It is claimed only
      on evidence in the service's own output. Anything unclassifiable is
      `failed` with its text attached — an honest unknown attributed to
      devcroft beats a flattering guess attributed to the user.
- [ ] 3.3 Capture the reason: devcroft's error for `refused`/`not-measured`,
      the tail of `.devcroft/<name>/services.log` for `failed`. This is
      the deliverable (design D7), not a decoration on it.
- [ ] 3.4 Group identical reasons so one cause across several services
      reads as one cause (spec: "Two services fail the same way").

## 4. Publish the matrix

- [ ] 4.1 Write `docs/service-matrix.md`: platform, devenv rev,
      process-compose version, devcroft commit, date, then the table.
- [ ] 4.2 Mark a partial run partial, and refuse to overwrite a full
      matrix with a subset's rows.
- [ ] 4.3 Fail the run if a catalog entry produced no row (spec: "A
      service module the enumeration finds has no row").
- [ ] 4.4 Header the document as generated, naming the script — the same
      contract `THIRD-PARTY-LICENSES.md` carries, for the same reason.

## 5. Run it for real

- [ ] 5.1 Full run on aarch64-darwin. This is the point of the change;
      everything above is scaffolding for this line.
- [ ] 5.2 **Re-run it and diff.** A matrix that is not reproducible on
      the same host and versions is measuring the harness, not the
      services (spec: determinism).
- [ ] 5.3 Check the two standing predictions and record whichever way
      they go: `refused` is expected empty (no module uses a refused
      field) and `started` is expected to be the majority (25 of 42
      declare no probe). A wrong prediction here is a finding, not an
      error to quietly fix.
- [ ] 5.4 Re-measure `postgres` specifically against the now-fixed `/tmp`
      grant. `docs/known-gaps.md` records its `shmget` EPERM with the
      cause **not** isolated, and this run either reproduces it with more
      context or shows the earlier diagnosis was measuring the missing
      grant. Either way the entry gets updated from data.

## 6. Fold the result back into what is published

- [ ] 6.1 Update `docs/known-gaps.md` from the matrix — service failures
      currently described one at a time become one entry with a table
      behind it.
- [ ] 6.2 Update the README's Status section only if the matrix changes
      what is claimed there. It is deliberately short; a link is likely
      the whole edit.
- [ ] 6.3 Document the run in `docs/implementation-log.md`, including
      whatever turned out to be wrong — the log's job.
- [ ] 6.4 If 0.2 found readiness to be reportable and it was surfaced,
      say so in the `services` capability's own terms: devcroft now
      reports a state it previously asked its supervisor to wait for and
      could not observe.
- [ ] 6.5 Note in `CLAUDE.md`'s working-commands section how to run it
      and that it is slow and not CI — the file already carries the
      licence generator's equivalent warning.

## 7. Linux

- [ ] 7.1 Run the same matrix on Linux. The whole motivation is that
      service behaviour is platform-dependent, and a darwin-only matrix
      proves that only half.
- [ ] 7.2 Publish both columns in one table. A reader deciding whether to
      develop on a Mac wants the comparison in one place, and the
      services that work on one platform and not the other are the most
      valuable rows in the document.
