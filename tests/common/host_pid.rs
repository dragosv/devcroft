//! A pid a sandbox reports, as the host numbers it.
//!
//! Since `add-up-pid-namespace`, a sandbox on a host that allows it has a PID
//! namespace of its own: `$$` in a session, a service's pid from
//! process-compose, and the keeper's own view are all that namespace's
//! numbers. A test that checks or signals one from the host must translate it
//! first, or it watches (and may signal) whatever host process happens to
//! have that number.

#![allow(dead_code)]

use std::path::Path;

fn ns_of(pid: &str) -> Option<std::path::PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/ns/pid")).ok()
}

/// The host pid of `ns_pid` in the PID namespace of the sandbox whose
/// pidfile is `pidfile`; `ns_pid` itself when the sandbox shares the host's.
/// `None` when no such process exists.
pub fn host_pid(pidfile: &Path, ns_pid: i32) -> Option<i32> {
    let keeper = std::fs::read_to_string(pidfile).ok()?;
    let keeper = keeper.split_whitespace().next()?;
    let sandbox_ns = ns_of(keeper)?;
    if Some(&sandbox_ns) == ns_of("self").as_ref() {
        return Some(ns_pid);
    }
    std::fs::read_dir("/proc").ok()?.flatten().find_map(|e| {
        let name = e.file_name();
        let pid = name.to_str()?;
        let host: i32 = pid.parse().ok()?;
        if ns_of(pid)? != sandbox_ns {
            return None;
        }
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        let innermost = status
            .lines()
            .find_map(|l| l.strip_prefix("NSpid:"))?
            .split_whitespace()
            .last()?
            .parse::<i32>()
            .ok()?;
        (innermost == ns_pid).then_some(host)
    })
}

/// Whether the sandbox whose pidfile is `pidfile` has a PID namespace of its
/// own.
pub fn has_own_pid_namespace(pidfile: &Path) -> bool {
    std::fs::read_to_string(pidfile)
        .ok()
        .and_then(|k| ns_of(k.split_whitespace().next()?))
        .is_some_and(|ns| Some(ns) != ns_of("self"))
}
