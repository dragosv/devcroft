# devcroft

Isolated, reproducible development environments in seconds, with no
host-wide daemon to install, no image build, no container, and no VM.
Each sandbox gets its toolchain from a lockfile, a boundary the kernel
enforces (Landlock on Linux, Seatbelt on macOS), and a real SSH server
your editor connects to like any remote machine.

This is the crate's own README — install and quick-start only. For the
full picture (architecture, how it compares, platform support, status,
known limitations) see the
[project README on GitHub](https://github.com/dragosv/devcroft#readme).

## Install

```sh
cargo install devcroft
```

Requires Rust stable (edition 2024) and one of `flox`, `nix`, or
`devbox` already installed — devcroft does not manage packages, it
sandboxes an environment one of those produces. Devbox itself builds on
Nix, so a devbox setup needs a working Nix installation too.

Run `devcroft doctor` after installing to check what this host actually
has.

## Run it

From a project that already has a flox/nix/devbox environment:

```sh
devcroft init    # write devcroft.toml
devcroft up      # build the environment, apply the policy, start the sandbox
devcroft shell   # open an interactive shell inside it
```

`devcroft exec -- <cmd>` runs one command instead of an interactive
shell. `devcroft down` stops the sandbox, keeping its state; `devcroft
rm` deletes it.

## Connect your editor

```sh
devcroft ssh-config --write
```

Each sandbox then answers at `<name>.devcroft` over SSH — open it with
VS Code or Cursor Remote-SSH, or use `ssh`, `scp`, `sftp`, `rsync`
directly. Nothing listens on a TCP port; the connection goes over a
unix socket only your user can open.

## Inspect the policy

```sh
devcroft policy --render [name]     # the compiled profile, every rule with its origin
devcroft why --path P --op <mode>   # whether one operation is allowed, and which rule decides
```

Nothing reaches the kernel that `policy --render` can't show you.

## Status and limits

devcroft is `0.0.x`: working, used daily in its own development, and
not yet offering compatibility guarantees between releases. The
isolation boundary catches mistakes, not attacks — full status, known
gaps, and the threat model are in the
[main README](https://github.com/dragosv/devcroft#readme) and linked
docs.

## License

Apache-2.0. Dependency license texts are in
[THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).
