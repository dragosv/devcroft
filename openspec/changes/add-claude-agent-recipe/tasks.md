## 1. Qualify the distribution

- [ ] 1.1 Pin the standalone Claude Code CLI, private Node, adapter, paired SDK and platform payloads. Record URLs, checksums, component versions, runtime dependencies, self-update controls, Linux-only `procfs = true` requirement (confirming `macos-arm64` is unaffected) and `state_paths = [".claude", ".claude.json"]` per target.
- [ ] 1.2 Implement and test the script-free `node-tarball` and `npm-lock` layouts described in design.md. Refuse an unqualified install script, an unsafe entry or a missing or invalid integrity value. Verify a read-only installation initializes and runs a project task. If script-free layout fails, record the blocked combination before considering confined provisioning.

## 2. Qualify both entrypoints

- [ ] 2.1 Check all launch paths with a reserved override inherited from the provider or activation script. Confirm the SDK-paired executable is used for ACP and synthetic session headers stay out of process arguments.
- [ ] 2.2 Use the CLI and ACP in both orders on one HOME. Merge the denial hook safely; check project-level precedence, existing user settings, malformed files, redirected paths and login persistence through recreation.
- [ ] 2.3 Measure brokered API-key access and subscription login separately per platform. Check active hosts, direct-login broker collisions, missing keys, exported keys under login and proxy compatibility on macOS. Advertise only successful modes; disclose who can read stored logins.
- [ ] 2.4 Complete an edit and a project test from the terminal and the qualified editor. Confirm the terminal transport and record whether this harness emits OSC 7501. Publish the paired versions and supported platform/auth matrix.
