//! `add-linux-agent-fleet`, first implemented slice.
//!
//! Fleet's full shape — a supervisor owning N agents, each with its own
//! git clone, cgroup scope, namespace set, service stack and SSH
//! endpoint — is a large change still mostly unbuilt (see that change's
//! tasks.md). What lives here is the piece its `service-ports`
//! capability depends on and nothing else does: giving an agent its own
//! network namespace, so N agents can each bind the same declared port
//! without colliding.
//!
//! Built first because the D5 spike established it is *independently*
//! deliverable. Fleet's open question is how an agent gets connectivity
//! *out* of its namespace (pasta versus slirp4netns), which needs a
//! `/dev/net/tun` this devcontainer does not have. Port isolation needs
//! none of that — verified by measurement, not argument. So this slice
//! ships while D5 stays open, and an agent built on it today is fully
//! network-isolated except its own loopback, which is a safe default
//! rather than a half-finished one.

pub mod netns;
