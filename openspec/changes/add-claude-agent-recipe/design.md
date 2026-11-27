## Package layout

Pin independently the standalone Claude CLI, private Node runtime, ACP adapter and the adapter’s SDK-matched platform binary. Record the build reported on each entrypoint. Both the standalone CLI and the SDK-bundled Claude executable are Bun binaries on Linux that require `/proc/self/maps` (`docs/agents.md`). So set `procfs = true` under `[requires.linux-*]` only (never on macOS) and declare `state_paths = [".claude", ".claude.json"]`. Start the adapter via its pinned Node and entrypoint. Validate the SDK pairing even when the project has configured a different Claude executable.

Extend `agent-packages` materialization with the `node-tarball` and reviewed `npm-lock` v3 layouts in Rust. Verify each tarball’s SHA-512 SRI, strip the package prefix, validate paths and links, select optional packages by OS, CPU and libc, and create local executable links. A selected install script needs evidence that it can be omitted. If this layout cannot run the adapter, stop qualification and decide whether a separately confined install is justified. That would be its own change. Its install step would write only to staging, with declared install hosts and a scrubbed environment that holds no credential of any mode. Downloaded scripts may never run on the host.

## Launch and state

Reserve `CLAUDE_CODE_EXECUTABLE` on the terminal and ACP paths after every environment overlay. The adapter uses the binary bundled with its SDK. Confirm session headers and environment stay out of child arguments using synthetic markers. Run the interactive and adapter builds in both orders on the same sandbox HOME. Record per-platform state paths, including the applicable Claude configuration and login files.

Merge the denial hook into the agent’s user-level settings in the sandbox HOME at `up`, with descriptor-anchored reads and atomic replacement. Never write it into the project. `add-agent-workload` currently says “Nothing SHALL write the hook at `up`.” That rule protects the project’s committed files, so this change amends it to say so before implementing the merge. A project that already registers the hook through `init --agent` must not run it twice. Measure that, along with precedence against project settings. Preserve unrelated preferences, and refuse malformed or redirected targets.

## Authentication

Qualify the broker and subscription login separately on Linux aarch64 and macOS arm64. Test paste-code or callback login where supported, host-side proxy behavior, persistence through recreation, and the active host list. A previously exported API key must not silently replace a selected login. Report the shared HOME custody of a subscription token. Publish only the platform and mode combinations evidenced by a real edit and project test.

The mode schema (`platforms`, per-platform `paths` and `json_keys`, and the same-recipe transition with `--drop-auth-state`) lives in `add-agent-packages`. Claude declares `.claude/.credentials.json` and the OAuth keys inside `.claude.json`, which also holds preferences and the hook, so a transition removes only those keys. On macOS, Claude Code may keep its login in the Keychain. `login` is offered there only if qualification shows that no Keychain token reaches, or outlives the move of, a sandbox switched to `api-key`. Qualification observes which credential the harness actually sends, not only `Meta.auth_mode`.

The SDK may pass client-supplied MCP configuration to its bundled binary on the command line. Qualification sends `session/new` with `mcpServers` carrying synthetic `headers` and inspects the child's `argv`. If the pinned SDK puts them there, `agent acp` refuses `mcpServers` with headers for this recipe.

1.0 keeps a login per sandbox. Sharing one login store across checkouts is a separate decision, recorded in `docs/decisions.md`: refresh-token rotation would let one checkout invalidate another's session, and copying would extend custody to every checkout's code.
