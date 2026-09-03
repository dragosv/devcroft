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
#[cfg(target_os = "linux")]
use std::io::Write;
#[cfg(target_os = "linux")]
use std::os::unix::fs::FileTypeExt;

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
    enter_mount_namespace_with_network(false)
}

/// [`enter_mount_namespace`], optionally also entering a network
/// namespace in the same `unshare()` call.
///
/// **This is the "caller's responsibility" that function's own doc
/// mentions**, and the reason it exists as a separate function rather
/// than making every caller combine flags by hand: mount isolation is
/// unconditional (every sandbox gets one — spec: "SHALL NOT... fall back
/// to the host's namespace") while network isolation stays conditional
/// (`CompiledPolicy::wants_network_isolation`), so `up`'s own keeper
/// spawn needs exactly this — always `CLONE_NEWNS`, `CLONE_NEWNET` only
/// sometimes — and cannot get it by calling this module's and `netns`'s
/// own entry points back to back (a second `unshare(CLONE_NEWUSER)`
/// fails). Bringing loopback up, if a network namespace was requested,
/// is still the caller's job — [`crate::fleet::netns::bring_loopback_up`]
/// needs no namespace-creation flags of its own, only the `CLONE_NEWNET`
/// this function already entered.
#[cfg(target_os = "linux")]
pub fn enter_mount_namespace_with_network(also_network: bool) -> io::Result<()> {
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    let mut flags = libc::CLONE_NEWUSER | libc::CLONE_NEWNS;
    if also_network {
        flags |= libc::CLONE_NEWNET;
    }
    if unsafe { libc::unshare(flags) } != 0 {
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
    enter_mount_namespace_with_network(false)
}

#[cfg(not(target_os = "linux"))]
pub fn enter_mount_namespace_with_network(_also_network: bool) -> io::Result<()> {
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

/// Build the sandbox's filesystem view at `new_root` — already created,
/// currently an empty directory — and `pivot_root` into it (task group
/// 2, "The mount plan").
///
/// `new_root` becomes `/` for everything that runs after this returns:
/// the keeper itself, and every session it later spawns. Contains
/// exactly `grants` (already resolved and canonicalized by
/// [`crate::policy::CapabilityPlan::resolved_grants`] — the same
/// resolver Landlock's own grants come from, so the two cannot diverge),
/// bind-mounted at the same absolute path they have on the host, plus
/// the keeper's own unconditional system requirements this function adds
/// regardless of what the manifest granted: `/proc`, bind-mounted from
/// the host's own (needed for `/proc/self/*` to resolve correctly for
/// the keeper and for every later session — see `mount_proc`'s own doc
/// for why this is a bind rather than the fresh instance design.md
/// originally called for), and a minimal `/dev`. `/tmp` is special-cased
/// to a fresh, private `tmpfs` rather than a bind of the host's shared one
/// (task 2.1) — but only when `grants` actually contains it, since an
/// ungranted `/tmp` must stay absent from the view like anything else
/// not granted (spec: "SHALL NOT contain paths the manifest did not
/// grant").
///
/// **Must run after [`enter_mount_namespace`] and
/// [`make_propagation_private`], in that order** (design.md M1): without
/// the user+mount namespace there is no unprivileged `CAP_SYS_ADMIN` for
/// any of these `mount()` calls, and without private propagation first,
/// every mount here would leak into the host's own namespace instead of
/// staying confined to this one.
///
/// **Fails closed by construction, not by a caller-added check.** Every
/// step here is a plain `?` — the first `io::Error` stops the function
/// immediately, part-built view and all, and propagates to the caller.
/// There is no fallback path inside this function that produces a
/// working-but-weaker view; design.md M4 ("does not fall back to the
/// host's namespace") is enforced by this function simply having no
/// branch that could do that, not by a flag the caller must remember to
/// check.
#[cfg(target_os = "linux")]
pub fn construct_view(
    new_root: &std::path::Path,
    grants: &[crate::policy::ResolvedGrant],
    proxy_socket: Option<&std::path::Path>,
) -> io::Result<()> {
    let tmp_path = std::path::Path::new("/tmp");

    mount_tmpfs(new_root)?;

    // **Three phases for /tmp, in this exact order — both the ordering
    // and the split are load-bearing, found live, not cosmetic.**
    //
    // 1. Mount /tmp *before* the grants loop below, writable regardless
    //    of its eventual mode. Mounting a filesystem over a directory
    //    hides (does not unmount) whatever was already mounted
    //    underneath it — a project root created under /tmp
    //    (`mktemp`-style worktrees, ephemeral CI directories) is exactly
    //    such a case. With /tmp mounted *after* the loop instead, the
    //    private tmpfs would land directly on top of the project root's
    //    own bind mount at `<new_root>/tmp/<project>`, and every
    //    session — the keeper's own `set_current_dir(project_root)`
    //    included — would then find nothing there. Measured: `up` on a
    //    project rooted under /tmp, with /tmp also granted, failed with
    //    a bare `ENOENT`.
    // 2. Run the grants loop with /tmp still writable, so a nested
    //    grant under /tmp (the project root, again) can create its own
    //    mount point there — `mkdir`/`touch` before `mount()` needs a
    //    writable parent.
    // 3. Only *after* the loop, finalize /tmp's own mode — a
    //    non-recursive remount, so a nested grant's own, possibly more
    //    permissive mode (the project root is `ReadWrite` even when
    //    `/tmp` itself is granted `Read`) is not silently overridden.
    //    Measured live: doing this *before* step 2 instead made every
    //    nested grant under /tmp fail with `EROFS`, trying to create a
    //    mount point under an already-read-only parent.
    let tmp_mode = grants.iter().find(|g| g.path == tmp_path).map(|g| g.mode);
    if tmp_mode.is_some() {
        mount_private_tmp(new_root)?;
    }

    for grant in grants {
        if grant.path == tmp_path {
            // Handled by mount_private_tmp/finalize_tmp_mode, privately —
            // never a bind of the host's own shared /tmp (task 2.1's own
            // item, distinct from "mirror what was granted").
            continue;
        }
        bind_mount_grant(new_root, &grant.path, grant.mode)?;
    }

    if let Some(mode) = tmp_mode {
        finalize_tmp_mode(new_root, mode)?;
    }

    mount_proc(new_root)?;
    setup_dev(new_root)?;
    setup_merged_usr_compat(new_root)?;

    // M3: the sandbox's own proxy socket, granted here explicitly and
    // never through the generic `grants` loop above — the surrounding
    // state directory is baseline-denied, so it never appears in
    // `resolved_grants`, and that is exactly the case M3 exists to
    // correct: the state dir staying masked is right, the socket inside
    // it staying reachable is a separate, deliberate exception.
    if let Some(sock) = proxy_socket {
        bind_mount_grant(new_root, sock, nono::AccessMode::ReadWrite)?;
    }

    pivot_into(new_root)
}

#[cfg(target_os = "linux")]
fn path_to_cstring(path: &std::path::Path) -> io::Result<std::ffi::CString> {
    std::ffi::CString::new(std::os::unix::ffi::OsStrExt::as_bytes(path.as_os_str()))
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
}

#[cfg(target_os = "linux")]
fn mount_tmpfs(target: &std::path::Path) -> io::Result<()> {
    let target_c = path_to_cstring(target)?;
    let tmpfs = c"tmpfs";
    // SAFETY: both C strings are valid and NUL-terminated; a `tmpfs`
    // mount needs no extra data. `CAP_SYS_ADMIN` comes from the user
    // namespace already entered.
    let ret = unsafe {
        libc::mount(
            tmpfs.as_ptr(),
            target_c.as_ptr(),
            tmpfs.as_ptr(),
            0,
            std::ptr::null(),
        )
    };
    if ret != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Bind-mount `source` (real, canonical, already confirmed to exist by
/// `resolved_grants`) into `new_root` at the identical absolute path it
/// has on the host, then remount it read-only if `mode` says so.
///
/// **Two `mount()` calls for a read-only bind, not one.** The kernel
/// ignores `MS_RDONLY` on the initial `MS_BIND` mount — a documented
/// Linux quirk, not an oversight here — so making a bind read-only is a
/// bind followed by an `MS_REMOUNT`. `MS_REC` on both calls covers a
/// source directory that might itself contain nested mounts (`/nix/store`
/// measured to have none today, but nothing here should assume that
/// stays true).
#[cfg(target_os = "linux")]
fn bind_mount_grant(
    new_root: &std::path::Path,
    source: &std::path::Path,
    mode: nono::AccessMode,
) -> io::Result<()> {
    let relative = source.strip_prefix("/").unwrap_or(source);
    let target = new_root.join(relative);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let source_type = std::fs::metadata(source)?.file_type();
    if source_type.is_dir() {
        // Idempotent: a grant nested inside an already-processed one (a
        // manifest can declare both a directory and something under it)
        // may have already created this exact directory.
        std::fs::create_dir_all(&target)?;
    } else {
        // A plain `touch` — the bind mount below is what gives it content;
        // an existing file left by a previous grant is left alone.
        if !target.exists() {
            std::fs::File::create(&target)?;
        }
    }
    bind_mount(source, &target, true)?;
    // Not applied to a character/block device: the kernel refuses an
    // unprivileged `MS_REMOUNT|MS_RDONLY` on a bind-mounted device node
    // with `EPERM` — measured live against `/dev/urandom` (`policy/
    // mod.rs`'s own `KEEPER_SYSTEM_READ` baseline grant), where the
    // identical remount succeeded moments earlier for `/usr/lib` and
    // `/etc/ld.so.cache`, so this is device-node-specific, not a general
    // remount failure. Not a gap this leaves open: mount-level read-only
    // is defense in depth here, not the enforcement point — Landlock
    // still governs actual read-vs-write access once the keeper
    // self-restricts after `pivot_root` (proposal.md: "Landlock still
    // governs access to what is visible; this governs what is visible at
    // all"), and a device special file's mount-level "writability" does
    // not correspond to mutable persistent state the way a regular
    // file's does anyway.
    if mode == nono::AccessMode::Read
        && !source_type.is_char_device()
        && !source_type.is_block_device()
    {
        remount_readonly(&target, true)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn bind_mount(
    source: &std::path::Path,
    target: &std::path::Path,
    recursive: bool,
) -> io::Result<()> {
    let source_c = path_to_cstring(source)?;
    let target_c = path_to_cstring(target)?;
    let flags = libc::MS_BIND | if recursive { libc::MS_REC } else { 0 };
    // SAFETY: both C strings are valid, NUL-terminated, and outlive this
    // call; a plain bind mount needs no filesystem type or extra data.
    let ret = unsafe {
        libc::mount(
            source_c.as_ptr(),
            target_c.as_ptr(),
            std::ptr::null(),
            flags as libc::c_ulong,
            std::ptr::null(),
        )
    };
    if ret != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// `recursive`: `true` for an ordinary granted subtree, where everything
/// underneath belongs to the same grant and should uniformly become
/// read-only — the common case, and what every caller except `/tmp`'s
/// own finalization uses.
///
/// `false` for `/tmp` specifically, and the reasoning is narrower than
/// it might look. **Measured with a minimal, `nono`-free reproduction
/// (a plain `unshare` + two `mount()` calls) before relying on either
/// answer**: recursively remounting a parent tmpfs read-only
/// (`MS_REMOUNT|MS_BIND|MS_RDONLY|MS_REC`) did *not* observably affect
/// an independently bind-mounted directory already nested underneath —
/// writes into the nested mount kept succeeding, only the parent's own
/// top-level entries became read-only. So `MS_REC` on a `remount` is not
/// proven dangerous here the way an initial guess assumed. Kept
/// non-recursive anyway because it is still the narrower, more
/// literally-correct request — "make this one mount read-only", not
/// "and whatever the kernel's `MS_REC` remount semantics happen to reach
/// today" — and the safer choice does not depend on an undocumented
/// behavior continuing to hold across kernel versions.
#[cfg(target_os = "linux")]
fn remount_readonly(target: &std::path::Path, recursive: bool) -> io::Result<()> {
    let target_c = path_to_cstring(target)?;
    let mut flags = libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY;
    if recursive {
        flags |= libc::MS_REC;
    }
    // SAFETY: `target_c` is valid and NUL-terminated; a remount needs no
    // source, filesystem type, or extra data.
    let ret = unsafe {
        libc::mount(
            std::ptr::null(),
            target_c.as_ptr(),
            std::ptr::null(),
            flags as libc::c_ulong,
            std::ptr::null(),
        )
    };
    if ret != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// A fresh, private `tmpfs` at `<new_root>/tmp` — never a bind of the
/// host's own shared `/tmp` (task 2.1). Conventional `1777` permissions
/// (world-writable, sticky), matching every real `/tmp` a tool invoked
/// inside a session would expect.
///
/// **Deliberately left writable here — read-only mode, if any, is
/// finalized separately, later, by [`finalize_tmp_mode`].** A project
/// root physically nested under `/tmp` on the host (`mktemp`-style
/// worktrees, ephemeral CI directories) needs to create its *own* bind
/// mount somewhere under this tmpfs later in [`construct_view`]'s grants
/// loop, and creating a mount point — `mkdir`/`touch` on a plain path,
/// before the `mount()` call itself — needs a writable parent. Making
/// `/tmp` read-only here, before that loop runs, would fail every such
/// nested grant with `EROFS`. Measured live while fixing the ordering
/// bug this function's sibling doc already describes.
#[cfg(target_os = "linux")]
fn mount_private_tmp(new_root: &std::path::Path) -> io::Result<()> {
    let target = new_root.join("tmp");
    std::fs::create_dir_all(&target)?;
    mount_tmpfs(&target)?;
    std::fs::set_permissions(
        &target,
        std::os::unix::fs::PermissionsExt::from_mode(0o1777),
    )?;
    Ok(())
}

/// The second half of `/tmp`'s handling, run *after* the grants loop —
/// remounts `/tmp` itself read-only if that is what the manifest granted
/// (`filesystem.read`, not `filesystem.allow`), so "the view mirrors
/// what was granted" holds for `/tmp` too, not just for everything
/// routed through the generic bind-mount path. Found live: an earlier
/// version always left `/tmp` writable regardless of the grant's own
/// mode, so `policy --render` could show `Read` while the constructed
/// view stayed writable until Landlock's own restriction caught up.
///
/// **Non-recursive, deliberately** — see [`remount_readonly`]'s own doc
/// for the measurement behind that choice: a project root nested under
/// `/tmp` with its own, more permissive grant already has its own
/// separate mount by the time this runs, and the narrower request is
/// what's actually wanted regardless of how a recursive one happens to
/// behave.
#[cfg(target_os = "linux")]
fn finalize_tmp_mode(new_root: &std::path::Path, mode: nono::AccessMode) -> io::Result<()> {
    if mode == nono::AccessMode::Read {
        remount_readonly(&new_root.join("tmp"), false)?;
    }
    Ok(())
}

/// A recursive bind mount of the host's own `/proc` at `<new_root>/proc`
/// — corrected live from a first attempt at a *fresh* `procfs` instance,
/// which is what design.md originally described and turned out to be
/// wrong in a load-bearing way, not just a style choice.
///
/// **A fresh `mount("proc", ...)` needs `CAP_SYS_ADMIN` in the user
/// namespace that owns the *PID* namespace being displayed — measured
/// live: `EPERM`, on the very first real test of this code, after every
/// preceding bind mount in the same view (including other read-only
/// remounts) succeeded.** Task 0.4 deliberately did not take a private
/// PID namespace, so the PID namespace here is still the host's, owned
/// by the *initial* user namespace — not the one this process's own
/// `enter_mount_namespace` created, which is the only one this process
/// has real privilege in. A bind mount sidesteps this entirely: it does
/// not create a new procfs superblock, only exposes an existing,
/// already-mounted one at a new path, which needs nothing beyond the
/// `CAP_SYS_ADMIN` already held for every other bind mount in this view.
///
/// **This still gets `/proc/self` right for every later session, not
/// just the keeper — the property a snapshot bind of individual
/// `/proc/self/*` entries could not have provided.** `/proc/self` is a
/// magic symlink resolved fresh per reading process at lookup time,
/// resolved through whichever procfs *superblock* backs the path — bind
/// or original makes no difference, since neither creates a new
/// superblock; the bind mount here still exposes the host's one, real,
/// live procfs instance, scoped to the one pid namespace every process
/// in this view — the keeper and everything it later spawns — actually
/// runs in. A bind of individual leaves (`/proc/self/exe`, ...) would
/// have frozen to whichever pid happened to perform the bind.
///
/// **The cost is larger than design.md's first pass claimed, and that
/// claim is corrected rather than left standing.** It described the view
/// as exposing only the handful of `/proc/self/*` entries design.md
/// Open Question 1 measured as needed. That was never achievable without
/// owning the PID namespace (this measurement is what proves it), so the
/// honest statement is the one open question 2 already made about
/// process visibility in general: without a private PID namespace, a
/// full directory listing under this mount enumerates every host
/// process, exactly as it would with no mount view at all. Narrower
/// `/proc` visibility remains available only by taking PID isolation —
/// unchanged from open question 2's own conclusion, now stated for the
/// right reason.
#[cfg(target_os = "linux")]
fn mount_proc(new_root: &std::path::Path) -> io::Result<()> {
    let target = new_root.join("proc");
    std::fs::create_dir_all(&target)?;
    bind_mount(std::path::Path::new("/proc"), &target, true)
}

/// The minimal `/dev` design.md Open Question 1 measured: `null`,
/// `urandom`, `tty` bind-mounted individually from the host's real device
/// nodes (small, safe, and exactly what a bind mount is for — a device
/// special file mounted onto an empty regular-file target behaves as
/// that device, the same trick bubblewrap uses, cited as the reference
/// in design.md M2); `ptmx` and `fd`/`stdin`/`stdout`/`stderr` handled
/// separately below because neither is a plain device bind.
///
/// **`/dev/pts` itself is not built here.** It is a normal
/// `KEEPER_SYSTEM_READWRITE` baseline grant (`policy/mod.rs`), so the
/// generic `grants` loop in [`construct_view`] already bind-mounts it —
/// this function only has to make `/dev/ptmx` resolve into that.
///
/// `/dev/ptmx` is host-shape-dependent, confirmed by `policy/mod.rs`'s
/// own `KEEPER_SYSTEM_READWRITE` doc comment: on this devcontainer (and
/// "every Linux system checked" there) it is a *symlink* to
/// `pts/ptmx`, not a standalone device node — Landlock evaluates the
/// resolved target, which is why granting `/dev/pts` alone is sufficient
/// there. A mount view is a real directory tree, though, so the symlink
/// itself must physically exist for that resolution to find anything —
/// replicated here rather than assumed, checking which shape this host
/// actually has instead of hard-coding one.
#[cfg(target_os = "linux")]
fn setup_dev(new_root: &std::path::Path) -> io::Result<()> {
    let dev = new_root.join("dev");
    std::fs::create_dir_all(&dev)?;

    for name in ["null", "urandom", "tty"] {
        let source = std::path::Path::new("/dev").join(name);
        if !source.exists() {
            continue;
        }
        let target = dev.join(name);
        std::fs::File::create(&target)?;
        bind_mount(&source, &target, false)?;
    }

    let host_ptmx = std::path::Path::new("/dev/ptmx");
    if let Ok(meta) = std::fs::symlink_metadata(host_ptmx) {
        let target = dev.join("ptmx");
        if meta.file_type().is_symlink() {
            let link_target = std::fs::read_link(host_ptmx)?;
            std::os::unix::fs::symlink(&link_target, &target)?;
        } else {
            std::fs::File::create(&target)?;
            bind_mount(host_ptmx, &target, false)?;
        }
    }

    // Dynamic, not bound, for the identical reason `/proc` itself is a
    // live mount rather than a snapshot: correct only if resolved fresh
    // per reading process, which a symlink into the live `/proc` mount
    // above gives for free.
    std::os::unix::fs::symlink("/proc/self/fd", dev.join("fd"))?;
    for (name, fd) in [("stdin", 0), ("stdout", 1), ("stderr", 2)] {
        std::os::unix::fs::symlink(format!("/proc/self/fd/{fd}"), dev.join(name))?;
    }

    Ok(())
}

/// Recreate the traditional top-level `/lib`, `/lib64`, `/bin`, `/sbin`
/// symlinks inside `new_root`, on a host that has them — found live, not
/// anticipated from design.md M2's own mention of "merged-`/usr`
/// symlinks" as something bubblewrap's mount setup has to get right.
///
/// **Why this is needed even though the *targets* are already bind-mounted.**
/// `resolved_grants` canonicalizes every entry — a granted `/lib` becomes
/// its real target, `/usr/lib`, and only `/usr/lib` gets a directory and
/// a bind mount in this view. That is correct for anything that opens a
/// path *through* Landlock's own resolution, which also follows symlinks
/// to their target. It is not correct for an ELF binary's own hard-coded
/// interpreter path: a binary linked on this exact host names its
/// dynamic linker `/lib/ld-linux-aarch64.so.1` — literally, not
/// `/usr/lib/ld-linux-aarch64.so.1` — and the kernel's loader resolves
/// that path *inside the view*, where `/lib` does not exist at all
/// unless something creates it. Measured: `connect_probe` (this
/// module's own live isolation check) failed with a plain `ENOENT` on
/// exec, not a linker error, until this function existed.
///
/// Checks each name individually and only recreates it if the *host*
/// itself has it as a symlink — some hosts (or some of these four names,
/// per host) may not, and inventing one this host doesn't have would be
/// exactly the "guessing" this project measures against rather than
/// assumes.
#[cfg(target_os = "linux")]
fn setup_merged_usr_compat(new_root: &std::path::Path) -> io::Result<()> {
    for name in ["lib", "lib64", "bin", "sbin"] {
        let host_path = std::path::Path::new("/").join(name);
        let Ok(target) = std::fs::read_link(&host_path) else {
            continue;
        };
        let view_path = new_root.join(name);
        // `symlink_metadata`, not `exists()`: this must detect anything
        // already at this exact path — including a broken symlink, which
        // `exists()` (follows symlinks) would misreport as absent.
        if std::fs::symlink_metadata(&view_path).is_ok() {
            continue;
        }
        std::os::unix::fs::symlink(&target, &view_path)?;
    }
    Ok(())
}

/// `pivot_root` into `new_root`, then detach the old root.
///
/// `libc` has no safe wrapper for `pivot_root(2)` (unlike `mount`/
/// `umount2`), so this goes through the raw syscall — same pattern this
/// project already uses where the crate's coverage stops (`fleet::netns`'s
/// raw `ioctl` for `SIOCSIFFLAGS`).
///
/// Already-open file descriptors — the inherited control/SSH listener
/// sockets, in particular — stay valid across this regardless of what
/// happens to the old root's mount, since an open fd does not depend on
/// its path remaining resolvable.
#[cfg(target_os = "linux")]
fn pivot_into(new_root: &std::path::Path) -> io::Result<()> {
    const OLD_ROOT_NAME: &str = ".devcroft-old-root";
    let old_root = new_root.join(OLD_ROOT_NAME);
    std::fs::create_dir_all(&old_root)?;

    let new_root_c = path_to_cstring(new_root)?;
    let old_root_c = path_to_cstring(&old_root)?;
    // SAFETY: both C strings are valid, NUL-terminated paths; `pivot_root`
    // requires `new_root` to be a mount point (it is — `mount_tmpfs`
    // above made it one) and `old_root` to be beneath it (it is, by
    // construction).
    let ret = unsafe {
        libc::syscall(
            libc::SYS_pivot_root,
            new_root_c.as_ptr(),
            old_root_c.as_ptr(),
        )
    };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }

    std::env::set_current_dir("/")?;

    let old_root_from_new = std::path::Path::new("/").join(OLD_ROOT_NAME);
    let old_root_from_new_c = path_to_cstring(&old_root_from_new)?;
    // SAFETY: valid, NUL-terminated path; `MNT_DETACH` makes this succeed
    // even though the mount is still busy (this process's own cwd was
    // just there).
    let ret = unsafe { libc::umount2(old_root_from_new_c.as_ptr(), libc::MNT_DETACH) };
    if ret != 0 {
        return Err(io::Error::last_os_error());
    }
    let _ = std::fs::remove_dir(&old_root_from_new);

    Ok(())
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

/// The non-Linux counterpart to [`construct_view`], which exists so this
/// module compiles at all off Linux — every one of that function's steps
/// is a `mount(2)`, `pivot_root(2)` or `umount2(2)` call with no macOS
/// equivalent, which is the same reason [`enter_mount_namespace`] and
/// [`make_propagation_private`] already carry non-Linux fallbacks above.
///
/// It returns `Unsupported` rather than degrading to a partial view, for
/// the reason the Linux version's own doc gives: there is no
/// working-but-weaker view, so the only honest answer off Linux is "not
/// available". `up` already treats that answer as non-fatal on macOS
/// (`lifecycle::up`'s `isolate_filesystem` probe warns and proceeds
/// Seatbelt-only); this function never actually runs there, because that
/// probe is what gates the call.
#[cfg(not(target_os = "linux"))]
pub fn construct_view(
    _new_root: &std::path::Path,
    _grants: &[crate::policy::ResolvedGrant],
    _proxy_socket: Option<&std::path::Path>,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "filesystem views require mount namespaces, which are Linux-only; \
         this platform has no equivalent (see docs/threat-model.md)",
    ))
}
