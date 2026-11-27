# Opt-in MicroVM Mode (`devcroft up --vm`)

`devcroft` keeps the native `process` tier as its default (`devcroft up`). On qualified **macOS ARM64 (`macOS 14+`)** hosts, you can opt into running a project inside a Linux ARM64 microVM backed by `libkrun` 2.0 via `devcroft up --vm`.

## Defense-in-depth architecture

1. **Host confinement (Seatbelt)**: the detached `devcroft-vm-helper` (`libkrun`) applies a macOS Seatbelt profile before initializing the VMM, restricting filesystem access strictly to the verified read-only artifacts, the instance's private disks, and its private `0700` socket directory.
2. **Hypervisor boundary (`libkrun` / `Hypervisor.framework`)**: runs a verified Linux ARM64 kernel (`Image`), initramfs (`initramfs.cpio`), read-only base runtime disk (`/dev/vda`), private state disk (`/dev/vdb`), optional read-only toolchain disk (`/dev/vdc` -> `/toolchain`), and optional persistent ext4 workspace disk (`/dev/vdd` -> `/workspace`).
   - **No guest NIC (`nic: false`)**, **no TSI (`tsi: false`)**, and **no `virtio-fs` host filesystem sharing**.
   - All host–guest communication uses four fixed vsock channels bound to `0600` Unix sockets inside the instance directory:
     - `vsock:1024` -> `control.sock` (PID 1 health/quiesce/shutdown control)
     - `vsock:1025` -> `keeper.sock` (`devcroft exec`, `shell`, `sync-in`, `sync-out`)
     - `vsock:1026` -> `ssh.sock` (guest `russh` for `devcroft ssh`, `proxy`, `-L`, SFTP)
     - `vsock:1027` -> `proxy.sock` (loopback egress relay to host `nono-proxy 0.82`)
3. **Guest confinement (`nono` Landlock)**: `guest-keeper` applies Linux Landlock before accepting sessions, allowing writes only to `/workspace`, `/home/devcroft`, and `/tmp`, read-only access to `/bin` and `/toolchain`, and outbound TCP connections only to explicitly granted loopback ports (`18080` for the egress relay, `8080` for the SSH fixture, etc.).
4. **Host egress proxy & zero-secret credential brokering (`nono-proxy 0.82`)**:
   - When `[network].allow` is empty, no proxy is started and the VM stays completely offline.
   - Unrestricted `[network].default = "allow"`, `localhost`, and private/local IP literals are refused at `up --vm`, `policy --render --vm`, and `why --vm`.
   - When `[network].allow` is non-empty, `up --vm` spawns `__egress_proxy` (`nono-proxy 0.82`) on the host and binds `proxy.sock`. Inside the guest, `guest-keeper` bridges `127.0.0.1:18080 -> vsock:1027 -> proxy.sock` and injects `HTTP_PROXY`/`HTTPS_PROXY=http://127.0.0.1:18080`, `NO_PROXY=localhost,127.0.0.1,::1`, and (when present in `/toolchain`) `SSL_CERT_FILE=/usr/etc/ssl/certs/ca-bundle.crt`.
   - Entries in `~/.config/devcroft/config.toml` under `[[credentials.broker]]` deliver only a placeholder key and `http://127.0.0.1:18080/brokerN` to guest sessions; the real secret is piped to `__egress_proxy` over `stdin` and never written to guest memory, disks, logs, or controller JSON files.

## Quick start

### 1. Install a local trusted VM bundle once

`install-local` copies the helper binary (preserving macOS Mach-O signatures), kernel, initramfs, base runtime image, and optional paired `--workload` + `--data-seed` disks into `~/.local/share/devcroft/vm-artifacts/` with verified SHA-256 digests:

```sh
devcroft runtime install-local \
  --helper experiments/microvm/libkrun/target/debug/devcroft-krun-spike \
  --kernel experiments/microvm/artifacts/kernel-landlock/Image \
  --initramfs experiments/microvm/artifacts/stage-b-nix-egress/initramfs.cpio \
  --runtime experiments/microvm/artifacts/stage-b-nix-egress/runtime.img \
  --workload experiments/microvm/artifacts/agents/toolchain.img \
  --data-seed experiments/microvm/artifacts/agents/data.seed.img \
  --version local-agents-1
```

Inspect qualification and installed bundle status:

```sh
devcroft runtime status
devcroft doctor
```

### 2. Configure project egress and optional brokered LLM credentials

In `devcroft.toml`:

```toml
[sandbox]
name = "my-agent"

[env]
provider = "flox"

[network]
default = "deny"
allow = ["api.anthropic.com", "api.openai.com"]
```

In `~/.config/devcroft/config.toml`:

```toml
[[credentials.broker]]
host = "api.anthropic.com"
header = "x-api-key"
env = "ANTHROPIC_API_KEY"
endpoint_env = "ANTHROPIC_BASE_URL"
placeholder = "sk-ant-proxy-{}"
```

Inspect effective VM authority before booting (never prints secret values):

```sh
devcroft policy --render --vm
devcroft why --vm --host api.anthropic.com
devcroft why --vm --env ANTHROPIC_API_KEY
devcroft why --vm --path /etc/passwd --op read
```

### 3. Boot, sync workspace, run commands / SSH, and retrieve changes

```sh
export ANTHROPIC_API_KEY="sk-ant-..."

# Boot the VM and sync project files safely into /workspace (skips symlinks, .git, target, etc.)
devcroft up --vm --sync

# Run commands or interactive shell inside the guest (with proxy + broker env)
devcroft exec -- claude --bare -p "Summarize /workspace"
devcroft shell

# Connect over OpenSSH (per-instance keys, no agent/X11/remote forwarding) or forward a guest port
devcroft ssh my-agent -- uname -a
devcroft ssh -N -L 127.0.0.1:8080:127.0.0.1:8080 my-agent

# Inspect status and combined helper + egress proxy logs
devcroft status
devcroft logs

# Pull edits safely out of /workspace (pure-Rust hardened ustar extractor; rejects traversal, .git writes, and symlinks)
devcroft runtime sync-out --out ./exported-edits
devcroft runtime sync-out --yes   # confirm overwriting host project files

# Graceful shutdown (preserves persistent ext4 workspace disk across cold restarts)
devcroft down
devcroft up --vm

# Export private state disk or delete all VM state
devcroft runtime export-disk my-agent --out ./state-backup.img
devcroft rm --yes
```

## Current scope and limitations

- **Opt-in only**: `devcroft up` continues to use the native `process` tier by default; VM mode requires `--vm` (or `--runtime vm`).
- **Platform qualification**: only `macos-arm64` (`macOS 14+` with `libkrun` 2.0) is currently qualified; Linux ARM64/x86_64 and macOS x86_64 fail closed until native qualification completes.
- **Darwin-only providers**: `provider = "swift"` refuses VM execution rather than silently falling back to native.
- **Open items before default migration (`add-vm-project-runtime` Task 4.5)**: in-VM preparation execution, host sleep/suspend qualification, native Linux qualification, and cross-spec reconciliation remain open.
