## Context

The keeper and proxy serve active sessions until `up --recreate` stops them. Existing credential checks can be answered using the manifest, user config and host environment.

## Decisions

Read the sandbox identity, recorded state and health under the existing lifecycle lock. Without `--recreate` (and without explicit flags that ask to change recorded state), an already healthy sandbox always takes the `AlreadyUp` fast path before startup credentials or tooling selections are re-validated, even if `~/.config/devcroft/config.toml` has changed since startup. For a new start, recovery or recreation, validate all currently resolvable credential and broker prerequisites before stopping either process, creating sandbox state or invoking the provider. Automatic starts (`maybe_auto_up` in `exec`, `shell`, `proxy`, `ssh`) check `Healthy` first and, only when the sandbox is down, load the same user configuration as explicit `up` and preserve the underlying error layer and exit code (`2` for `config`, `3` for `provider`, `4` for `backend`, `1` for `state`, `5` for `keeper`/`ssh`) instead of mapping every failure to `3`.

The provider may still fail during resolution. Protection applies to failures discoverable before that phase. A future consumer can prepare its own verified downloads at this boundary while retaining its lease until the new state is recorded.
