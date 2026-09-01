//! Per-sandbox mount namespace construction (`add-mount-isolation`).
//!
//! Closes a gap `netns` cannot: Landlock's network rules cover TCP for
//! AF_INET/AF_INET6 only, so a `connect()` to a unix socket falls through
//! to ordinary filesystem permissions and is reachable from inside a
//! sandbox whose compiled policy grants none of it — measured live
//! against the nix daemon socket (`tests/unix_socket_not_mediated.rs`,
//! proposal.md). No Landlock ABI expresses AF_UNIX, so the fix is not a
//! rule: it is not having the path in the sandbox's mount view at all.
//!
//! This module is the namespace primitive alone — entering a private
//! mount namespace and making its propagation private, mirroring
//! `netns`'s own shape (`enter_network_namespace`, `probe`) and its
//! reasoning for being built and testable independently of the mount plan
//! that consumes it (task 2, "The mount plan"). Constructing the view
//! itself — which paths get bind-mounted, from the compiled policy — is
//! that later task's job, not this module's.

use std::io;
use std::io::Write;

/// Enter a fresh user + mount namespace, with an identity uid/gid mapping
/// established so filesystem operations inside it behave normally.
///
/// The user namespace comes first, for the same reason
/// [`crate::fleet::netns::enter_network_namespace`] documents: it is what
/// makes the rest unprivileged, granting a full capability set *inside
/// it* — including the `CAP_SYS_ADMIN` the mount and remount calls this
/// module's other functions make require — without any privilege on the
/// host.
///
/// **The uid/gid mapping is not optional, and `netns` never needed one.**
/// `user_namespaces(7)`: a freshly created user namespace has no mapping
/// at all, so this process's uid/gid resolve to the overflow id (65534)
/// both inside and out until one is written. Networking never looks at
/// that, which is why `enter_network_namespace` gets away with skipping
/// it — but a `tmpfs` mount does, and creating a file on one from an
/// unmapped id fails with `EOVERFLOW`, not a permission error. Measured
/// live while building this module's own isolation test: `write()`
/// against a marker file on a freshly mounted `tmpfs`, right after
/// `unshare`, failed with exactly that before this mapping was added.
///
/// Maps the real uid/gid to `0` inside the namespace — the same
/// "fake root" convention bubblewrap and `unshare --map-root-user` use
/// (design.md M2 names bubblewrap as the reference for what a working
/// mount setup needs). Cosmetic for what the process sees as its own
/// identity inside the namespace only: outside-facing DAC checks against
/// host files still resolve through the mapping back to the real uid, so
/// this changes nothing about what the sandboxed process can actually
/// read or write on the host (`up.rs`'s own comment on
/// `enter_network_namespace` makes the identical point for `CLONE_NEWUSER`
/// generally). `setgroups` must be denied before `gid_map` is written —
/// the kernel refuses an unprivileged write to `gid_map` otherwise, since
/// without that restriction a process could use supplementary groups to
/// gain privileges the mapping did not actually grant it.
///
/// **Callers that also want a network namespace must not call
/// [`crate::fleet::netns::enter_network_namespace`] afterwards.** A
/// process may create a user namespace only once via `unshare`
/// (`user_namespaces(7)`); a second `unshare(CLONE_NEWUSER)` call fails.
/// Combining flags into one `unshare()` call — `CLONE_NEWUSER |
/// CLONE_NEWNS`, optionally `| CLONE_NEWNET` — is the caller's
/// responsibility; this function's own call covers the mount-only case
/// and is what the standalone probe and tests below exercise.
#[cfg(target_os = "linux")]
pub fn enter_mount_namespace() -> io::Result<()> {
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    if unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNS) } != 0 {
        return Err(io::Error::last_os_error());
    }
    write_id_map("/proc/self/setgroups", "deny")?;
    write_id_map("/proc/self/uid_map", &format!("0 {uid} 1"))?;
    write_id_map("/proc/self/gid_map", &format!("0 {gid} 1"))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn write_id_map(path: &str, contents: &str) -> io::Result<()> {
    std::fs::File::create(path)?.write_all(contents.as_bytes())
}

#[cfg(not(target_os = "linux"))]
pub fn enter_mount_namespace() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "mount namespaces are Linux-only; this platform has no equivalent \
         (see docs/threat-model.md)",
    ))
}

/// Make this mount namespace's propagation private, recursively, starting
/// from `/` (design.md M1: `MS_REC | MS_PRIVATE`).
///
/// **Load-bearing, and must run before any other mount in this
/// namespace.** A fresh mount namespace inherits its parent's propagation
/// settings, which on most Linux distributions are `shared` — a mount
/// made without this call first would propagate into the host's own
/// namespace (and vice versa), which is both a correctness bug (the
/// sandbox's private `/tmp`, its narrowed `/nix` view, would leak) and
/// exactly the kind of silent scope-widening this whole change exists to
/// prevent. `MS_REC` covers every mount already visible under `/`, not
/// just `/` itself, since the inherited tree can be arbitrarily deep.
#[cfg(target_os = "linux")]
pub fn make_propagation_private() -> io::Result<()> {
    let root = c"/";
    // SAFETY: `root` is a valid, NUL-terminated path; the remaining
    // arguments are null as the private-propagation remount requires no
    // source, filesystem type, or data. This call needs `CAP_SYS_ADMIN`,
    // which the user namespace `enter_mount_namespace` already entered
    // grants inside itself.
    let ret = unsafe {
        libc::mount(
            std::ptr::null(),
            root.as_ptr(),
            std::ptr::null(),
            (libc::MS_REC | libc::MS_PRIVATE) as libc::c_ulong,
            std::ptr::null(),
        )
    };
    if ret != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn make_propagation_private() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "mount namespaces are Linux-only; this platform has no equivalent \
         (see docs/threat-model.md)",
    ))
}

/// Whether this host can give a sandbox its own mount namespace at all.
///
/// Probed by running the real thing in a throwaway child, never by
/// inspecting a sysctl or kernel version — the same reasoning
/// [`crate::fleet::netns::probe`] already applies for network namespaces,
/// since both rest on the identical unprivileged user namespace and can
/// each be independently denied by seccomp, AppArmor, or
/// `max_user_namespaces`. `doctor` reports both from this one probe
/// family rather than as separate capabilities (design.md M4, spec:
/// "Diagnosis before the attempt").
///
/// The caller supplies the path to this binary, which is re-exec'd as
/// `__mount_probe`; that subcommand performs the irreversible namespace
/// entry in a process nobody minds losing.
pub fn probe(exe: &std::path::Path) -> io::Result<bool> {
    let out = std::process::Command::new(exe)
        .arg("__mount_probe")
        .output()?;
    Ok(out.status.success())
}
