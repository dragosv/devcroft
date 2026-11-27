## Context

Every sandbox today has a provider. The documented answer for a project without one is `flox init` (`CLAUDE.md`, `docs/decisions.md`). That rule exists because running a project without a pinned environment means taking the host’s toolchain, or an unpinned one. A file-only sandbox runs no project code at all, so the rule’s premise does not apply. What it must guarantee instead is that nothing except the agent runs.

Execution today follows read grants:

- **Linux:** nono maps `AccessMode::Read` to `ReadFile | ReadDir | Execute` (`src/policy/capability_set.rs`), so every readable path, the read-write workspace included, is executable.
- **macOS:** `scope_exec_to_read_grants` allows `process-exec*` on every read grant.

Neither is enough here, so this change adds a mechanism of its own on each platform.

The contract comes from the local notes `new/croft-products-reproducibility-and-microvm-isolation-2026-11-23.md` (§5) and `new2/devcroft-1.0-release-scope-2026-11-25.md`. That earlier “croft” profile is folded into devcroft, not shipped as a second CLI.

## Goals / Non-Goals

**Goals:**

- A useful analysis-and-edit task on a project with no provider.
- OS-enforced: nothing runs except the selected agent’s object. A prompt instruction is not enforcement.
- No route from the workspace to host execution that the agent can plant without review.
- An honest result: the workflow never claims to have run a build or a test.

**Non-Goals:**

- Running the project. There is no detection, no `--with` and no build, which is `add-manifestless-mode`’s subject.
- Hostile-code containment beyond what the native backend gives. The kernel is still shared.
- A shell for the user. `devcroft shell` is refused, and the user edits files on the host as usual.

## Decisions

**D1. Explicit selection, no manifest read.** `--file-only` on `up` or `agent run` selects the profile. The root is the nearest ancestor of the canonical working directory holding `devcroft.toml` or `.git` (a directory or a file), and otherwise the working directory itself. Only names are checked; no file is opened. The sandbox name is `--name`, or the slug of the root’s basename. Nothing from a manifest is read: not the name, not grants, hooks or services. So there is nothing to approve, whatever the checkout’s trust state.

Alternative rejected: `env.provider = "none"`. It would put a non-executing profile in a field whose values all mean “here is how the project runs”, and it would reopen the `none` provider the decisions already reject.

**D2. The profile is fixed.** `Meta` records `profile = "file-only"`. Plain `up` on that sandbox restarts it in the same profile, found by its recorded root. A sandbox of the same name with the other profile is refused, naming `--name` or `rm`. Switching in place would mix a managed `HOME` with a file-only one for no benefit.

**D3. An agent is required, and it must be qualified.** `--file-only` with no selected recipe fails at layer `config`. So does a recipe that is not qualified for file-only on this platform, naming the recipes that are.

**D4. Execution confined to the object.**

- **Linux:** the view mounts the workspace, the sandbox `HOME`, `/tmp` and every other writable path with `MS_NOEXEC`, which also refuses executable mappings, so a workspace `.so` cannot be loaded. Only the agent’s object, and any loader or library the recipe declares for the platform, is mounted executable. devcroft’s runtime directory, the tooling layer and any store are not in the view. The keeper is already running when the agent starts and needs no executable grant of its own. Where the kernel supports it (Linux 6.3 and later), the sandbox’s PID namespace sets `vm.memfd_noexec = 2` to close fileless execution.
- **macOS:** the profile denies `process-exec*` and `file-map-executable` everywhere except the object and its declared libraries, instead of following read grants.
- **Both:** a host that cannot build this (no mount view, Landlock-only fallback, or a backend without the rule) refuses file-only at layer `backend`.

Alternative rejected: Landlock alone, which cannot grant read without execute through nono today.

**D5. Agent sessions only.** The keeper admits only the recipe’s own entrypoints, started by `agent run` and `agent acp`. `shell`, `exec`, `ssh` and `proxy` fail at layer `config` naming the profile. There are no hooks, services or published ports, because nothing declares them.

**D6. Network and credentials.** Default deny. The recipe’s active hosts are reachable through the egress proxy, with its broker under `api-key`. A recipe requiring `loopback_any` is refused, since no manifest can opt in. The user’s `[credentials] env` and brokers are withheld, as for an untrusted checkout. Without execution there is nothing to use them for, and they would only widen custody.

**D7. Workspace, `.git` and the manifest.** The project root is read-write with origin `workspace`. `.git` is read-only. Git configuration (`core.fsmonitor`, `core.hooksPath`) and hooks run on the host during routine commands without review, so a planted hook is host execution. A linked worktree’s gitdir lies outside the root and is not granted; with no `git` executable the agent could not use it anyway. `devcroft.toml`, if present, is protected as in manifest approval D9. `.devcroft/<name>/` stays read-only except `home` and `run`.

**D8. Recipe qualification for file-only.** The recipe schema records `file_only` per platform. It is set only when a measured run shows all of the following:

1. A read, search and edit task succeeds with every execution outside the object denied. The harness’s shell tool fails closed, and the harness reports the failure to the model instead of hanging.
2. Project files cannot run inside the harness’s own runtime. No model-reachable eval or run tool exists. Interpreter switches are reserved or unreachable: Bun’s `BUN_BE_BUN`, and Node’s `--eval` and `--require` through `NODE_OPTIONS`. Project-local plugins or extensions that load into the runtime are disabled by a setting devcroft writes outside the workspace, which the workspace cannot override. OpenCode’s project plugin directory is the first case to measure.
3. Project-local harness configuration cannot add execution or egress. A stdio MCP server or hook cannot execute, and a remote MCP server is outside the recipe’s hosts.
4. Search helpers the harness needs (for example ripgrep) ship inside the object as declared artifacts.

A recipe that fails any criterion is not offered here, whatever its other qualifications.

**D9. Reporting.** `agent run --file-only` prints one `stderr` line saying that no project command can run in this sandbox. `status` shows `profile: file-only`. `policy --render` shows the executable set (the object and its declared libraries) with origin `agent:<recipe>`, and the `noexec` mounts. Qualification evidence distinguishes static analysis from verification by execution. A file-only task never claims a passing test.

**D10. Revising the decision.** `docs/decisions.md` and `CLAUDE.md`’s framing rule change from “there is no non-reproducible mode” to “there is no non-reproducible execution”. File-only executes nothing from the project or the host, so it brings in no unpinned toolchain. `host` and `none` stay rejected as providers, with their existing messages.

## Risks / Trade-offs

- [The harness’s runtime can be turned into an interpreter] → OS rules cannot tell arguments apart. D8 criterion 2 is a qualification gate, and a recipe that fails it is not offered.
- [Fileless execution on kernels older than 6.3] → It needs native code already running under the model’s control, which criterion 2 excludes. Measure it on the reference hosts. If a qualified harness can reach it there, file-only refuses on such kernels.
- [Useful features need exec (LSP servers, formatters)] → They fail closed and the qualification records which. File-only trades them for the guarantee.
- [Edits to files the user later runs on the host, such as a `Makefile` or editor tasks] → Not enforceable. They are ordinary project files, and review is the defense. `.git` is special because it runs without review.
- [The same `.git` vector in the managed workflow] → Out of scope here. `docs/known-gaps.md` records it.
- [Name collision with a managed sandbox of the same checkout] → Refused, naming `--name`.
