//! A lifeline: what makes an agent's host-side helpers (its egress proxy,
//! its port forwarders) exit when the agent does, with no command running.
//!
//! **A pipe nobody writes to.** The agent's PID 1 holds the write end, and
//! each helper blocks reading the other. PID 1's exit takes the whole
//! agent with it (a PID namespace dies with its init), and the kernel
//! closes its fds as it goes, so the read returns EOF at the moment the
//! agent is gone, however it went: a clean stop, an OOM kill, a
//! `cgroup.kill` from outside. The helper exits, and its host port and its
//! proxy socket go with it.
//!
//! Before this, the helpers died only when a later command's reconcile
//! noticed the agent was gone and killed their leaf: until someone ran
//! `fleet ls`, a crashed agent's host ports stayed allocated to nothing,
//! which the `service-ports` spec rules out ("a mapping SHALL NOT outlive
//! the agent it belongs to").
//!
//! **The supervisor holds a write end too, until PID 1 has one.** So a
//! supervisor that dies between starting the helpers and starting the
//! agent also closes the last writer, and the helpers exit rather than
//! running on with no record and no socket through which to find them.
//! Both write ends are close-on-exec: one leaking into an unrelated child
//! would keep every helper it serves alive for as long as that child
//! lives.

use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::process::Command;

/// Where a helper finds its end: the fd number, which `attach` keeps open
/// across the helper's `exec`.
pub const ENV: &str = "DEVCROFT_LIFELINE_FD";

/// A lifeline's two ends, both close-on-exec.
pub struct Lifeline {
    /// Handed to each helper ([`attach`]).
    pub read: OwnedFd,
    /// Handed to the agent's PID 1, and dropped here once it has it.
    pub write: OwnedFd,
}

impl Lifeline {
    pub fn new() -> io::Result<Lifeline> {
        let mut fds = [0; 2];
        // SAFETY: `fds` is a valid two-element out-array.
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: both fds were just created and are owned by nothing else.
        Ok(unsafe {
            Lifeline {
                read: OwnedFd::from_raw_fd(fds[0]),
                write: OwnedFd::from_raw_fd(fds[1]),
            }
        })
    }
}

/// Give `cmd` the read end: inheritable in that child alone, and named in
/// its environment for [`watch`].
pub fn attach(cmd: &mut Command, read: &OwnedFd) {
    crate::lifecycle::pass_fds(cmd, &[read.as_raw_fd()]);
    cmd.env(ENV, read.as_raw_fd().to_string());
}

/// In a helper: if it was given a lifeline, exit once every writer is gone.
/// Returns at once either way; the wait is on its own thread.
pub fn watch() {
    let Some(fd) = std::env::var(ENV)
        .ok()
        .and_then(|v| v.parse::<RawFd>().ok())
    else {
        return;
    };
    // SAFETY: the supervisor placed this fd for this process alone (`attach`).
    let mut end = unsafe { std::fs::File::from_raw_fd(fd) };
    std::thread::spawn(move || {
        let mut byte = [0u8; 1];
        // Nobody writes, so this returns only at EOF (the agent is gone) or
        // on an error, and either way the helper has nothing left to serve.
        loop {
            match end.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => continue,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        std::process::exit(0);
    });
}
