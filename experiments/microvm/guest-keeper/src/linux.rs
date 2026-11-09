use crate::keeper;
use nono::{AccessMode, CapabilitySet, Sandbox, SignalMode};
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;

const SOCKET: &str = "/run/keeper/control.sock";
const SSH_SOCKET: &str = "/run/keeper/ssh.sock";

fn vsock_listener(port: u32) -> io::Result<OwnedFd> {
    // SAFETY: checked socket result is uniquely owned; sockaddr is initialized.
    unsafe {
        let raw = libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = OwnedFd::from_raw_fd(raw);
        let mut addr: libc::sockaddr_vm = std::mem::zeroed();
        addr.svm_family = libc::AF_VSOCK as _;
        addr.svm_cid = libc::VMADDR_CID_ANY;
        addr.svm_port = port;
        if libc::bind(
            raw,
            (&addr as *const libc::sockaddr_vm).cast(),
            std::mem::size_of_val(&addr) as _,
        ) != 0
            || libc::listen(raw, 16) != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(fd)
    }
}

fn relay(mut incoming: UnixStream, path: &str) -> io::Result<()> {
    // UnixStream is used only for fd I/O here, never AF_UNIX address queries.
    // Both directions retain EOF independently. No protocol parsing in the relay.
    let mut outgoing = UnixStream::connect(path)?;
    let mut reader = incoming.try_clone()?;
    let mut writer = outgoing.try_clone()?;
    let upstream = std::thread::spawn(move || {
        let result = io::copy(&mut reader, &mut writer);
        let _ = writer.shutdown(Shutdown::Write);
        result
    });
    let downstream = io::copy(&mut outgoing, &mut incoming);
    let _ = incoming.shutdown(Shutdown::Write);
    // Wake a reader if the keeper closed after sending Exit without host EOF.
    let _ = incoming.shutdown(Shutdown::Read);
    upstream
        .join()
        .map_err(|_| io::Error::other("relay panicked"))??;
    downstream?;
    Ok(())
}

fn serve_transport(transport: OwnedFd, path: &'static str) -> io::Result<()> {
    loop {
        // SAFETY: accept4 returns a new owned fd. Only stream byte I/O is used.
        let fd = unsafe {
            libc::accept4(
                transport.as_raw_fd(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                libc::SOCK_CLOEXEC,
            )
        };
        if fd < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        let stream = unsafe { UnixStream::from_raw_fd(fd) };
        std::thread::spawn(move || {
            if let Err(e) = relay(stream, path) {
                eprintln!("guest relay: {e}");
            }
        });
    }
}

/// A small HTTP identity fixture, not the later Redis/build/service workload.
fn http_fixture(listener: std::net::TcpListener, id: String) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { return };
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(3)));
        let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(3)));
        let mut header = Vec::new();
        let mut byte = [0];
        while header.len() < 1024 && !header.ends_with(b"\r\n\r\n") {
            if stream.read(&mut byte).ok() != Some(1) {
                break;
            }
            header.push(byte[0]);
        }
        if !header.ends_with(b"\r\n\r\n") {
            continue;
        }
        let body = format!("instance={id}\n");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
    }
}

pub(super) fn run() -> Result<(), Box<dyn std::error::Error>> {
    let listener = UnixListener::bind(SOCKET)?;
    let transport = vsock_listener(1025)?;
    let seed = crate::bootstrap::read("/dev/vdb")?;
    let ssh = if let Some(seed) = seed {
        let key = russh::keys::PrivateKey::from_openssh(&seed.host_key)?;
        let authorized = russh::keys::ssh_key::PublicKey::from_openssh(&seed.authorized_key)?;
        let service = std::net::TcpListener::bind("127.0.0.1:8080")?;
        let blocked = std::net::TcpListener::bind("127.0.0.1:8081")?;
        // A live denied destination prevents a closed port masquerading as policy.
        drop(std::net::TcpStream::connect("127.0.0.1:8081")?);
        std::fs::write("/workspace/instance-id", &seed.instance_id)?;
        Some((
            UnixListener::bind(SSH_SOCKET)?,
            vsock_listener(1026)?,
            key,
            authorized,
            service,
            blocked,
            seed.instance_id,
        ))
    } else {
        None
    };
    // Prove these targets exist and are usable before restriction. In
    // particular an executing binary would fail with ETXTBSY even unconfined.
    std::fs::read("/private/credential")?;
    drop(
        std::fs::OpenOptions::new()
            .write(true)
            .open("/bin/readonly-data")?,
    );
    drop(std::net::TcpListener::bind("0.0.0.0:12345")?);
    let mut caps = CapabilitySet::new()
        .allow_path("/bin", AccessMode::Read)?
        .allow_path("/workspace", AccessMode::ReadWrite)?
        .allow_path("/home/devcroft", AccessMode::ReadWrite)?
        .allow_path("/tmp", AccessMode::ReadWrite)?
        .allow_path("/dev/pts", AccessMode::ReadWrite)?
        .allow_file("/dev/ptmx", AccessMode::ReadWrite)?
        .allow_file("/dev/null", AccessMode::ReadWrite)?
        .set_signal_mode(SignalMode::AllowSameSandbox)
        .block_network();
    if std::path::Path::new("/private-base/rootfs/.baseline-ready").is_file() {
        // The base and bound workspace must be disjoint Landlock subtrees.
        caps = caps.allow_path("/private-base/rootfs", AccessMode::Read)?;
    }
    if std::path::Path::new("/toolchain/usr/bin/gcc").is_file() {
        caps = caps
            .allow_path("/toolchain", AccessMode::Read)?
            // This procfs belongs to this VM alone, like a project's own PID
            // namespace. Redis' forked ARM64 safety check needs its own smaps;
            // a /proc/self rule would canonicalize to the keeper's PID only.
            .allow_path("/proc", AccessMode::Read)?
            .allow_tcp_connect(6379)
            .allow_tcp_bind(6379);
    }
    if ssh.is_some() {
        caps = caps.allow_tcp_connect(8080);
    }
    if std::path::Path::new("/toolchain/editor/node_modules/code-server/out/node/entry.js")
        .is_file()
        || std::path::Path::new("/toolchain/usr/bin/code-server").is_file()
    {
        caps = caps.allow_tcp_connect(9090).allow_tcp_bind(9090);
    }
    Sandbox::apply_landlock(&caps)?;
    // Denials must have a working baseline: PID 1 created these fixtures.
    let denials = [
        std::fs::read("/private/credential").map(|_| ()),
        std::fs::OpenOptions::new()
            .write(true)
            .open("/bin/readonly-data")
            .map(|_| ()),
        std::net::TcpListener::bind("0.0.0.0:12345").map(|_| ()),
    ];
    for denial in denials {
        if !matches!(denial, Err(e) if e.kind() == io::ErrorKind::PermissionDenied) {
            return Err("nono negative control did not return permission denied".into());
        }
    }
    std::fs::write("/workspace/allowed", b"nono-positive")?;
    std::env::set_current_dir("/workspace")?;
    // Both listeners and keys were prepared while single-threaded. The actual
    // reused SSH server, its tokio workers and all session/SFTP code inherit nono.
    let _blocked_destination = if let Some((
        ssh_listener,
        ssh_transport,
        key,
        authorized,
        http,
        blocked,
        id,
    )) = ssh
    {
        if !matches!(std::net::TcpStream::connect("127.0.0.1:8081"), Err(e) if e.kind() == io::ErrorKind::PermissionDenied)
        {
            return Err("nono did not deny the live TCP destination".into());
        }
        crate::ssh_server::spawn(
            ssh_listener,
            key,
            authorized,
            Arc::new(keeper::LocalSessionBackend),
        );
        std::thread::spawn(move || http_fixture(http, id));
        std::thread::spawn(move || {
            if let Err(e) = serve_transport(ssh_transport, SSH_SOCKET) {
                eprintln!("guest SSH transport: {e}");
                std::process::exit(1);
            }
        });
        eprintln!("guest SSH: nono applied; serving port 1026; guest loopback 8080");
        Some(blocked)
    } else {
        None
    };
    let service = keeper::Keeper::new(listener, Arc::new(keeper::LocalSessionBackend));
    std::thread::spawn(move || {
        if let Err(e) = service.serve() {
            eprintln!("guest keeper: {e}");
            std::process::exit(1);
        }
    });
    eprintln!("guest keeper: nono applied; serving port 1025");
    // Readiness is only published after restriction and the keeper accept loop.
    std::fs::write("/tmp/keeper-ready", b"ready")?;
    serve_transport(transport, SOCKET)?;
    Ok(())
}
