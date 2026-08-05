//! The `lifecycle` capability (tasks 4.2/4.3): the supervisor half of
//! design.md decision 1. `up` performs the fixed sequence — create the
//! listener, resolve the environment, compile the policy, spawn the
//! keeper under nono, wait for it to come up — while `down`/`rm` tear it
//! back down with the grace-period termination the lifecycle spec
//! requires. `status`/`logs`/`ps` are read-only views over the same state.

mod state;
mod status;
mod terminate;
mod up;

pub use state::{Health, Meta, StatePaths, health};
pub use status::{KeeperStatus, SandboxStatus, SandboxSummary, StatusError, logs, ps, status};
pub use terminate::{GRACE_PERIOD, TerminateError, down, rm};
pub use up::{UpError, UpOptions, UpOutcome, up};
