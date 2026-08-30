//! devcroft's internals, published so the integration suite in `tests/`
//! can drive them as a library.
//!
//! # This is not a stable API
//!
//! devcroft is a command-line tool. The modules below are its internal
//! architecture — a policy compiler, a supervisor, a provider layer, an
//! SSH server — exposed because `tests/*.rs` runs out-of-process against
//! real sandboxes and needs to call `up`/`down` directly. They were
//! designed as internals, not as a curated public interface, and no
//! stability guarantee is offered or implied: any of these items may
//! change shape or disappear in a patch release.
//!
//! Depend on the `devcroft` **binary** and its documented command surface
//! (`devcroft --help`, and the README), not on these types. If you have a
//! use case that genuinely needs a library, open an issue describing it —
//! a supported subset can then be carved out deliberately and versioned
//! properly, which is a different thing from freezing whatever the
//! internals happen to look like today.

pub mod config;
pub mod exec;
pub mod fleet;
pub mod keeper;
pub mod lifecycle;
pub(crate) mod paths;
pub mod policy;
pub mod provider;
pub mod proxy;
pub mod services;
pub mod ssh;
