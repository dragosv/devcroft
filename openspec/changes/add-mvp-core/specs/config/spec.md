# config Specification

## Purpose

Define, parse, and validate the declarative manifest (`devcroft.toml`) that
is the single source of truth for a sandbox: its identity, environment
provider, filesystem policy, network policy, and hooks.

## ADDED Requirements

### Requirement: Manifest discovery
The system SHALL locate the manifest by searching for `devcroft.toml` in the
current directory and its ancestors, stopping at the filesystem root or a
`.git` boundary, whichever comes first.

#### Scenario: Run from a subdirectory
- **WHEN** the user runs any subcommand from `<root>/src/deep/`
- **AND** `<root>/devcroft.toml` exists
- **THEN** that manifest is used and `<root>` is the sandbox project root

#### Scenario: No manifest found
- **WHEN** no `devcroft.toml` exists in the ancestor chain
- **AND** the subcommand requires one (`up`, `shell`, `exec`, `ssh`)
- **THEN** the command fails with exit code 2 and a message suggesting
  `devcroft init`

### Requirement: Manifest schema
The system SHALL accept a TOML manifest with sections `[sandbox]`, `[env]`,
`[filesystem]`, `[network]`, `[ssh]`, `[hooks]`, where only `[sandbox].name`
is mandatory and every other field has a documented default.

#### Scenario: Minimal manifest
- **WHEN** the manifest contains only `[sandbox] name = "myproj"`
- **THEN** validation succeeds with defaults: provider `flox`,
  filesystem allow `["."]`, network default `deny`, no hooks
- **AND** if no flox environment exists at `up`, the failure is at the
  provider layer with a hint to run `flox init` (see env-provider spec)

#### Scenario: Unknown key
- **WHEN** the manifest contains a key not in the schema
- **THEN** validation fails with the exact key path and closest valid
  alternative (typo suggestion), and exit code 2

### Requirement: Static environment variables
The system SHALL accept `[env] vars` as a table of static environment
variables injected into every session, applied AFTER provider resolution
so they can override provider-set values. Values are literal strings; no
host-environment interpolation is performed (that would leak
non-reproducible host state).

#### Scenario: Var overrides provider value
- **WHEN** the provider sets `FOO=a` and the manifest sets
  `vars = { FOO = "b" }`
- **THEN** sessions see `FOO=b`

#### Scenario: No host interpolation
- **WHEN** the manifest sets `vars = { TOKEN = "$HOST_TOKEN" }`
- **THEN** sessions see the literal string `$HOST_TOKEN`, and validation
  prints a one-time warning that interpolation is not supported

### Requirement: Name constraints
The system SHALL restrict `[sandbox].name` to `[a-z0-9][a-z0-9-]{0,31}`,
because the name becomes a hostname label (`<name>.devcroft`) and a state
directory component. **Every other source of a sandbox name — a CLI
positional argument, an SSH hostname, or any future caller —** SHALL be
held to the identical constraint before it is used to construct a state
path, because it becomes the identical state directory component
regardless of where it came from. The constraint is a property of what the
name is used *for*, not of which code path produced it.

#### Scenario: Invalid name
- **WHEN** `name = "My Project"`
- **THEN** validation fails naming the constraint and showing a suggested
  slug (`my-project`)

#### Scenario: CLI argument attempts path traversal
- **WHEN** an explicit name argument to any command (`rm`, `down`, `exec`,
  `status`, `logs`, `ps`, `policy`, `why`, `shell`, `ssh`) is not a valid
  slug — e.g. contains `/`, `..`, or is empty
- **THEN** the command fails at layer `config` (exit 2) naming the invalid
  value, before any path is constructed or any filesystem operation runs
- **AND** this holds even for a value that only resolves outside the state
  root after joining, not only for a value that is rejected by inspecting
  its characters in isolation

#### Scenario: SSH hostname attempts path traversal
- **WHEN** a client's SSH `ProxyCommand` invocation names a sandbox whose
  extracted name is not a valid slug
- **THEN** `proxy` refuses before constructing any state path, with the
  same validation `rm`/`down`/etc. apply to an explicit CLI argument

### Requirement: Filesystem policy validation
The system SHALL validate that every path in `filesystem.allow`,
`filesystem.read`, and `filesystem.deny` is either relative to the project
root or an absolute/tilde path, and SHALL reject rules that are provably
useless (deny of a path never granted). A project-relative entry SHALL
additionally be rejected if, once symlinks are followed, its real target
falls outside the project root — the manifest string is what a reviewer
reads and what every other check in this requirement (deny-wins-over-allow,
the sensitive-path warning, baseline-deny-unless-granted) reasons about, and
none of those checks may be silently bypassed by what the string actually
resolves to on disk.

#### Scenario: A deny nested inside a broader allow is rejected, not narrowed
- **WHEN** `allow = ["~"]` and `deny = ["~/.ssh"]`
- **THEN** compilation fails naming both entries, rather than succeeding
  with a compiled policy that grants `~` minus `~/.ssh`
- **AND** this holds identically when the two entries are the *same*
  string, not only when one nests inside the other — a manifest granting
  `~/.local/share/devcroft` (devcroft's own data dir, which every
  compilation denies unconditionally per the "Baseline denials"
  requirement) verbatim fails the same way, not silently

**Corrected from an earlier, aspirational version of this scenario.**
Landlock is purely additive — there is no deny primitive to carve a hole
out of a broader grant with, confirmed live against `nono-cli` itself
refusing to start on exactly this shape ("Landlock deny-overlap is not
enforceable on Linux"). A literal "`~` minus `~/.ssh`" was never
achievable; a compile-time rejection is what "deny wins" actually means
under this constraint — the sandbox does not start with a grant broader
than intended, rather than starting with one. `policy::capability_set`'s
own module doc has stated this since `use-nono-library`; this scenario
had not been updated to match until a review of the exact-match variant
above found the two describing different outcomes for the same
architecture.

#### Scenario: Sensitive-path warning
- **WHEN** `allow` includes a known credential directory (`~/.ssh`, `~/.aws`,
  `~/.config/gcloud`, `~/.kube`)
- **THEN** validation succeeds but a warning is printed at `up`, once

#### Scenario: Path traversal is rejected
- **WHEN** any `filesystem.allow`/`read`/`deny` entry contains a `..`
  path segment (e.g. `../../etc`, `~/../../etc`, `/etc/../root`)
- **THEN** validation fails with `ConfigError::InvalidPath` naming the
  field and value — a `..` segment is unresolved by the containment
  model every other check in this requirement relies on (deny-wins-over-
  allow, the sensitive-path warning, baseline-deny-unless-granted), and
  left unrejected it lets a relative entry resolve outside the project
  root once `nono` (invoked with the project root as its cwd) resolves
  it, silently violating "relative to the project root"

#### Scenario: A project-relative symlink escapes the project root
- **WHEN** a `filesystem.allow`/`read` entry is project-relative (not
  `~`-rooted or absolute) and is, or resolves through, a symlink whose
  real target lies outside the project root — e.g. a dependency or a
  malicious PR creating `vendor/cache -> ~/.ssh` inside the project, then
  an innocuous-looking `allow = ["vendor/cache"]`
- **THEN** `up` fails at layer `config` naming the entry and its real
  target, before any sandbox is created
- **AND** `policy --render` fails identically rather than silently
  showing the lexical entry as an ordinary in-project grant — the two
  commands must agree, since a render that looks fine for a policy that
  cannot actually be enforced is the exact failure this command exists to
  prevent
- **AND** an explicit `~/...` or absolute entry is unaffected: it already
  names its target directly, so there is no lexical string for a symlink
  to diverge from

### Requirement: Network policy model
The system SHALL model network policy as `default = "deny" | "allow"` plus
an `allow` list of domain names, and SHALL treat domain filtering as a
capability that may be unenforceable on a given host (see policy spec).

#### Scenario: Domains with deny default
- **WHEN** `default = "deny"` and `allow = ["github.com", "crates.io"]`
- **THEN** validation succeeds and the compiled policy requests a
  proxy-backed allowlist from the backend
