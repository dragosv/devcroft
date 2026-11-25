## 1. Preserve the current session

- [ ] 1.1 Read root identity and health under the lifecycle lock. Move the existing credential and broker preflight ahead of teardown, provider resolution and state creation. Keep the unconditional `AlreadyUp` fast path on plain `up` without `--recreate`, even when `~/.config/devcroft/config.toml` changed after startup.
- [ ] 1.2 In `maybe_auto_up`, check `Healthy` first; when starting a down sandbox, load the same user configuration as explicit `up` and propagate the real error layer/exit code (`2` for `config`, etc.) across `exec`, `shell`, `proxy` and `ssh` instead of hardcoded exit code `3`.

## 2. Verify the user-visible result

- [ ] 2.1 With a healthy sandbox, recreate with a missing broker key. Confirm the original keeper, proxy and an existing session still work. Confirm plain `up` and `devcroft shell` from a shell lacking the startup credential (or after editing `config.toml`) still succeed via `AlreadyUp`.
- [ ] 2.2 On a stopped or new sandbox, show that failed preflight invokes no provider, creates no new sandbox state, and exits with code `2` (layer `config`) on both explicit `up` and automatic startup (`devcroft shell` / `exec`).
