//! State-dir layout and pid/health bookkeeping for one sandbox (task 4.2).
//! Everything here is pure filesystem/process bookkeeping — `up.rs` is
//! where it gets composed into the actual supervisor sequence.

use serde::{Deserialize, Serialize};
use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Where one sandbox's runtime state lives. Mirrors the layout CLAUDE.md
/// and design.md already document as `<state>/<name>/...`.
pub struct StatePaths {
    pub root: PathBuf,
    pub socket: PathBuf,
    pub pidfile: PathBuf,
    pub profile: PathBuf,
    pub log: PathBuf,
    pub meta: PathBuf,
    /// The ssh spec's embedded server socket: mode 0600, inside this
    /// (mode 0700 — see [`Self::new`]) state dir. Bound host-side and fd-
    /// inherited by the keeper, same as `socket` above — never read back
    /// off disk by the keeper itself.
    pub ssh_socket: PathBuf,
    /// The ssh spec's per-sandbox *ephemeral* host key: regenerated every
    /// `up`, never reused across them. Written here for at-rest storage,
    /// but — like every other file in this baseline-denied tree (see
    /// `policy::DEVCROFT_DATA_DIR`) — the keeper cannot read it back
    /// either; `up` passes the key material down directly instead (see
    /// `ssh::keys` and `up.rs`).
    pub ssh_host_key: PathBuf,
    /// The egress proxy's own pidfile (add-egress-proxy) — separate from
    /// `pidfile` because the proxy is a separate process from the keeper,
    /// spawned only when `CompiledPolicy::wants_egress_proxy()` is true,
    /// and torn down independently: it is never restricted by (and so
    /// never shares fate with) the keeper's own `apply_auto` call.
    pub proxy_pidfile: PathBuf,
    /// The egress proxy's own log — separate from `log`, which is the
    /// keeper's (and hooks', per its own `O_APPEND`/single-write
    /// discipline). Kept apart rather than shared because the two are
    /// independent processes with independent lifetimes; nothing currently
    /// requires interleaving their records in one file the way hook
    /// output and keeper spawn/exit records must interleave.
    pub proxy_log: PathBuf,
    /// An `flock(2)` mutex serializing `up`/`down`/`rm` for this one
    /// sandbox — see `acquire_lifecycle_lock`'s doc for why it exists
    /// and what it closes. Never removed by `clear_runtime_state`, `rm`'s
    /// directory removal included: an open, already-locked fd stays
    /// valid after its directory entry is unlinked (POSIX `flock` binds
    /// to the open file description, not the path), so a concurrent
    /// waiter already blocked on it is unaffected, and the next `up`
    /// simply recreates the path fresh via `O_CREAT`, acquiring an
    /// unrelated, unclaimed lock.
    pub lifecycle_lock: PathBuf,
}

impl StatePaths {
    /// The single choke point every caller reaches a state directory
    /// through — which is exactly why the name is validated *here*
    /// rather than trusted to each caller. Before this check, a raw,
    /// unvalidated string reached this join from several independent
    /// places (`cli_exec`/`cli_shell`'s own explicit-name parsing,
    /// `ssh::proxy::sandbox_name_from_host`'s SSH-hostname extraction),
    /// and a value like `../../target` survived the join to make
    /// `rm`/`down` operate outside the state root entirely — confirmed
    /// live by actually deleting a scratch directory with it. The config
    /// spec's "Name constraints" requirement was already written to cover
    /// exactly this ("every other source of a sandbox name... SHALL be
    /// held to the identical constraint"); this is where it was missing.
    ///
    /// Returns `io::ErrorKind::InvalidInput` for an invalid name — a
    /// distinct kind from filesystem/permission failures, so a caller
    /// that wants a precise "not a valid sandbox name" message (rather
    /// than devcroft's generic "state" error layer) can match on it, the
    /// way `resolve_name_arg` does before this is ever called at all, for
    /// the two commands (`down`, `rm`) most directly implicated by the
    /// traversal above.
    pub fn new(sandbox_name: &str) -> io::Result<Self> {
        if !crate::config::is_valid_name(sandbox_name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "'{sandbox_name}' is not a valid sandbox name \
                     ([a-z0-9][a-z0-9-]{{0,31}})"
                ),
            ));
        }
        let dir = data_dir()?;
        let mut paths = Self::in_dir(dir.join(sandbox_name));
        // Deliberately *not* `root.join("lifecycle.lock")` here (unlike
        // `in_dir`'s fallback below): `up` must acquire this lock before
        // it knows whether it will ever need `root` to exist at all — a
        // provider resolution failure leaves `root` never created on
        // purpose (see `up.rs`'s own comment on that; found by an earlier
        // review after this repo's own state dir accumulated 23 empty
        // leftover sandbox directories from failed `up`s). Putting the
        // lock one level up, in the shared data dir all sandboxes already
        // live under, means acquiring it never has to create — or risk
        // leaving behind — a per-sandbox directory that whole prior fix
        // exists to avoid. `ps`'s own directory scan already skips
        // non-directory entries, so a `<name>.lock` file here is inert to
        // it regardless.
        paths.lifecycle_lock = dir.join(format!("{sandbox_name}.lock"));
        Ok(paths)
    }

    /// Builds every path under a given root. `new` is the production
    /// entrypoint (root derived from `$HOME`); tests use this directly to
    /// point at a scratch dir without needing a struct literal repeated
    /// per file or touching the real `HOME`-derived data dir.
    pub fn in_dir(root: PathBuf) -> Self {
        StatePaths {
            socket: root.join("control.sock"),
            pidfile: root.join("keeper.pid"),
            profile: root.join("profile.json"),
            log: root.join("keeper.log"),
            meta: root.join("meta.json"),
            ssh_socket: root.join("ssh.sock"),
            ssh_host_key: root.join("ssh_host_ed25519_key"),
            proxy_pidfile: root.join("proxy.pid"),
            proxy_log: root.join("proxy.log"),
            // Inside `root` here (unlike `new`'s override above) because
            // every test that calls `in_dir` directly already creates
            // `root` itself before doing anything else with it — there is
            // no "don't create the directory yet" concern to preserve for
            // a caller that made the directory before this function ever
            // ran.
            lifecycle_lock: root.join("lifecycle.lock"),
            root,
        }
    }
}

/// Held for `up`/`down`/`rm`'s entire critical section — see
/// `acquire_lifecycle_lock`. Never read; its only job is to keep the
/// fd open (and so the `flock` held) until this is dropped, at which
/// point the kernel releases it as a side effect of the fd closing.
#[allow(dead_code)]
pub struct LifecycleLock(std::fs::File);

/// Serializes every lifecycle operation on one sandbox against every
/// other. Before this existed, two concurrent `up` invocations for the
/// same (not-yet-running) sandbox could both observe `Health::None`, both
/// resolve the provider and compile the policy, and both bind the control
/// socket and spawn a keeper — the second `write_pidfile` silently
/// overwrites the first's record, orphaning that first keeper's listener
/// and any sessions it already accepted, with nothing left on disk
/// pointing at it for a later `down`/`rm` to find. Found by adversarial
/// review.
///
/// `flock(2)`, not a lockfile-with-a-pid convention: the kernel releases
/// an `flock` automatically when the holding process's file descriptor
/// table is torn down for *any* reason — clean exit, panic, `SIGKILL` —
/// so a process that dies while holding this can never leave a
/// permanently stuck lock for a later invocation to wait on forever, the
/// way a hand-rolled "does a lockfile exist" check would need its own
/// staleness logic to avoid. Blocks until acquired rather than failing
/// fast: two `up`s racing for the same sandbox should serialize, not
/// have the second one error out for a condition that resolves itself in
/// milliseconds.
pub fn acquire_lifecycle_lock(path: &Path) -> io::Result<LifecycleLock> {
    use std::os::fd::AsRawFd;
    // Idempotent, and needed on a genuinely fresh install: `path` lives
    // in the shared data dir (`StatePaths::new`'s doc explains why it's
    // not inside the per-sandbox root), which nothing may have created
    // yet the very first time any sandbox is ever brought up.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // `truncate(false)`: this file's content is never read or meant to
    // carry anything — `flock` locks the inode, not any bytes in it — so
    // there is nothing to preserve, but nothing to gain by truncating
    // it either, and doing so is one more write to a file another
    // process might (harmlessly, but needlessly) be mid-`open` on.
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    // SAFETY: `file` owns a valid fd for the duration of this call, and
    // `flock` neither reads nor writes through it.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(LifecycleLock(file))
}

/// Where the client ed25519 keypair lives (ssh spec's "Key management"
/// requirement): a sibling of every sandbox's own state dir, under the
/// same data dir, rather than inside any one of them — the same keypair
/// authenticates to every sandbox, so it isn't owned by one.
pub fn client_key_paths() -> io::Result<(PathBuf, PathBuf)> {
    let dir = data_dir()?;
    Ok((dir.join("id_ed25519"), dir.join("id_ed25519.pub")))
}

/// The `~/.local/share/devcroft` root all sandboxes live under.
/// `pub(super)` so `ps` (status.rs) can enumerate every sandbox directory
/// — the one thing that needs the root itself rather than one sandbox's
/// path under it.
pub(super) fn data_dir() -> io::Result<PathBuf> {
    let home = std::env::var("HOME")
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    Ok(PathBuf::from(home).join(".local/share/devcroft"))
}

/// Recorded at `up`, alongside the compiled profile: what `status`/`ps`
/// (task 4.3) need but can't ask the keeper for — the project root (the
/// keeper itself is never told its own state dir) and the environment
/// fingerprint from that `up`, for `provider::is_stale` to compare
/// against the environment's current fingerprint. `read_only_grants` is
/// what the provider resolved at that `up` (add-nix-provider task 3.4):
/// `policy --render`/`why` are otherwise pure functions of the manifest
/// alone and have no way to show a provider's store grants, since
/// resolving a provider means running it (`flox activate`/`nix develop`),
/// which those commands don't do. `#[serde(default)]` so `meta.json`
/// written before this field existed still deserializes, simply reporting
/// no grants until the next `up` records them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub project_root: String,
    pub env_fingerprint: String,
    #[serde(default)]
    pub read_only_grants: Vec<String>,
    /// The concrete backend the isolation tier resolved to at this `up`
    /// (`"process"` — the only value since `remove-gvisor-backend`) — recorded
    /// here for the same reason `env_fingerprint` is: `status` needs it
    /// and the keeper itself is never told its own state dir, so it
    /// can't answer either. `#[serde(default = "default_resolved_backend")]`
    /// so `meta.json` written before this field existed still
    /// deserializes: every sandbox that predates the hardened tier was,
    /// by definition, a `process`-tier one.
    #[serde(default = "default_resolved_backend")]
    pub resolved_backend: String,
    /// The service names the provider declared at this `up`.
    ///
    /// Recorded because it is the only thing that makes a *missing*
    /// service reportable. `status` learns live state by querying
    /// process-compose, which can only ever describe services it
    /// accepted — so a declared service the supervisor never took, or a
    /// supervisor that died before accepting any, showed up as an empty
    /// listing indistinguishable from a sandbox declaring no services.
    /// Reconciling the live answer against this list is what turns both
    /// into something `status` can name (`services::reconcile`).
    #[serde(default)]
    pub declared_services: Vec<String>,
    /// Whether resolving this environment executed a project-defined
    /// activation hook on the host, outside any sandbox
    /// (`fix-provisioning-hooks`).
    ///
    /// Recorded rather than only printed, for the same reason
    /// `resolved_backend` is: `up` reports it once, but `status` needs
    /// to answer the same question later without re-resolving, and the
    /// keeper cannot be asked. `#[serde(default)]` so a `meta.json`
    /// written before this field existed still deserializes, reading as
    /// "no hook" until the next `up` records the truth.
    #[serde(default)]
    pub ran_activation_hook: bool,
    /// The egress proxy's bound port, when `up` started or reused one for
    /// this sandbox (add-egress-proxy) — `None` when `network.allow` is
    /// empty and no domain filtering was requested. Recorded for the same
    /// reason `resolved_backend` is: a later `up` reusing a still-live
    /// proxy across a `Health::Stale` recovery needs its port back, and
    /// the only other place it lived was this process's own now-gone
    /// memory. `#[serde(default)]` so `meta.json` written before this
    /// field existed still deserializes, reading as "no proxy" until the
    /// next `up` records the truth — correct for every sandbox that
    /// predates this change, since none of them could have started one.
    #[serde(default)]
    pub proxy_port: Option<u16>,
    /// The per-session token this sandbox's proxy requires on every
    /// request, when one is running — companion to `proxy_port` for the
    /// same reuse reason (a `Health::Stale` recovery needs it back, and
    /// this process's memory is the only other place it lived). Without
    /// this, binding the proxy to loopback is not an authorisation
    /// boundary: any local process can reach loopback, so an
    /// unauthenticated proxy lends this sandbox's allowlisted egress to
    /// anything on the host (`add-egress-proxy`, the corrected "two
    /// sandboxes with different allowlists" task). `#[serde(default)]`
    /// so `meta.json` written before this field existed still
    /// deserializes — such a sandbox's proxy (if still alive) predates
    /// authentication and gets replaced rather than reused, since
    /// `ensure_egress_proxy` requires a token to consider a proxy
    /// reusable.
    #[serde(default)]
    pub proxy_token: Option<String>,
}

fn default_resolved_backend() -> String {
    "process".to_string()
}

/// Writes via a same-directory temp file plus `rename`, not a direct
/// `std::fs::write` — found via review while adding
/// `read_only_grants` (add-nix-provider task 3.4): `ps` (status.rs)
/// enumerates and reads *every* sandbox's `meta.json`, including ones
/// belonging to an `up` that is concurrently running elsewhere, and a
/// direct write is not atomic — a reader can observe a truncated or
/// half-written file mid-write and fail to parse it. `rename` within the
/// same directory is atomic on every platform devcroft targets, so a
/// concurrent reader only ever sees the old complete file or the new
/// complete file, never a partial one.
///
/// **This is a whole-file replace, not a merge**, and its one caller
/// (`up`) builds a fresh [`Meta`] literal every time. So any field added
/// here is re-derived at every `up` or it is silently reset — there is no
/// read-modify-write anywhere in this path. Every field today is
/// re-derived, which is why that works; a field that must *persist*
/// across `up` (a sticky allocated port, say) cannot simply be added to
/// the struct, it needs the caller to read the previous value back first.
pub fn write_meta(path: &Path, meta: &Meta) -> io::Result<()> {
    let json = serde_json::to_string_pretty(meta)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

pub fn read_meta(path: &Path) -> io::Result<Option<Meta>> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// No pidfile: never started, or already cleanly torn down.
    None,
    /// pidfile names a live process and its control socket accepts
    /// connections.
    Healthy(libc::pid_t),
    /// pidfile present but the process is dead, or alive yet unresponsive
    /// (socket refuses connections) — recovery (clear runtime state, then
    /// start fresh) is needed before a plain `up` can proceed.
    Stale(libc::pid_t),
}

/// `(pid, start_time)` — `start_time` is what lets a later reader tell
/// "this pid" from "a different process that happens to have the same
/// number now", see [`process_start_time`]'s doc for why plain `kill(pid,
/// 0)` cannot.
pub fn read_pidfile(path: &Path) -> io::Result<Option<(libc::pid_t, u64)>> {
    match std::fs::read_to_string(path) {
        Ok(s) => {
            let mut parts = s.split_whitespace();
            let pid = parts.next().and_then(|p| p.parse().ok());
            let start_time = parts.next().and_then(|p| p.parse().ok());
            Ok(pid.zip(start_time))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Records `pid` alongside its current start time, read fresh from the
/// OS at write time — never trust a caller-supplied start time, since the
/// whole point is to capture what the kernel says about this specific
/// process right now.
pub fn write_pidfile(path: &Path, pid: libc::pid_t) -> io::Result<()> {
    std::fs::write(path, format!("{pid} {}", process_start_time(pid)?))
}

/// `kill(pid, 0)`: sends no signal, just checks whether the pid could be
/// signaled. `EPERM` still means a live process (just not one we own);
/// only `ESRCH` (and friends) means it's actually gone.
///
/// This alone cannot tell "the process we recorded" from "a different
/// process the kernel has since reused this pid number for" — see
/// `is_same_process`, which is what every caller that is about to
/// *signal* a recorded pid should use instead. Kept as its own function
/// (rather than folded away) because a few callers only ever care about
/// bare liveness of a pid they hold for other reasons (e.g. a test's own
/// freshly-spawned child, checked once, never persisted to disk and read
/// back across a process boundary where reuse could occur).
pub fn is_process_alive(pid: libc::pid_t) -> bool {
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// A process's start time in clock ticks since boot — Linux's
/// `/proc/[pid]/stat`, field 22 (`starttime`), per `proc(5)`. Two
/// different processes can share a pid number at different times, but
/// never the same (pid, start_time) pair at once: the kernel does not
/// reuse a pid until the previous holder has been fully reaped, and
/// start time only increases, so a stale on-disk recording can never
/// coincidentally match a new process's.
///
/// `comm` (field 2) is parenthesized and may itself contain spaces or
/// parentheses, so fields are located from the *last* `)` rather than by
/// naive whitespace splitting — the standard technique `ps`/`top` also
/// use for this file.
///
/// Non-Linux (no `/proc`): returns `0`, a sentinel `is_same_process`
/// treats as "not verifiable here" and falls back to plain liveness —
/// the same protection this project's other platform-dependent checks
/// already state honestly rather than silently degrading (`policy::
/// degraded`'s macOS domain-filtering caveat is the same shape). `0` is
/// not a value a real process can have here: it would mean starting in
/// the same clock tick as the kernel itself booted.
#[cfg(target_os = "linux")]
fn process_start_time(pid: libc::pid_t) -> io::Result<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let after_comm = stat.rfind(')').ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected /proc/{pid}/stat format: no ')' found"),
        )
    })?;
    // Field 3 (`state`) is the first field after `comm`'s closing paren;
    // field 22 (`starttime`) is therefore index 22 - 3 = 19 in a
    // zero-indexed split of everything after it.
    stat[after_comm + 1..]
        .split_whitespace()
        .nth(19)
        .and_then(|f| f.parse().ok())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("/proc/{pid}/stat has no parseable starttime field"),
            )
        })
}

#[cfg(not(target_os = "linux"))]
fn process_start_time(_pid: libc::pid_t) -> io::Result<u64> {
    Ok(0)
}

/// Whether `pid` currently identifies the same process it did when
/// `recorded_start_time` was captured (via [`write_pidfile`]) — the check
/// every caller about to *signal* a pid read back from disk must use
/// instead of [`is_process_alive`] alone, per the review that found
/// `down`/`rm`/`up --recreate` would otherwise `SIGTERM`/`SIGKILL`
/// whatever unrelated process a dead keeper's or proxy's pid had since
/// been reused for. `recorded_start_time == 0` (this platform has no
/// `/proc`, see [`process_start_time`]) falls back to bare liveness,
/// preserving this project's existing (weaker, stated) macOS posture
/// rather than claiming a guarantee this platform can't back.
pub fn is_same_process(pid: libc::pid_t, recorded_start_time: u64) -> bool {
    if recorded_start_time == 0 {
        return is_process_alive(pid);
    }
    process_start_time(pid)
        .map(|current| current == recorded_start_time)
        .unwrap_or(false)
}

/// A pidfile alone only proves a process with that pid existed *when it
/// was written* — pids get reused, so this cross-checks the recorded
/// start time (see `is_same_process`) before ever calling the result
/// `Healthy`/`Stale` rather than "gone". A successful socket connect adds
/// a second, independent signal for `Healthy` specifically, but `Stale`
/// (pid apparently alive, socket not responding) had no such backstop
/// before this — exactly the case a resurrected unrelated process could
/// otherwise be mistaken for our own.
pub fn health(paths: &StatePaths) -> io::Result<Health> {
    let Some((pid, start_time)) = read_pidfile(&paths.pidfile)? else {
        return Ok(Health::None);
    };
    if !is_same_process(pid, start_time) {
        return Ok(Health::Stale(pid));
    }
    match UnixStream::connect(&paths.socket) {
        Ok(_) => Ok(Health::Healthy(pid)),
        Err(_) => Ok(Health::Stale(pid)),
    }
}

/// Clears everything a dead or unresponsive keeper left behind so a fresh
/// `up` can bind the socket again. Deliberately leaves `profile`/`log`
/// alone: `up` recompiles and overwrites the profile unconditionally on
/// every run, and the log is append-worthy history, not runtime state.
pub fn clear_runtime_state(paths: &StatePaths) -> io::Result<()> {
    let _ = std::fs::remove_file(&paths.pidfile);
    let _ = std::fs::remove_file(&paths.socket);
    // Not the keeper's own pidfile, and not touched by `health()` above,
    // but still runtime state: a stale entry here would make a later
    // `up` believe a proxy from a previous, now-gone run is still owned
    // by this sandbox. `terminate.rs::stop_if_running` is what actually
    // kills the process before this runs.
    let _ = std::fs::remove_file(&paths.proxy_pidfile);
    Ok(())
}

/// SIGTERM, then SIGKILL if `pid` is still alive after `grace`. Used both
/// by `up --recreate` (replacing a running keeper) and by `down`/`rm`
/// (lifecycle::terminate) — the exact "escalating SIGTERM to SIGKILL
/// after a grace period" the lifecycle spec's teardown requirement names.
/// Reads `pidfile`, verifies it still names the same process it did when
/// written (`is_same_process` — a pid whose identity can't be confirmed
/// is left alone entirely, on the theory that the number may by now
/// belong to something devcroft never spawned), and only then signals
/// it. Takes the pidfile itself rather than a bare `pid_t` specifically
/// so every caller re-reads and re-verifies at the moment of signaling
/// rather than trusting a pid a caller extracted from `Health` (or from
/// its own earlier `read_pidfile` call) some indeterminate time earlier
/// — narrowing, not just moving, the reuse window the missing check
/// originally left open.
///
/// A no-op if the pidfile is absent or already stale — callers no longer
/// need their own liveness check before calling this.
pub fn terminate_and_wait(pidfile: &Path, grace: Duration) {
    let Ok(Some((pid, start_time))) = read_pidfile(pidfile) else {
        return;
    };
    if !is_same_process(pid, start_time) {
        return;
    }
    terminate_and_wait_pid(pid, start_time, grace);
}

/// The actual SIGTERM-then-SIGKILL escalation, re-checking identity (not
/// just liveness) at each step — the process could exit and its pid be
/// reused by something else *during* the grace period, not only before
/// this function was called.
fn terminate_and_wait_pid(pid: libc::pid_t, start_time: u64, grace: Duration) {
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if !is_same_process(pid, start_time) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if is_same_process(pid, start_time) {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    /// Short and hash-based rather than `name` verbatim: a unix socket
    /// path has a low, OS-enforced length ceiling (macOS's `SUN_LEN` is
    /// 104 bytes for the *whole* path) that a descriptive test name plus
    /// a deep host `TMPDIR` can blow through — confirmed failing with
    /// `InvalidInput: path must be shorter than SUN_LEN` on macOS under a
    /// long `TMPDIR`. `name` still selects the hash, so distinct test
    /// names still get distinct (collision-free in practice, for this
    /// small fixed set) directories.
    fn tempdir(name: &str) -> StatePaths {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        name.hash(&mut hasher);
        let dir = std::env::temp_dir().join(format!(
            "dcst-{:08x}-{}",
            hasher.finish() as u32,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        StatePaths::in_dir(dir)
    }

    /// The traversal this check exists to close, confirmed live at the
    /// CLI level (`devcroft rm ../VICTIM --yes` recursively deleting a
    /// directory outside the state root) and here at the unit level so
    /// the guarantee doesn't depend on remembering to check at every
    /// call site — `StatePaths::new` is the one place every caller
    /// (`rm`, `down`, `exec`, `shell`, `ssh::proxy`'s hostname parsing)
    /// necessarily passes through.
    #[test]
    fn new_rejects_path_traversal() {
        for bad in ["../../etc", "..", "foo/../bar", "/etc/passwd"] {
            match StatePaths::new(bad) {
                Err(e) => assert_eq!(
                    e.kind(),
                    io::ErrorKind::InvalidInput,
                    "{bad:?} must be rejected, not joined into a state path"
                ),
                Ok(_) => panic!("{bad:?} must be rejected, not joined into a state path"),
            }
        }
    }

    #[test]
    fn new_rejects_empty_and_uppercase_and_overlong_names() {
        assert!(StatePaths::new("").is_err());
        assert!(StatePaths::new("Has-Upper").is_err());
        assert!(StatePaths::new(&"a".repeat(33)).is_err());
    }

    #[test]
    fn new_accepts_an_ordinary_slug() {
        assert!(StatePaths::new("my-project-1").is_ok());
    }

    #[test]
    fn pidfile_roundtrips() {
        let paths = tempdir("pidfile-roundtrip");
        assert_eq!(read_pidfile(&paths.pidfile).unwrap(), None);

        // A real pid, not an arbitrary number: `write_pidfile` now reads
        // `/proc/<pid>/stat` to capture a real start time, which a made-up
        // pid has none of.
        let pid = std::process::id() as libc::pid_t;
        write_pidfile(&paths.pidfile, pid).unwrap();
        let (read_pid, start_time) = read_pidfile(&paths.pidfile).unwrap().unwrap();
        assert_eq!(read_pid, pid);
        assert!(is_same_process(pid, start_time));
    }

    #[test]
    fn is_process_alive_true_for_self_false_after_child_exits() {
        assert!(is_process_alive(std::process::id() as libc::pid_t));

        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id() as libc::pid_t;
        child.wait().unwrap();
        assert!(!is_process_alive(pid));
    }

    #[test]
    fn meta_roundtrips() {
        let paths = tempdir("meta-roundtrip");
        assert_eq!(read_meta(&paths.meta).unwrap(), None);

        let meta = Meta {
            project_root: "/proj".to_string(),
            env_fingerprint: "abc123".to_string(),
            read_only_grants: vec!["/nix/store".to_string()],
            resolved_backend: "process".to_string(),
            declared_services: Vec::new(),
            ran_activation_hook: false,
            proxy_port: None,
            proxy_token: None,
        };
        write_meta(&paths.meta, &meta).unwrap();
        assert_eq!(read_meta(&paths.meta).unwrap(), Some(meta));
    }

    #[test]
    fn write_meta_leaves_no_tmp_file_behind() {
        // Proves the write actually goes through the rename path (not
        // just a direct write that happens to succeed) — a leftover
        // `.json.tmp` would mean the rename step was skipped or failed
        // silently.
        let paths = tempdir("meta-atomic");
        let meta = Meta {
            project_root: "/proj".to_string(),
            env_fingerprint: "abc123".to_string(),
            read_only_grants: Vec::new(),
            resolved_backend: "process".to_string(),
            declared_services: Vec::new(),
            ran_activation_hook: false,
            proxy_port: None,
            proxy_token: None,
        };
        write_meta(&paths.meta, &meta).unwrap();
        assert!(!paths.meta.with_extension("json.tmp").exists());
    }

    #[test]
    fn meta_deserializes_without_read_only_grants_field() {
        // Backward compat for meta.json written before this field existed
        // (`#[serde(default)]`) — a real file from before add-nix-provider,
        // not a synthetic shape.
        let paths = tempdir("meta-old-shape");
        std::fs::write(
            &paths.meta,
            r#"{"project_root":"/proj","env_fingerprint":"abc123"}"#,
        )
        .unwrap();
        let meta = read_meta(&paths.meta).unwrap().unwrap();
        assert_eq!(meta.project_root, "/proj");
        assert!(meta.read_only_grants.is_empty());
        // Every sandbox from before the hardened tier existed was, by
        // definition, a process-tier one.
        assert_eq!(meta.resolved_backend, "process");
    }

    #[test]
    fn health_is_none_without_a_pidfile() {
        let paths = tempdir("health-none");
        assert_eq!(health(&paths).unwrap(), Health::None);
    }

    #[test]
    fn health_is_stale_when_pid_is_dead() {
        let paths = tempdir("health-stale-dead-pid");
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let dead_pid = child.id() as libc::pid_t;
        child.wait().unwrap();
        // Not `write_pidfile`: it reads the *live* process's start time,
        // which a pid already dead by this point has none of — see
        // `status.rs`'s identical test for why.
        std::fs::write(&paths.pidfile, format!("{dead_pid} 1")).unwrap();

        assert_eq!(health(&paths).unwrap(), Health::Stale(dead_pid));
    }

    #[test]
    fn health_is_stale_when_pid_alive_but_socket_unresponsive() {
        let paths = tempdir("health-stale-orphan-socket"); // socket never bound
        write_pidfile(&paths.pidfile, std::process::id() as libc::pid_t).unwrap();

        assert_eq!(
            health(&paths).unwrap(),
            Health::Stale(std::process::id() as libc::pid_t)
        );
    }

    #[test]
    fn health_is_healthy_when_pid_alive_and_socket_accepts() {
        let paths = tempdir("health-healthy");
        let _listener = UnixListener::bind(&paths.socket).unwrap();
        write_pidfile(&paths.pidfile, std::process::id() as libc::pid_t).unwrap();

        assert_eq!(
            health(&paths).unwrap(),
            Health::Healthy(std::process::id() as libc::pid_t)
        );
    }

    #[test]
    fn clear_runtime_state_removes_pidfile_and_socket_but_keeps_profile() {
        let paths = tempdir("clear-runtime-state");
        // `clear_runtime_state` just removes the file unconditionally —
        // an arbitrary, not-necessarily-real pid is fine here, unlike
        // `write_pidfile` (which now reads a real process's start time).
        std::fs::write(&paths.pidfile, "1234 1").unwrap();
        let _listener = UnixListener::bind(&paths.socket).unwrap();
        std::fs::write(&paths.profile, "{}").unwrap();

        clear_runtime_state(&paths).unwrap();

        assert!(!paths.pidfile.exists());
        assert!(!paths.socket.exists());
        assert!(paths.profile.exists());
    }

    #[test]
    fn terminate_and_wait_kills_a_live_process() {
        let paths = tempdir("terminate-live");
        let mut child = std::process::Command::new("sleep")
            .arg("100")
            .spawn()
            .unwrap();
        let pid = child.id() as libc::pid_t;
        write_pidfile(&paths.pidfile, pid).unwrap();
        assert!(is_process_alive(pid));

        // A signaled process is a zombie (kill(pid, 0) still "succeeds")
        // until something reaps it; here that's this thread, standing in
        // for init reparenting + reaping a detached keeper in production
        // (`up` never waits on the keeper it spawns either — see up.rs).
        let reaper = std::thread::spawn(move || {
            let _ = child.wait();
        });

        terminate_and_wait(&paths.pidfile, Duration::from_secs(2));
        reaper.join().unwrap();

        assert!(!is_process_alive(pid));
    }

    /// The bug this whole mechanism exists to close: a pidfile naming a
    /// pid that is very much alive, but is no longer (or never was) the
    /// process devcroft recorded — the `is_same_process` check must
    /// refuse to signal it, however "live" `kill(pid, 0)` alone would
    /// say it is.
    #[test]
    fn terminate_and_wait_does_not_signal_a_mismatched_recording() {
        let paths = tempdir("terminate-mismatch");
        let mut child = std::process::Command::new("sleep")
            .arg("100")
            .spawn()
            .unwrap();
        let pid = child.id() as libc::pid_t;
        // A pidfile naming this real, live pid but with a start time that
        // cannot be its real one — standing in for "this pid has since
        // been reused by an unrelated process" without needing to
        // actually wait for a real reuse to occur, which isn't
        // deterministically producible in a test.
        std::fs::write(&paths.pidfile, format!("{pid} 1")).unwrap();

        terminate_and_wait(&paths.pidfile, Duration::from_millis(200));

        assert!(
            is_process_alive(pid),
            "a pid whose recorded start time doesn't match must not be signaled"
        );
        child.kill().unwrap();
        let _ = child.wait();
    }

    #[test]
    fn terminate_and_wait_is_a_no_op_without_a_pidfile() {
        let paths = tempdir("terminate-absent");
        // Must not panic or error — every current caller relies on this
        // being safe to call unconditionally, with no separate liveness
        // check of their own beforehand.
        terminate_and_wait(&paths.pidfile, Duration::from_millis(50));
    }

    #[test]
    fn is_same_process_false_after_the_process_exits() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id() as libc::pid_t;
        let start_time = process_start_time(pid).unwrap_or(0);
        child.wait().unwrap();
        assert!(!is_same_process(pid, start_time));
    }
}
