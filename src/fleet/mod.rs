//! `add-linux-agent-fleet`, implemented slices.
//!
//! Fleet's full shape — a supervisor owning N agents, each with its own
//! git clone, cgroup scope, namespace set, service stack and SSH
//! endpoint — is a large change still mostly unbuilt (see that change's
//! tasks.md). What lives here are the pieces that are independently
//! deliverable ahead of the supervisor that will eventually own them:
//!
//! - [`netns`]: an agent's own network namespace, so N agents can each
//!   bind the same declared port without colliding
//!   (`specs/service-ports`). Needs no forwarding helper and no
//!   `/dev/net/tun`, unlike egress — see that module's doc for why it
//!   shipped ahead of D5.
//! - [`workspace`]: a shared bare git mirror plus an independent clone
//!   per agent, so no agent's git operations can lock or corrupt
//!   another's (`specs/workspace-isolation`, design.md D7).

pub mod netns;
pub mod workspace;
