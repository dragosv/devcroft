//! The `lifecycle` capability (task 4.2): the supervisor half of
//! design.md decision 1. `up` performs the fixed sequence — create the
//! listener, resolve the environment, compile the policy, spawn the
//! keeper under nono, wait for it to come up — while `down`/`rm` tear it
//! back down with the grace-period termination the lifecycle spec
//! requires.

mod state;
mod terminate;
mod up;

pub use state::{Health, StatePaths, health};
pub use terminate::{GRACE_PERIOD, TerminateError, down, rm};
pub use up::{UpError, UpOptions, UpOutcome, up};
