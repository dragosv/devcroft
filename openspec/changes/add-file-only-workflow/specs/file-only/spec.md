## ADDED Requirements

### Requirement: A file-only sandbox is selected explicitly and reads no manifest
`devcroft up --file-only` and `devcroft agent run --file-only [<recipe>]` SHALL
create a sandbox with no environment provider. Its root SHALL be the nearest
ancestor of the canonical working directory that holds `devcroft.toml` or
`.git`, else the working directory, and its name `--name` or the slug of the
root's basename. devcroft SHALL NOT open or parse any manifest for it. `Meta`
SHALL record `profile = "file-only"`, and plain `up` on that sandbox SHALL
restart it in the same profile. A sandbox of the same name recorded with the
other profile SHALL fail at layer `config` naming `--name` and `rm`.

#### Scenario: Project without a provider
- **WHEN** `devcroft agent run --file-only opencode` runs in a Git checkout that has no `devcroft.toml`
- **THEN** a sandbox rooted at the checkout starts with no provider and the agent attaches

#### Scenario: Manifest present but not read
- **WHEN** the root holds a `devcroft.toml` that would allow a host and `up --file-only` runs
- **THEN** the host is not reachable, and no manifest key appears in `policy --render`

#### Scenario: Profile conflict
- **WHEN** a managed sandbox named `app` exists and `up --file-only` would also name its sandbox `app`
- **THEN** it fails at layer `config` naming `--name` and `rm`

### Requirement: A file-only sandbox needs a qualified agent
`--file-only` SHALL fail at layer `config` when no recipe is selected, or when
the selected recipe is not qualified for file-only on this platform. The error
SHALL name the recipes that are.

#### Scenario: No agent
- **WHEN** `devcroft up --file-only` runs with no `--agent` and no `[agent]`
- **THEN** it fails at layer `config` before creating state

#### Scenario: Recipe not qualified
- **WHEN** the selected recipe has no file-only qualification for `macos-arm64` and `up --file-only` runs there
- **THEN** it fails at layer `config` naming the qualified recipes

### Requirement: Only the agent's object can execute
In a file-only sandbox no file SHALL be executable or mappable as executable
except the selected recipe's object and the loader or libraries it declares for
the platform. On Linux the workspace, the sandbox `HOME`, `/tmp` and every
other writable path SHALL be mounted `noexec`. devcroft's runtime directory,
the tooling layer and any store SHALL be absent from the view, and the PID
namespace SHALL set `vm.memfd_noexec = 2` where the kernel supports it. On macOS
the profile SHALL deny `process-exec*` and `file-map-executable` outside the
object and its declared libraries. A host that cannot enforce this SHALL refuse
file-only at layer `backend`.

#### Scenario: Prebuilt binary in the workspace
- **WHEN** the agent asks to execute an executable file committed to the workspace
- **THEN** the execution fails and the file is unchanged

#### Scenario: Shell tool
- **WHEN** the agent's shell tool runs any command
- **THEN** it fails because no shell can execute, and the agent continues the task

#### Scenario: Library from the workspace
- **WHEN** a process in the sandbox tries to map a shared library from the workspace as executable
- **THEN** the mapping fails

#### Scenario: Host without a mount view
- **WHEN** a Linux host cannot construct a mount view
- **THEN** `up --file-only` fails at layer `backend` naming the missing enforcement

### Requirement: Only agent sessions run
A file-only sandbox SHALL admit only the recipe's entrypoints, started by
`agent run` and `agent acp`. `devcroft shell`, `exec`, `ssh` and `proxy` SHALL
fail at layer `config` naming the profile. It SHALL have no hooks, services or
published ports.

#### Scenario: Shell refused
- **WHEN** `devcroft shell` runs against a file-only sandbox
- **THEN** it fails at layer `config` saying the sandbox is file-only

### Requirement: Egress is the recipe's hosts alone
A file-only sandbox SHALL deny egress by default and allow only the selected
recipe's active hosts, through the egress proxy and, under `api-key`, its
broker. A recipe requiring `loopback_any` SHALL be refused. The user
configuration's `[credentials] env` and `[[credentials.broker]]` SHALL be
withheld, and `up` SHALL name each one.

#### Scenario: Model reachable, nothing else
- **WHEN** the agent runs in a file-only sandbox and a process tries to reach a host outside the recipe's
- **THEN** the model request succeeds and the other connection is refused

### Requirement: The workspace is writable, Git state is not
The project root SHALL be read-write with origin `workspace`. Its `.git` SHALL
be read-only. A linked worktree's gitdir outside the root SHALL NOT be
granted. A `devcroft.toml` at the root SHALL be protected as the `policy`
requirement "The workload cannot reach the authoritative manifest"
(`add-manifest-approval`) requires.

#### Scenario: Planted Git hook
- **WHEN** the agent tries to write `.git/hooks/pre-commit` or `.git/config`
- **THEN** the write fails, while an edit to a source file succeeds

### Requirement: Recipes are qualified for file-only per platform
A recipe SHALL be offered for file-only on a platform only when a recorded run
there showed all of the following:

- a read, search and edit task succeeding with all execution outside the object denied;
- its shell tool failing closed, reported to the model rather than hanging;
- no model-reachable way to run project files in its own runtime, whether by an eval or run tool, an interpreter switch such as `BUN_BE_BUN` or `NODE_OPTIONS`, or project-local plugins (which SHALL be disabled by a setting devcroft writes outside the workspace);
- no execution or egress added by project-local harness configuration;
- every search helper it needs shipped inside its object.

#### Scenario: Project plugin directory
- **WHEN** the workspace contains a plugin for the harness that writes a marker when loaded
- **THEN** after a file-only task the marker is absent

#### Scenario: Interpreter switch
- **WHEN** a fixture tries to make the harness run a workspace script through an interpreter switch it can set
- **THEN** the script does not run, or the recipe is not qualified for file-only

### Requirement: File-only reports what it cannot do
`agent run --file-only` SHALL print one `stderr` line saying that no project
command can run in the sandbox. `status` SHALL show `profile: file-only`, and
`policy --render` SHALL show the executable set with its origins and the
`noexec` mounts. Published qualification SHALL distinguish static analysis from
verification by execution.

#### Scenario: Status of a file-only sandbox
- **WHEN** `status` runs for a file-only sandbox
- **THEN** it prints `profile: file-only` and the selected agent
