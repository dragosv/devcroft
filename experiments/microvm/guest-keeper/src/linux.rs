use crate::keeper;
use nono::{AccessMode, CapabilitySet, Sandbox, SignalMode};
use std::io;
use std::net::Shutdown;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;

const SOCKET: &str = "/run/keeper/control.sock";

fn vsock_listener() -> io::Result<OwnedFd> {
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
        addr.svm_port = 1025;
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

fn relay(mut incoming: UnixStream) -> io::Result<()> {
    // UnixStream is used only for fd I/O here, never AF_UNIX address queries.
    // Both directions retain EOF independently. No protocol parsing in the relay.
    let mut outgoing = UnixStream::connect(SOCKET)?;
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

pub(super) fn run() -> Result<(), Box<dyn std::error::Error>> {
    let listener = UnixListener::bind(SOCKET)?;
    let transport = vsock_listener()?;
    // Prove these targets exist and are usable before restriction. In
    // particular an executing binary would fail with ETXTBSY even unconfined.
    std::fs::read("/private/credential")?;
    drop(
        std::fs::OpenOptions::new()
            .write(true)
            .open("/bin/readonly-data")?,
    );
    drop(std::net::TcpListener::bind("0.0.0.0:12345")?);
    let caps = CapabilitySet::new()
        .allow_path("/bin", AccessMode::Read)?
        .allow_path("/workspace", AccessMode::ReadWrite)?
        .allow_path("/home/devcroft", AccessMode::ReadWrite)?
        .allow_path("/tmp", AccessMode::ReadWrite)?
        .allow_path("/dev/pts", AccessMode::ReadWrite)?
        .allow_file("/dev/ptmx", AccessMode::ReadWrite)?
        .allow_file("/dev/null", AccessMode::ReadWrite)?
        .set_signal_mode(SignalMode::AllowSameSandbox)
        .block_network();
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
    loop {
        // SAFETY: accept4 returns a new owned connected fd or an error. The
        // relay only needs byte-stream operations, which work with AF_VSOCK.
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
            return Err(error.into());
        }
        let stream = unsafe { UnixStream::from_raw_fd(fd) };
        std::thread::spawn(move || {
            if let Err(e) = relay(stream) {
                eprintln!("guest relay: {e}");
            }
        });
    }
}
