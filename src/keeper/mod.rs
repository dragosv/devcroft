//! The keeper (task 4.1): the resident spawn server design.md decision 1
//! describes. It never binds its own listener — `up` (task 4.2) creates
//! the socket before sandbox restriction and hands it to the keeper via
//! fd inheritance (CLAUDE.md's listener-before-restriction ordering,
//! proven by the task 1.1/1.2 spike) — so `Keeper::new` only ever takes an
//! already-bound `UnixListener`.
//!
//! One OS thread per connection/session. MVP's session counts (a handful
//! of interactive shells and one-shot execs per sandbox) do not warrant an
//! async runtime, and no async executor crate is vendored in this
//! workspace to use one anyway.

pub(crate) mod connection;
mod registry;

/// Visible beyond `keeper` (task 6.3): the ssh server's channel handling
/// reuses the exact same pty allocation and spawn primitives the control
/// socket's own `connection.rs` uses, so an `exec`/`shell` session behaves
/// identically regardless of which transport it arrived over.
pub(crate) mod pty;
pub(crate) mod session;

pub mod protocol;

pub use protocol::{
    ExitStatus, Frame, PtySize, QueryResult, SessionSignal, SessionSummary, SpawnRequest,
};
pub use registry::{Registry, SessionInfo};
pub use session::{LocalSessionBackend, SessionBackend};

use std::io;
use std::os::unix::net::UnixListener;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

pub struct Keeper {
    listener: UnixListener,
    registry: Arc<Registry>,
    started: Instant,
    backend: Arc<dyn SessionBackend>,
}

impl Keeper {
    /// `backend` decides how a session actually spawns: [`LocalSessionBackend`]
    /// for the `process` tier (today's fork/exec), or a hardened backend's
    /// own implementation (e.g. `runsc exec`) for the `hardened` tier —
    /// everything else about the keeper is identical either way.
    pub fn new(listener: UnixListener, backend: Arc<dyn SessionBackend>) -> Self {
        Self {
            listener,
            registry: Arc::new(Registry::new()),
            started: Instant::now(),
            backend,
        }
    }

    /// The live session registry, e.g. for `status`/`ps` (task 4.3) to
    /// read from a handle shared with the accept loop.
    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// Serves forever: one accepted connection is either a `Query` (task
    /// 4.3's `status`/`ps`, answered directly) or a session, start to
    /// reap, handled entirely on its own thread (connection.rs). Only
    /// returns if `accept` itself fails.
    pub fn serve(&self) -> io::Result<()> {
        loop {
            let (stream, _) = self.listener.accept()?;
            let registry = Arc::clone(&self.registry);
            let started = self.started;
            let backend = Arc::clone(&self.backend);
            thread::spawn(move || connection::handle(stream, registry, started, backend));
        }
    }
}
