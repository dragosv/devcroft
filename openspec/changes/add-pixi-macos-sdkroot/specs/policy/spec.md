## ADDED Requirements

### Requirement: Data-only SDK grant excludes process execution and developer usr/bin
The grant emitted for `[env.pixi] macos_sdk` SHALL cover only the canonical `.../SDKs/MacOSX*.sdk` directory with origin `host:macos-sdk`, SHALL be marked non-executable (`no-exec`, emitting no `(allow process-exec* ...)` rule on macOS Seatbelt), and SHALL NOT grant `/Library/Developer/CommandLineTools`, `Xcode.app/Contents`, or any `usr/bin` directory.

#### Scenario: Host clang and executables under SDK path remain denied
- **WHEN** a `pixi` sandbox with `macos_sdk = true` is up on macOS
- **THEN** `policy --render` lists `.../SDKs/MacOSX*.sdk` as read-only `(no-exec)` with origin `host:macos-sdk`, `/Library/Developer/CommandLineTools/usr/bin/clang` fails `EPERM`, and `execve` of any file inside the SDK directory fails `EPERM`
