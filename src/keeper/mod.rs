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

mod connection;
mod pty;
mod registry;
mod session;

pub mod protocol;

pub use protocol::{ExitStatus, Frame, PtySize, SessionSignal, SpawnRequest};
pub use registry::{Registry, SessionInfo};

use std::io;
use std::os::unix::net::UnixListener;
use std::sync::Arc;
use std::thread;

pub struct Keeper {
    listener: UnixListener,
    registry: Arc<Registry>,
}

impl Keeper {
    pub fn new(listener: UnixListener) -> Self {
        Self {
            listener,
            registry: Arc::new(Registry::new()),
        }
    }

    /// The live session registry, e.g. for `status`/`ps` (task 4.3) to
    /// read from a handle shared with the accept loop.
    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// Serves forever: one accepted connection is one session, start to
    /// reap, handled entirely on its own thread (connection.rs). Only
    /// returns if `accept` itself fails.
    pub fn serve(&self) -> io::Result<()> {
        loop {
            let (stream, _) = self.listener.accept()?;
            let registry = Arc::clone(&self.registry);
            thread::spawn(move || connection::handle(stream, registry));
        }
    }
}
