//! The `lifecycle` capability (tasks 4.2/4.3): the supervisor half of
//! design.md decision 1. `up` performs the fixed sequence — create the
//! listener, resolve the environment, compile the policy, spawn the
//! keeper under nono, wait for it to come up — while `down`/`rm` tear it
//! back down with the grace-period termination the lifecycle spec
//! requires. `status`/`logs`/`ps` are read-only views over the same state.

mod hooks;
mod state;
// `pub(crate)` for the same reason `up` is: `test_support` re-exports one
// seam function out of it under the `test-support` feature.
pub(crate) mod status;
mod terminate;
// `pub(crate)` so the crate root can name the path; the module is still not
// public API. `test_support` re-exports the single seam function out of it
// under the `test-support` feature, and nothing else in here is reachable
// from outside the crate.
pub(crate) mod up;

pub use hooks::HookError;
pub use state::{Health, Meta, StatePaths, client_key_paths, health, read_meta};
pub use status::{KeeperStatus, SandboxStatus, SandboxSummary, StatusError, logs, ps, status};
pub use terminate::{GRACE_PERIOD, TerminateError, down, rm};
pub use up::{UpError, UpOptions, UpOutcome, up};
// `pub(crate)`, not part of the public surface: `crate::proxy::spawn`
// needs the identical fd-inheritance dance `up_process` already does for
// the control/SSH sockets, and re-exporting through here is what makes
// `up`'s private submodule path reachable from outside `lifecycle` at
// all — `up::clear_cloexec` being `pub(crate)` doesn't help by itself
// when the `up` module segment itself is private.
pub(crate) use up::clear_cloexec;
