# gvisor-kotlin-sample

A hello-world HTTP server using [Ktor](https://ktor.io) — JetBrains' own
web framework, and the natural choice over Spring Boot's much heavier
dependency graph for a "simple" sample — demonstrating two things at
once: that the `hardened` isolation tier (`add-hardened-tier` /
`add-gvisor-backend`) is language-agnostic the same way the nix provider
already is ([nix-go-sample](../nix-go-sample/) covers Go,
[nix-flake-sample](../nix-flake-sample/) covers Rust; this one covers
Kotlin/JVM via Gradle), and that `[sandbox].isolation = "hardened"` is
just another manifest key — nothing about `up`, `exec`, or `status`
changes shape because of it.

## What it is

```sh
GET /         -> {"message":"hello from a devcroft sandbox"}
GET /health    -> ok
```

`flake.nix`'s `devShells.<system>.default` installs `jdk21` and `gradle`
from nixpkgs, pinned by `flake.lock`'s locked nixpkgs revision —
closure-level reproducibility for the toolchain itself, the same
guarantee every other sample in this directory gets from the same
mechanism. There is no second, per-dependency lockfile the way
`Cargo.lock`/`go.sum` give the Rust and Go samples: Gradle's own module
cache is content-addressed by checksum already, just keyed by this
project's build script rather than a separate committed artifact — see
`flake.nix` for how the cache gets warmed host-side, before the sandbox
restriction, the same two-phase shape either way.

## Three real problems this sample hit, and how they were fixed

All three were found by actually building and running this sample — nix
build, then a real `devcroft up`/`exec` cycle through the process tier
(the only one this devcontainer can currently exercise end to end; see
"Hardened tier: what's verified and what isn't" below) — same standard
the rest of this repo holds itself to.

**The plain `application`-plugin jar has no runnable manifest.**
`gradle build` succeeds and produces `build/libs/gvisor-kotlin-sample-0.1.0.jar`,
but `java -jar` on it fails with `no main manifest attribute` — the
default `jar` task packages only this project's own compiled classes,
not a fat jar with a `Main-Class` entry and every dependency bundled in.
Fixed by using `gradle run` or `gradle installDist` (this sample's
`README` uses the latter) instead, which is also the idiomatic way an
`application`-plugin project is meant to be run — reaching for a
shadow/fat-jar plugin would have been solving a problem this project
doesn't actually have.

**Netty/Ktor log at DEBUG by default, with no config.** logback-classic
ships no default configuration, so `gradle run`'s output was almost
entirely `io.netty.*` buffer-allocator and resource-leak-detector
internals — the one line that's actually useful
(`Responding at http://0.0.0.0:8080`) was buried in noise. Fixed with a
minimal `src/main/resources/logback.xml`: root level `INFO`, a plain
one-line-per-event pattern.

**The Gradle daemon has no session to persist across.** Gradle's daemon
exists to amortize JVM startup cost across *repeated invocations against
the same on-disk state*, which is exactly what a normal dev machine
gives it and exactly what a `devcroft exec` session does not: each one
is its own fresh process group, torn down with the session. Left
running, a daemon started by one `exec` would just be an orphaned
background JVM once that session ends — not wrong, exactly, but not
buying anything either. `gradle.properties` sets
`org.gradle.daemon=false`; every command below also passes
`--no-daemon` explicitly, since the properties file alone doesn't stop
a daemon a *previous*, unpatched invocation already started.

## `$PWD`, not `self` — same reasoning as the other nix samples

Gradle's default `GRADLE_USER_HOME` (`$HOME/.gradle`) is outside the
project root, so a devcroft-sandboxed session correctly denies writing
there — same shape as nix-go-sample's `GOPATH` and nix-flake-sample's
`CARGO_HOME`. Redirected to `$PWD/.gradle-home` in `flake.nix`'s
`shellHook`, which then runs `gradle --no-daemon build` to resolve and
cache every dependency (Ktor, Netty, logback, and their full transitive
graphs) before the sandbox restriction applies — `devcroft exec --
gradle ...` never needs network access at session time as a result,
confirmed directly against a real `devcroft up`/`exec` cycle.

## The listen-socket gap, hit directly — and not fixed by this tier either

Same finding [nix-go-sample](../nix-go-sample/) already documents:
`network.default = "deny"` denies `bind`/`listen` outright, loopback
included, so `devcroft.toml` here sets `network.default = "allow"` as
the documented workaround. What's specific to *this* sample: it exists
to demonstrate the `hardened` tier, and an earlier internal draft of
`add-gvisor-backend` assumed gVisor's per-sandbox netstack would close
this gap for free at that tier. It doesn't — `runsc` rejects
`--network=sandbox` under `--rootless`, and devcroft runs unprivileged
everywhere by design, so the hardened tier shares the host's network
namespace exactly like `process` does. See
[docs/decisions.md](../../docs/decisions.md), "Rejected (for now):
non-rootless gVisor for netstack", for the full reasoning. This sample's
own `devcroft.toml` carries the same workaround and the same caveat, not
a tier-specific fix.

## Hardened tier: what's verified and what isn't

Building and running the Kotlin/Ktor/Gradle project itself — `nix
develop`'s dependency resolution, the generated start script, both HTTP
endpoints — was verified directly, including through a real
`devcroft up`/`exec` cycle (temporarily under `isolation = "process"`,
since that is the only tier this devcontainer can currently exercise
end to end). What was **not** verified is `isolation = "hardened"`
itself actually starting a gVisor sandbox for this project: this
devcontainer has no `runsc` on `PATH` by default and, more
fundamentally, cannot create unprivileged user namespaces at all
(`unshare --user` fails `EPERM`) — the exact wall `add-gvisor-backend`'s
own work already hit and documented, independent of this sample.
`devcroft doctor` reports this precisely when it applies. Once a
devcontainer rebuild unlocks a working `runsc`, this sample is what to
point `devcroft up` at to exercise the hardened tier against something
heavier than a hello-world — a real (if small) JVM process tree, a real
Gradle daemon-adjacent build tool, real Netty event-loop threads.

## Try it

```sh
cd samples/gvisor-kotlin-sample
devcroft up
devcroft exec -- gradle --no-daemon installDist
devcroft exec -- sh -c './build/install/gvisor-kotlin-sample/bin/gvisor-kotlin-sample & sleep 1; curl localhost:8080/; curl localhost:8080/health; kill %1'
devcroft status                           # isolation: hardened (gvisor/<platform>) once runsc works here
devcroft policy --render                  # shows the nix store grant with origin provider:nix
devcroft down
```

`gradle installDist` needs no network at all — dependency resolution
already happened once, host-side, in `flake.nix`'s `shellHook`, before
the sandbox restriction applies.
