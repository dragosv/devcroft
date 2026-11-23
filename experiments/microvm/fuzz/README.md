# Host vsock fuzzing

Run these against a scratch copy of `artifacts/{libkrun,hvi}-source`, never the
artifacts themselves: the build recipes refuse a drifted source tree.

- libkrun: copy `libkrun_vsock_fuzz.rs` to
  `src/devices/src/virtio/vsock/fuzz_tests.rs`, add `#[cfg(test)] mod fuzz_tests;`
  to that directory's `mod.rs`, then run
  `cargo test --lib fuzz_tests -- --nocapture --test-threads=1` in `src/devices`.
  The probes print what happened; a passing test is not a passing result.
  `FUZZ_FIXED=1` re-enables the packets that trigger the known panic.
- hvi: append `hvi_vsock_fuzz.rs` to `src/devices/virtio/vsock.rs`.
- `FUZZ_SEED` and `FUZZ_SECS` control the random runs.

`libkrun-vsock-hardening.candidate.patch` applies on top of the recorded patch
series. No build uses it. Results: `../results/macos-arm64-vsock-fuzz.json`.
