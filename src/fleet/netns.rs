//! Per-agent network namespace construction — the first implemented
//! slice of `add-linux-agent-fleet`, and the one its `service-ports`
//! capability rests on entirely.
//!
//! This is what lets N agents each run their own Postgres on `5432`
//! without colliding: each agent gets its own network namespace, and so
//! its own port table. Two agents binding `5432` are binding two
//! different `5432`s. Nothing negotiates, allocates, or rewrites
//! anything — see `specs/service-ports/spec.md`, which requires exactly
//! that the in-namespace port be used unchanged in every agent.
//!
//! **Built before the rest of fleet on purpose.** The D5 spike found
//! that this half depends on none of fleet's open questions: it needs no
//! forwarding helper (pasta/slirp4netns), no `/dev/net/tun`, and no
//! privilege beyond the unprivileged user namespace. D5 gates *egress* —
//! reaching the proxy, reaching a registry, the optional host-side port
//! mapping — not port isolation. See that change's design.md under D5.

use std::io;

/// Enter a fresh user + network namespace.
///
/// The user namespace comes first and is what makes the rest
/// unprivileged: a process that creates one holds a full capability set
/// *inside it*, including the `CAP_NET_ADMIN` that [`bring_loopback_up`]
/// needs, without any privilege on the host and without writing a uid
/// map first (verified live — the ioctl below succeeds immediately after
/// this call). `user_namespaces(7)` guarantees the ordering: given
/// several `CLONE_NEW*` flags in one call, the user namespace is created
/// first and owns the others.
///
/// Irreversible for this process, like every other restriction devcroft
/// applies — callers run it in a child they are willing to lose, which
/// is why `__netns_probe` exists as its own re-exec rather than being a
/// function `doctor` calls in-process.
#[cfg(target_os = "linux")]
pub fn enter_network_namespace() -> io::Result<()> {
    if unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn enter_network_namespace() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "per-agent network namespaces are Linux-only; fleet does not run on this platform",
    ))
}

// `SIOCGIFFLAGS`/`SIOCSIFFLAGS` from <linux/sockios.h>. Not in `libc`'s
// exported constants for every target, and the two values are stable
// kernel ABI, so they are written out rather than derived.
#[cfg(target_os = "linux")]
const SIOCGIFFLAGS: libc::c_ulong = 0x8913;
#[cfg(target_os = "linux")]
const SIOCSIFFLAGS: libc::c_ulong = 0x8914;

/// `struct ifreq` — 16 bytes of interface name followed by a 24-byte
/// union, of which only the leading `c_short` of flags is used here.
#[cfg(target_os = "linux")]
#[repr(C)]
struct IfReq {
    name: [libc::c_char; libc::IF_NAMESIZE],
    flags: libc::c_short,
    _union_tail: [u8; 22],
}

/// Bring this network namespace's loopback interface up.
///
/// **Load-bearing, and easy to leave out.** A fresh network namespace
/// contains a loopback *device*, which is what makes "an empty netns has
/// loopback" sound true — but it is `DOWN` with no address. The failure
/// that produces is the worst shape available:
///
/// ```text
/// lo DOWN:  bind(127.0.0.1:5432) -> OK,  connect() -> ENETUNREACH
/// lo UP:    bind                 -> OK,  connect() -> OK
/// ```
///
/// A service would therefore *start*, report itself healthy, and be
/// silently unreachable — precisely what `add-flox-services` was written
/// to prevent, arriving by a different route. Measured, not assumed; it
/// is why fleet's task list now carries a reachability test rather than
/// a bind test, since asserting `bind()` succeeded passes against the
/// broken case.
///
/// Uses `ioctl` rather than shelling out to `ip`: an external
/// `iproute2` would be a host binary dependency in a project whose whole
/// premise is that the toolchain comes from a declared closure, and
/// `SIOCSIFFLAGS` is stable kernel ABI that needs no package at all.
#[cfg(target_os = "linux")]
pub fn bring_loopback_up() -> io::Result<()> {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let result = set_loopback_up_flag(fd);
    // SAFETY: `fd` is ours, opened just above and not shared.
    unsafe { libc::close(fd) };
    result
}

#[cfg(target_os = "linux")]
fn set_loopback_up_flag(fd: libc::c_int) -> io::Result<()> {
    let mut req = IfReq {
        name: [0; libc::IF_NAMESIZE],
        flags: 0,
        _union_tail: [0; 22],
    };
    for (slot, byte) in req.name.iter_mut().zip(b"lo") {
        *slot = *byte as libc::c_char;
    }

    // Read-modify-write rather than assigning `IFF_UP` outright: the
    // kernel keeps other flags in the same field, and clobbering them
    // would be a silent change to state this function has no business
    // touching.
    // SAFETY: `req` is a correctly sized, initialized `ifreq`, and `fd`
    // is a valid AF_INET socket — the two things both ioctls require.
    if unsafe { libc::ioctl(fd, SIOCGIFFLAGS, &mut req) } < 0 {
        return Err(io::Error::last_os_error());
    }
    req.flags |= (libc::IFF_UP | libc::IFF_RUNNING) as libc::c_short;
    if unsafe { libc::ioctl(fd, SIOCSIFFLAGS, &req) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn bring_loopback_up() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "loopback configuration is Linux-only",
    ))
}

/// Whether this host can give an agent its own network namespace at all.
///
/// Probed by running the real thing in a throwaway child, never by
/// inspecting a sysctl or a kernel version: the container runtime's
/// seccomp profile, an AppArmor policy restricting unprivileged user
/// namespaces (Ubuntu 23.10+), and `max_user_namespaces` can each
/// independently deny this, and no single readable value predicts all
/// three. Same reasoning `policy::backend_support` already applies to
/// Landlock — a real attempt is the only honest probe.
///
/// The caller supplies the path to this binary, which is re-exec'd as
/// `__netns_probe`; that subcommand performs the irreversible namespace
/// entry in a process nobody minds losing.
pub fn probe(exe: &std::path::Path) -> io::Result<bool> {
    let out = std::process::Command::new(exe)
        .arg("__netns_probe")
        .output()?;
    Ok(out.status.success())
}
