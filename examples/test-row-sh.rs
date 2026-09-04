//! A POSIX shell for the Nix-free test row (`add-nix-free-test-row`).
//!
//! **One line, and that is the point.** The row needs a real shell from
//! neither the Nix store nor the host's `/bin` — the store because the row
//! exists to work without one, and `/bin` because a copied macOS platform
//! binary lands in an unkillable `UE` state when executed (measured; see
//! that change's design.md N2).
//!
//! Taking `brush` as a dev-dependency dissolves the problems every other
//! source had: one artifact on both platforms instead of BusyBox-on-Linux
//! and dash-on-macOS, pinned by `Cargo.lock` instead of a hand-maintained
//! per-architecture hash, MIT instead of GPL-2.0, and no C toolchain at
//! fixture-setup time.
//!
//! **Why `examples/` and not `src/bin/`.** `src/bin/` targets are
//! auto-discovered — CLAUDE.md records that excluding `spike.rs` from the
//! packaging allowlist is load-bearing for exactly that reason — and they
//! cannot use dev-dependencies, so a wrapper there would drag `brush` into
//! the *shipped* tree. An example can use dev-dependencies and is never
//! installed. Measured: adding this changes devcroft's shipped dependency
//! tree by zero crates.

fn main() {
    brush_shell::entry::run()
}
