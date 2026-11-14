# devenv-antigravity-sample

The Antigravity CLI (`agy`), Google's coding agent and Gemini CLI's
successor, running **inside** a devcroft sandbox. The agent comes from the
project's closure (`pkgs.antigravity-cli`, pinned by `devenv.lock`), not
from your machine.

It has the same shape as [devenv-claude-sample](../devenv-claude-sample/),
whose README explains what every agent here relies on: a PID namespace of
its own for `/proc`, a shell by name, the CA bundle. The matrix of all
eleven agents is [docs/agents.md](../../docs/agents.md).

## Run it

```sh
devcroft up
devcroft exec devenvagy -- agy --version
```

With a key, which reaches the sandbox from your shell and is never written to
a file here:

```sh
GEMINI_API_KEY=... devcroft up --recreate --yes
devcroft exec devenvagy -- agy -p "say ok"
```

`devenv` must be on your `PATH` for `up`. `antigravity-cli` is unfree in
nixpkgs, which is why `devenv.yaml` sets `allowUnfree: true`.

## What it needs that the others do not

**Any port on its own loopback.** `agy` starts an embedded language server
on two ports the kernel chooses (`127.0.0.1:0`) and then connects to them.
No flag or variable fixes those ports. A sandbox that grants only declared
ports refuses the bind, and `agy` exits before making a single request
(`listen tcp 127.0.0.1:0: bind: permission denied` in
`.gemini/antigravity-cli/cli.log`). `devcroft.toml` therefore sets
`network.loopback = "any"`.

That key covers the sandbox's *own* loopback only. On Linux every
`default = "deny"` sandbox has a network namespace with no route out, so a
port there is reachable by nothing outside the sandbox. The proxy remains the
only way out (`tests/loopback_any_port.rs`).

**macOS refuses the key.** There is no per-sandbox loopback there, so "any
port" would mean every service on your Mac's loopback. `up` says so and stops
rather than granting that. `agy` does not run in a devcroft sandbox on macOS
until its ports can be fixed (see `docs/known-gaps.md`).

**API-key mode is a setting, not a variable.** `GEMINI_API_KEY` alone is
ignored: `agy` then starts a browser sign-in and, headless, waits for it.
The key is used only with `"modelProvider": "gemini"` in
`~/.gemini/antigravity-cli/settings.json`. This sample's `enterShell` writes
that setting into the sandbox's own home (`.devcroft/devenvagy/home`), never
into yours, and leaves an existing file alone.

## What it reaches

`network.default = "deny"`, with `generativelanguage.googleapis.com` allowed.
Measured 2026-11-14 with a dummy key (agy 1.4.1): the request reached the API
and was refused there (`API_KEY_INVALID`). The proxy refused these along the
way without stopping it:

- `antigravity-unleash.goog`, feature flags;
- `play.googleapis.com`, telemetry.

## Its endpoint can be moved

`agy` honours `GOOGLE_GEMINI_BASE_URL`. Measured against a local listener,
it sent `POST /v1beta/models/…:streamGenerateContent` over plain HTTP, with
the key in `X-Goog-Api-Key`. That is what brokering a credential at the
egress proxy needs (`add-agent-workload`), so the sandbox could hold a
placeholder instead of the key.

## Not covered

A dummy key only, so no real conversation or file edit was run. The
Google-account sign-in was not tried: it needs a browser, and its tokens
would live in the sandbox's home.
