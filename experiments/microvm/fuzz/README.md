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

The libkrun fixes are now `../patches/libkrun-vsock-hardening.patch`, the last patch in
its series. The recorded results in `../results/macos-arm64-vsock-fuzz.json`
used its candidate form, under this directory in commit 848cf35.

The hvi pending reply queue bound is now `../patches/hvi-vsock-pending-bound.patch`,
verified in `../results/macos-arm64-hvi-pending-bound.json`.
