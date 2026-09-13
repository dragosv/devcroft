# Feature request for nono: scope `process-exec` to the read grants on macOS

Draft for <https://github.com/nolabs-ai/nono>, in the fields of its
`feature_request.yml` template. Verified against **nono 0.77.0** (the
rule is unchanged from 0.74.0, where devcroft first measured it; it dates
from the initial commit); measurements on macOS 15.7.4 / aarch64-darwin.
Not yet filed — filing is the maintainer's to do, in their own name;
strip this preamble.

**Relationship to #1865** (*"nono allows exec of a binary it denies read
on MacOS breaking dyld / Security.framework"*): same root fact, opposite
direction. That issue wants exec to imply read on the target so `gh`
stops failing with a bogus TLS error; this asks that no-read imply
no-exec. Tying `process-exec` to the read set gives both — the incoherent
"exec permitted, own image unreadable" state cannot arise, and the
failure moves to `execve` with `EPERM`, which is #1865's option 2. Filing
this as a comment on #1865 rather than a new issue is reasonable; the
maintainers' contribution policy asks that an agent-authored PR be
preceded by intent and approach disclosed in the issue thread, and an
implementation exists (below).

---

## What problem are you trying to solve?

On macOS the generated Seatbelt profile allows `process-exec*`
unconditionally (`src/sandbox/macos.rs`, `generate_profile`, since the
initial commit) while deriving `file-read*` from the capability set.
Seatbelt checks the two independently, so a binary at a path nothing
granted **executes** — with its own image unreadable, which is #1865's
symptom. Linux does not behave this way: the Landlock backend maps
`AccessMode::Read` to `ReadFile | ReadDir | Execute` and nothing else
carries `Execute`, so an ungranted path cannot be executed at all.

Measured live in a devcroft sandbox (a devbox closure, the read set
being the closure's store path, the project, `/tmp` and a handful of
`/dev` entries — no `/bin`, `/usr/bin`):

- `/bin/echo`, `/bin/ls`, `/usr/bin/gcc`, `/usr/bin/clang` all **run**.
- `ls -l /usr/bin/gcc` from the same sandbox: `Operation not permitted`.

So "the sandbox runs only what it was granted" — true on Linux — is false
on macOS for any consumer that does not grant the system executable
directories. nono-cli's own `system_read_macos` group grants `/bin`,
`/sbin`, `/usr/bin`, `/usr/local/bin`, `/opt`, `/Applications`, `/nix`
and more, described as *"macOS system paths required for executables to
function"* — which is exactly the right place for "which host executables
may run" to be decided, and under the change below it would go on
deciding it, unchanged. The unconditional rule does not implement that
policy; it duplicates it for granted paths and contradicts it for
ungranted ones.

## What would you like to see?

A mode on `CapabilitySet` that scopes `process-exec*` to the read set,
opt-in, default unchanged:

```rust
#[derive(Default)]
pub enum ProcessExecMode {
    #[default]
    Unrestricted,   // today's (allow process-exec*)
    ReadGranted,    // (allow process-exec* (subpath|literal "...")) per Read/ReadWrite grant,
                    // both spellings, in place of the unconditional rule; no-op on Linux
}
caps.set_process_exec_mode(ProcessExecMode::ReadGranted);
```

Implemented, with tests, on
<https://github.com/dragosv/nono/tree/macos-process-exec-mode> (three
files; `ProcessExecMode` beside `ProcessInfoMode`, the match in
`generate_profile`, profile-text tests, and a forked live test: a copy of
`/usr/bin/true` inside the one granted directory runs, `/bin/sh -c 'exit
7'` is refused at `execve` with `EPERM`, and the same grants under
`Unrestricted` run `/bin/sh` — the control that shows the mode is what
does it). 828 library tests pass; clippy strict is clean.

Two design points, both measured rather than argued:

- **It has to be in the one profile.** The natural shape would be a
  macOS `Sandbox::restrict_execute`, stacked after `apply()` like the
  Linux one. It cannot be: `sandbox_init()` from an already sandboxed
  process returns `EPERM`, in either order (permissive-then-narrow and
  narrow-then-permissive). Seatbelt does not stack profiles.
- **Scoped exec works.** `(deny default)` with `(allow process-exec*
  (subpath "<dir>"))`: a Mach-O inside `<dir>` runs, `/bin/echo` and
  `/usr/bin/true` fail at `execve` with exit 127 from the shell.

Not asked for: a change to the default. Making `ReadGranted` the default
would be the principled end state — it is what Linux already does — but
it is a behaviour change for every macOS consumer and is the
maintainers' call, not this request's.

## What have you tried instead?

`CapabilitySet::platform_rule`, which is where devcroft's fix lives
today: `(deny process-exec*)` followed by one `(allow process-exec* ...)`
per read grant, both spellings. It works because the library appends
platform rules after its own and Seatbelt is last-rule-wins — the
library's own comment on that ordering cites #970. Devcroft derives the
rules from `caps.fs_capabilities()` so they cannot drift from the read
set, and its end-to-end test asserting the host gcc is refused now runs
on both platforms.

What the raw form cannot do: appear in the capability model. A consumer
has to know that platform rules are emitted last, that Seatbelt is
last-rule-wins, that the library emits both spellings for a symlinked
grant and escapes paths a certain way — and reproduce all of it. A typed
mode makes the guarantee the library's to keep.

## How is this blocking you?

It would be a nice improvement but I have a workaround.

## Additional context

Consumer is devcroft (<https://github.com/dragosv/devcroft>), which links
`nono` for its process tier and deliberately excludes the
`system_read_*` groups — its sandboxes are meant to run only what a
pinned Nix closure provides, so "host toolchain denied" is a property it
tests for. The macOS half of that test was gated off with a pointer to a
published gap until the raw-rule fix; it is un-gated now, and passes with
the rules and fails without them.
