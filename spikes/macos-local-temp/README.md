# macOS local temp prototype

`run.py` compiles an interposer, evaluates the checked-in devenv sample's
`enterShell` in a scratch copy, and runs probes in a Seatbelt profile. The
profile grants the scratch project and denies host temp data. Two profiles
also deny each other's physical temp directories while sharing that checkout.
The Cargo and file-loop benchmarks run outside Seatbelt. Both benchmark arms
load the existing exec redirect. The prototype arm also loads `interpose.c`.

Run on macOS with a warm Nix cache for the sample's pinned inputs. Use closure
executables for Bash, coreutils, Python, Node, process-compose, Git and PostgreSQL
so dyld can load the prototype. Supply the coreutils `bin` directory and
absolute executable paths for the other tools.

```sh
python3 spikes/macos-local-temp/run.py \
  --bash "$DC_BASH" --coreutils "$DC_COREUTILS_BIN" \
  --python "$DC_PYTHON" --node "$DC_NODE" \
  --process-compose "$DC_PROCESS_COMPOSE" --git "$DC_GIT" --initdb "$DC_INITDB" \
  --cargo "$(rustup which cargo)"
```

`--mysql-install-db` supplies the MySQL or MariaDB initialization executable.
`--skip-bench` runs the compatibility probes alone. Apple SSH is a control for
a program that drops the library. Its literal path is refused and the physical
path works. The same-checkout devenv probe currently exits with a failure.
See the [OpenSpec design](../../openspec/changes/add-macos-local-temp/design.md)
for the result.
