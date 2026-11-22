# Locked mise Go build

The root example targets **Linux glibc ARM64**. On **macOS ARM64**, use
`cd macos-arm64` first; that directory carries the corresponding literal URL
and lock. x64 and musl runtimes have not been qualified and fail closed.
Install mise **2026.11.9 or later** on the host. A newer release draws a warning
until qualified. No Nix installation is needed.

```sh
devcroft up
devcroft exec -- go build -buildvcs=false -o hello .
devcroft exec -- ./hello
devcroft policy --render
devcroft status
devcroft rm --yes
```

`up` and `status` print:

```text
guarantee: artifact (identical downloaded artifacts; runtime links against host libraries, so behavior depends on this host)
```

The provider verifies the locked SHA-256 and restores the HTTP archive into
`~/.local/share/devcroft-artifacts/mise/<object-id>/data/installs/`. Sandboxes
with the same recipe share that object read-only. The last `rm` reclaims it;
`down` keeps it. `policy --render` attributes installation, shell and runtime
grants to `provider:mise`. The manifest grants only this project for writing;
Go's build cache lives in `.cache/go`.

`mise run build` is unavailable inside: the host mise binary, its configuration
and task runner are not granted. Run the task's command directly as above.
The `[tasks]` entry demonstrates a declaration that is deliberately not carried.
`[env]` is interpreted by devcroft and runs once in confined provisioning,
including the supported `_.source`, `_.file` and `_.path` directives.

Only literal HTTPS `http:<name>` tools with `version` and `url` options are
admitted initially. Ordinary `go = "1.24.0"` uses mise's core backend, which
executes the downloaded Go binary on the host during installation and is refused.
Plugin, aqua, GitHub and conda backends are also unadmitted pending their own
real-runtime qualification. No downloaded tool runs host-side through the
admitted HTTP path. Services (`[daemons]`) belong to `add-mise-daemons` and are
currently refused; `--skip-hooks` explicitly skips their declaration.

To update Go, change the literal version and platform URL, run `mise lock` on
that native platform, and check the checksum against Go's published release
metadata before `devcroft up --recreate`. Provisioning may download the locked
archive using the host network; the sample's runtime has no network access.
