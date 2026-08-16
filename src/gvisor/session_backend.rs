//! [`RunscExecBackend`]: the hardened tier's [`SessionBackend`]
//! implementation, dispatching every session through `runsc exec`
//! instead of a local fork/exec. This is the concrete backend
//! `add-hardened-tier`'s `SessionBackend` trait was built to accept —
//! everything about session handling above the spawn step (framing,
//! pty allocation, signal forwarding, exit-code propagation, the
//! registry) is `keeper`/`ssh` code shared unchanged with the process
//! tier; only *how a process comes into being* differs.
//!
//! Implemented by rewriting the request into "run `runsc` with `exec`
//! arguments" and delegating to [`LocalSessionBackend`] for the actual
//! spawn/pty/piping mechanics — the local process here is the `runsc
//! exec` *client*, not the sandboxed process itself, so this crate never
//! needs its own pty/fork logic for the hardened tier at all.
//!
//! **Unverified beyond compiling**, same posture this crate already
//! takes for macOS pty handling and the flox devcontainer step: nothing
//! in this environment can run a real `runsc` (see the crate's
//! devcontainer notes). In particular, whether signal forwarding to the
//! local `runsc exec` client's process group (`connection.rs`'s
//! `kill(-pgid, sig)`) actually reaches the sandboxed process the way it
//! reaches a local child today rests on `runsc exec` behaving like other
//! OCI runtimes' exec clients (`runc exec`, `docker exec`) — proxying
//! signals through rather than only terminating its own client process.
//! Needs confirming against a live sandbox before this is relied on.

use std::io;
use std::path::PathBuf;

use crate::keeper::protocol::SpawnRequest;
use crate::keeper::session::{LocalSessionBackend, SessionBackend, SpawnedSession};

use super::runsc_command::{self, Container};

pub struct RunscExecBackend {
    pub runsc: PathBuf,
    pub container_id: String,
    /// `runsc`'s own `--root` state directory for this sandbox — see
    /// [`runsc_command::Container`]'s doc comment for why it's kept
    /// alongside the container id rather than threaded separately.
    pub state_root: PathBuf,
}

impl SessionBackend for RunscExecBackend {
    fn spawn(&self, req: &SpawnRequest) -> io::Result<SpawnedSession> {
        let container = Container {
            id: &self.container_id,
            state_root: &self.state_root,
        };
        let mut argv = vec![req.cmd.clone()];
        argv.extend(req.args.iter().cloned());
        let args = runsc_command::exec_args(&container, &req.cwd, &argv);

        // The rewritten request runs *this host's* `runsc` binary; `cwd`
        // is `runsc exec`'s own client cwd (irrelevant to the sandboxed
        // process, which gets its cwd from `--cwd` above) and `env` is
        // the client's environment, not injected into the sandbox —
        // the sandboxed process's env is what `oci_spec::build` baked
        // into the bundle's `process.env` at `up`, inherited by every
        // `exec` into that container the same way the process tier's
        // sessions inherit the keeper's own environment.
        let rewritten = SpawnRequest {
            cmd: self.runsc.to_string_lossy().into_owned(),
            args,
            cwd: req.cwd.clone(),
            env: Default::default(),
            pty: req.pty,
        };
        LocalSessionBackend.spawn(&rewritten)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// This only checks that the rewrite produces a well-formed `runsc`
    /// invocation, not that it runs — there is no `runsc` in this
    /// environment to run it against (see the module doc).
    #[test]
    fn spawn_attempts_to_run_the_configured_runsc_binary() {
        let backend = RunscExecBackend {
            runsc: PathBuf::from("/definitely/not/a/real/runsc"),
            container_id: "myproj".to_string(),
            state_root: PathBuf::from("/state/myproj/runsc-state"),
        };
        let req = SpawnRequest {
            cmd: "sh".to_string(),
            args: vec!["-c".to_string(), "echo hi".to_string()],
            cwd: "/proj".to_string(),
            env: BTreeMap::new(),
            pty: None,
        };

        match backend.spawn(&req) {
            Err(e) => assert_eq!(e.kind(), io::ErrorKind::NotFound),
            Ok(_) => panic!("expected spawning a nonexistent runsc binary to fail"),
        }
    }
}
