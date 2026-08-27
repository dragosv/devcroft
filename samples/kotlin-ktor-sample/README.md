# kotlin-ktor-sample

A hello-world HTTP server using [Ktor](https://ktor.io) — JetBrains' own
web framework, and the natural choice over Spring Boot's much heavier
dependency graph for a "simple" sample. It covers Kotlin/JVM via Gradle,
alongside [nix-go-sample](../nix-go-sample/) for Go and
[nix-flake-sample](../nix-flake-sample/) for Rust: three languages, one
provider, no per-language machinery in devcroft.

> **This sample was built for a tier that no longer exists.** It was
> `gvisor-kotlin-sample`, and its original point was that
> `[sandbox].isolation = "hardened"` was just another manifest key —
> nothing about `up`, `exec` or `status` changed shape because of it.
> `remove-gvisor-backend` deleted that tier (Landlock cannot mediate
> `mount()`, which `runsc` requires, so the two could never be stacked;
> see `docs/decisions.md`). The manifest key is gone from this file and
> the sample runs on the one remaining tier like everything else.
>
> What survives is the half that was never about gVisor: a JVM build under
> a devcroft sandbox, and the JVM-specific frictions below, which are
> unchanged.

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
`gradle build` succeeds and produces `build/libs/kotlin-ktor-sample-0.1.0.jar`,
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

## The listen-socket gap, hit directly — and no tier was going to fix it

Same finding [nix-go-sample](../nix-go-sample/) already documents:
`network.default = "deny"` denies `bind`/`listen` outright, loopback
included, so `devcroft.toml` here sets `network.default = "allow"` as
the documented workaround.

What was specific to *this* sample is now a piece of history worth
keeping. It existed to demonstrate the `hardened` tier, and an early
internal draft of `add-gvisor-backend` assumed gVisor's per-sandbox
netstack would close this gap for free there. It would not have: `runsc`
rejects `--network=sandbox` under `--rootless`, and devcroft runs
unprivileged by design, so that tier shared the host's network namespace
exactly as `process` does. See
[docs/decisions.md](../../docs/decisions.md), "Rejected (for now):
non-rootless gVisor for netstack", for the measurements. The tier is gone
now, which settles the question a second way: this is a network-policy
gap, and only a network-policy change closes it.

## What's verified

Building and running the Kotlin/Ktor/Gradle project itself — `nix
develop`'s dependency resolution, the generated start script, both HTTP
endpoints — was verified directly, through a real `devcroft up`/`exec`
cycle on the process tier.

What was never verified is the thing this sample was named for:
`isolation = "hardened"` actually starting a gVisor sandbox for this
project. This devcontainer had no `runsc` on `PATH` by default and,
more fundamentally, could not create unprivileged user namespaces at all
(`unshare --user` fails `EPERM`). That gap is now permanent and harmless
— `remove-gvisor-backend` deleted the tier, so there is nothing left
unverified here. The sample kept everything that was actually about the
JVM.

## Try it

```sh
cd samples/kotlin-ktor-sample
devcroft up
devcroft exec -- gradle --no-daemon installDist
devcroft exec -- sh -c './build/install/kotlin-ktor-sample/bin/kotlin-ktor-sample & sleep 1; curl localhost:8080/; curl localhost:8080/health; kill %1'
devcroft status
devcroft policy --render                  # shows the nix store grant with origin provider:nix
devcroft down
```

`gradle installDist` needs no network at all — dependency resolution
already happened once, host-side, in `flake.nix`'s `shellHook`, before
the sandbox restriction applies.
