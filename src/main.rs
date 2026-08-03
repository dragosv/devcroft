//! Spike (tasks 1.1/1.2): prove the keeper trick under `nono wrap`.
//!
//! Same binary, same three properties, run unmodified on both backends:
//! Linux/Landlock (task 1.1) and macOS/Seatbelt (task 1.2). Which backend
//! applies is decided by `nono` itself based on the host OS.
//!
//! Three properties must hold for the design in
//! `openspec/changes/add-mvp-core/design.md` to be buildable at all:
//!
//!   P1  A unix listener created BEFORE sandbox application survives the
//!       exec into the sandbox and can still accept() afterwards.
//!   P2  That socket stays reachable from OUTSIDE the boundary, so the CLI
//!       can talk to a keeper it can no longer join.
//!   P3  The keeper and every child it spawns are inside the boundary and
//!       cannot reach what the profile did not grant.
//!
//! Layout under a scratch root:
//!   inside/allowed.txt    granted   -> reads must succeed
//!   outside/secret.txt    ungranted -> reads must be denied
//!   keeper.sock           listener created pre-restriction

use std::env;
use std::fs;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let args: Vec<String> = env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("run");

    match mode {
        "run" => supervisor(),
        "keeper" => keeper(args[2].parse().expect("fd"), Path::new(&args[3])),
        "probe" => probe(Path::new(&args[2])),
        "client" => client(Path::new(&args[2]), Path::new(&args[3])),
        other => {
            eprintln!("unknown mode: {other}");
            std::process::exit(2);
        }
    }
}

/// Host side. Builds the scratch tree, binds the listener, starts an
/// out-of-sandbox client, then execs into `nono wrap`, handing the listener
/// fd across.
fn supervisor() {
    // NOT under /tmp: nono's macOS baseline grants /private/tmp read+write
    // via group:system_read_macos, which would make "outside" reachable and
    // the confinement check vacuous. Using $HOME keeps the Linux/Landlock
    // run under the same layout so both backends exercise identical paths.
    let root = PathBuf::from(env::var("HOME").unwrap())
        .join(format!(".devcroft-spike-{}", std::process::id()));
    let inside = root.join("inside");
    let outside = root.join("outside");
    fs::create_dir_all(&inside).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(inside.join("allowed.txt"), b"granted\n").unwrap();
    fs::write(outside.join("secret.txt"), b"ungranted\n").unwrap();

    let sock = root.join("keeper.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let fd = listener.as_raw_fd();

    // The client is forked BEFORE the sandbox is applied, so it is not
    // confined. It stands in for the devcroft CLI on the host.
    let exe = env::current_exe().unwrap();
    Command::new(&exe)
        .arg("client")
        .arg(&sock)
        .arg(root.join("client-report.txt"))
        .spawn()
        .expect("spawn client");

    // Rust sets FD_CLOEXEC on sockets; clear it or the fd dies at exec.
    unsafe {
        if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
            panic!("clearing FD_CLOEXEC: {}", std::io::Error::last_os_error());
        }
    }
    std::mem::forget(listener); // exec() never returns; keep the fd open

    println!("supervisor: root={} fd={fd}", root.display());

    let err = Command::new("nono")
        .arg("wrap")
        .arg("--silent")
        // nono refuses to infer CWD access non-interactively; the keeper
        // needs it declared even though the spike never touches the CWD.
        .arg("--allow-cwd")
        .arg("--allow")
        .arg(&inside)
        .arg("--read")
        .arg(exe.parent().unwrap())
        .arg("--")
        .arg(&exe)
        .arg("keeper")
        .arg(fd.to_string())
        .arg(&root)
        .exec(); // replaces this process; nono then execs the keeper

    panic!("exec into nono failed: {err}");
}

/// Post-restriction. Everything from here runs inside the boundary.
fn keeper(fd: RawFd, root: &Path) {
    let mut report = String::new();
    report.push_str("== keeper (inside the boundary) ==\n");

    // P3, self: the profile granted inside/ and nothing else.
    report.push_str(&format!(
        "P3 keeper  read inside/allowed.txt  : {}\n",
        outcome(&root.join("inside/allowed.txt"))
    ));
    report.push_str(&format!(
        "P3 keeper  read outside/secret.txt  : {}\n",
        outcome(&root.join("outside/secret.txt"))
    ));

    // P3, descendants: a child must inherit the boundary.
    let exe = env::current_exe().unwrap();
    for (label, path) in [
        ("inside/allowed.txt", root.join("inside/allowed.txt")),
        ("outside/secret.txt", root.join("outside/secret.txt")),
    ] {
        let out = Command::new(&exe).arg("probe").arg(&path).output();
        let verdict = match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            Err(e) => format!("SPAWN FAILED: {e}"),
        };
        report.push_str(&format!("P3 child   read {label:<20}: {verdict}\n"));
    }

    // The socket path was never granted: only <root>/inside was. If accept()
    // still works, path mediation does not apply to an already-open fd —
    // which is the whole reason the design can work.
    report.push_str(&format!(
        "P1 keeper  read keeper.sock (path)  : {}\n",
        outcome(&root.join("keeper.sock"))
    ));

    // P1: the inherited fd is still a working listener.
    let listener = unsafe { UnixListener::from_raw_fd(fd) };
    report.push_str("P1 inherited fd is a live listener  : OK\n");

    match listener.accept() {
        Ok((mut stream, _)) => {
            report.push_str("P2 accepted a connection from host  : OK\n");
            print!("{report}");
            stream.write_all(report.as_bytes()).unwrap();
        }
        Err(e) => {
            report.push_str(&format!("P2 accept FAILED: {e}\n"));
            print!("{report}");
            std::process::exit(1);
        }
    }
}

/// Child of the keeper. Reports whether one path is reachable.
fn probe(path: &Path) {
    println!("{}", outcome(path));
}

/// Runs on the host, outside the boundary. Proves P2 from the other side.
fn client(sock: &Path, out: &Path) {
    // Wait past the exec into nono, so connect() lands after the keeper is
    // confined. Without this the connection could queue in the backlog
    // pre-restriction and P2 would prove nothing.
    std::thread::sleep(std::time::Duration::from_millis(750));

    let mut stream = match UnixStream::connect(sock) {
        Ok(s) => s,
        Err(e) => {
            fs::write(out, format!("client: connect failed: {e}\n")).ok();
            return;
        }
    };
    let mut buf = String::new();
    stream.read_to_string(&mut buf).ok();
    fs::write(out, format!("client (outside) received:\n{buf}")).ok();
}

fn outcome(path: &Path) -> String {
    match fs::read(path) {
        Ok(_) => "ALLOWED".into(),
        Err(e) => format!("DENIED ({})", e.kind()),
    }
}
