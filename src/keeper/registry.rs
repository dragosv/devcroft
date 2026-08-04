//! The keeper's in-memory session registry (task 4.1). Tracks every session
//! currently spawned so `status`/`ps` (task 4.3) can list them and so
//! `connection::handle` can tell whether a session it is watching has
//! already been reaped before deciding to escalate a signal.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// Always the child's own pid: every session is spawned as its own
    /// process-group leader (`setsid` for pty sessions, `setpgid(0, 0)`
    /// for piped ones), so `kill(-pgid, sig)` reaches the whole job
    /// without risk of hitting the keeper itself.
    pub pgid: libc::pid_t,
    pub command: String,
    pub started: SystemTime,
}

#[derive(Default)]
pub struct Registry {
    sessions: Mutex<HashMap<u64, SessionInfo>>,
    next_id: AtomicU64,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a spawned session and returns its id. Ids start at 1 and
    /// are never reused within a keeper's lifetime.
    pub fn insert(&self, pgid: libc::pid_t, command: String) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.sessions.lock().unwrap().insert(
            id,
            SessionInfo {
                pgid,
                command,
                started: SystemTime::now(),
            },
        );
        id
    }

    pub fn remove(&self, id: u64) -> Option<SessionInfo> {
        self.sessions.lock().unwrap().remove(&id)
    }

    pub fn contains(&self, id: u64) -> bool {
        self.sessions.lock().unwrap().contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn snapshot(&self) -> Vec<(u64, SessionInfo)> {
        self.sessions
            .lock()
            .unwrap()
            .iter()
            .map(|(id, info)| (*id, info.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_monotonic_and_never_reused() {
        let registry = Registry::new();
        let a = registry.insert(100, "sleep 1".to_string());
        let b = registry.insert(200, "sleep 2".to_string());
        assert_ne!(a, b);
        registry.remove(a);
        let c = registry.insert(300, "sleep 3".to_string());
        assert_ne!(c, a);
        assert_ne!(c, b);
    }

    #[test]
    fn remove_returns_the_removed_entry_once() {
        let registry = Registry::new();
        let id = registry.insert(42, "zig version".to_string());
        assert!(registry.contains(id));

        let removed = registry.remove(id).unwrap();
        assert_eq!(removed.pgid, 42);
        assert!(!registry.contains(id));
        assert!(registry.remove(id).is_none());
    }

    #[test]
    fn snapshot_reflects_current_sessions() {
        let registry = Registry::new();
        assert!(registry.is_empty());
        let a = registry.insert(1, "a".to_string());
        let b = registry.insert(2, "b".to_string());
        let ids: Vec<u64> = registry.snapshot().into_iter().map(|(id, _)| id).collect();
        assert_eq!(registry.len(), 2);
        assert!(ids.contains(&a));
        assert!(ids.contains(&b));
    }
}
