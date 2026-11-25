## Package layout

Pin independently the standalone Claude CLI, private Node runtime, ACP adapter and the adapter's SDK-matched platform binary. Record the build reported on each entrypoint. Both the standalone CLI and the SDK-bundled Claude executable are Bun binaries on Linux that require `/proc/self/maps` (`docs/agents.md`), so set `procfs = true` under `[requires.linux-*]` only (never on macOS) and declare `state_paths = [".claude", ".claude.json"]`. Start the adapter via its pinned Node and entrypoint. Validate the SDK pairing even when the project has configured a different Claude executable.

Extend `agent-packages` materialization with the `node-tarball` and reviewed `npm-lock` v3 layouts in Rust. Verify each tarball's SHA-512 SRI, strip the package prefix, validate paths and links, select optional packages by OS, CPU and libc, and create local executable links. A selected install script needs evidence that it can be omitted. If this layout cannot run the adapter, stop qualification and decide whether a separately confined install is justified. Such an install could write only to staging with declared install hosts. Downloaded scripts may never run on the host.

## Launch and state

Reserve `CLAUDE_CODE_EXECUTABLE` on the terminal, launcher and ACP paths after every environment overlay. The adapter uses the binary bundled with its SDK. Confirm session headers and environment stay out of child arguments using synthetic markers. Run the interactive and adapter builds in both orders on the same sandbox HOME. Record per-platform state paths, including the applicable Claude configuration and login files.

Merge the denial hook into the agent’s user settings with descriptor-anchored reads and atomic replacement. Verify precedence against project settings, preserve unrelated preferences and refuse malformed or redirected targets.

## Authentication

Qualify the broker and subscription login separately on Linux aarch64 and macOS arm64. Test paste-code or callback login where supported, host-side proxy behavior, persistence through recreation, and the active host list. A previously exported API key must not silently replace a selected login. Report the shared HOME custody of a subscription token. Publish only the platform and mode combinations evidenced by a real edit and project test.
