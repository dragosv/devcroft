# macOS exec redirect: the measurement prototype

Evidence for `openspec/changes/add-macos-exec-redirect` (group 0). This is a
prototype, not the shipped library: it maps into a fixed runtime directory
(`DC_RT`), logs every redirect on stderr, and interposes only `posix_spawn`
and `execve`.

| File | What it is |
|---|---|
| `interpose.c` | the library exactly as measured on 2026-11-22 |
| `prog.c`, `prog2.c` | the two test programs (`system()`/`popen()`; `posix_spawn` of a script). Reconstructed on 2026-11-23 from the strings of the 2026-11-22 binaries, whose sources were not kept |
| `goprobe/` | task 0.2: `os/exec` and `syscall.Exec` from Go |
| `inside.sh`, `inside2.sh` | the 2026-11-22 in-sandbox scripts: variables exported after the shell started, then a fresh shell started with them |
| `inside3.sh` | group 0 workloads: Go, `make`, Python, `npm run`, Node |
| `s.sh` | the `#!/bin/sh` script every case runs |
| `run.sh` | builds everything and runs all three scripts in a devbox sandbox |

```sh
DEVCROFT=$PWD/target/debug/devcroft DEVCROFT_KEEPER_EXE=$PWD/target/debug/devcroft \
  spikes/macos-exec-redirect/run.sh
```

The runtime directory is the repo's `test-row-sh` example (brush) with `sh`,
`echo`, `pwd` and `sleep` links, so `BASH_VERSION` reads `5.2.37` when brush
runs and `5.3.15` when the closure's bash does. The results are in the
change's `design.md`, "Group 0".
