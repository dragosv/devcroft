//! Experimental Linux boundary, installed while the helper is single-threaded.
//! No logging/bypass mode: a syscall outside the compiled list kills the process.
use crate::config::Launch;
use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

const READ: u64 = 1 << 2;
const WRITE: u64 = 1 << 1;
const READ_DIR: u64 = 1 << 3;
const IOCTL_DEV: u64 = 1 << 15;
const FS_HANDLED: u64 = (1 << 16) - 1;

#[repr(C)]
struct Ruleset {
    fs: u64,
    net: u64,
    scoped: u64,
}

#[repr(C)]
struct PathRule {
    access: u64,
    parent: i32,
}

fn checked(value: libc::c_long, operation: &str) -> Result<libc::c_long, String> {
    if value < 0 {
        Err(format!(
            "Linux confinement: {operation}: {}",
            io::Error::last_os_error()
        ))
    } else {
        Ok(value)
    }
}

fn grant(ruleset: &File, path: &Path, access: u64) -> Result<(), String> {
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let fd = checked(
        unsafe { libc::open(path.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) }.into(),
        "open rule path",
    )?;
    let file = unsafe { File::from_raw_fd(fd as i32) };
    let rule = PathRule {
        access,
        parent: file.as_raw_fd(),
    };
    checked(
        unsafe {
            libc::syscall(
                libc::SYS_landlock_add_rule,
                ruleset.as_raw_fd(),
                1,
                &rule,
                0,
            )
        },
        "add path rule",
    )?;
    Ok(())
}

pub fn apply(cfg: &Launch) -> Result<(), String> {
    let threads = std::fs::read_dir("/proc/self/task")
        .map_err(|e| e.to_string())?
        .count();
    if threads != 1 {
        return Err(format!(
            "Linux boundary must precede threads; got {threads}"
        ));
    }
    let limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    checked(
        unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) }.into(),
        "disable core files",
    )?;
    // Compile before removing filesystem authority. The JSON is part of this
    // binary, not a runtime path or an environment-controlled policy.
    #[cfg(target_arch = "aarch64")]
    let arch = seccompiler::TargetArch::aarch64;
    #[cfg(not(target_arch = "aarch64"))]
    return Err("Linux helper syscall inventory currently covers ARM64 only".into());
    #[cfg(target_arch = "aarch64")]
    let mut filters = seccompiler::compile_from_json(
        include_bytes!("../resources/seccomp-aarch64.json").as_slice(),
        arch,
    )
    .map_err(|e| format!("compile seccomp: {e}"))?;
    #[cfg(target_arch = "aarch64")]
    let filter = filters.remove("helper").ok_or("missing helper filter")?;
    let abi = checked(
        unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<u8>(),
                0,
                1,
            )
        },
        "query Landlock ABI",
    )?;
    if abi < 6 {
        return Err(format!(
            "Linux helper requires Landlock ABI >= 6; got {abi}"
        ));
    }
    // Handle all ABI-6 filesystem rights, all TCP bind/connect and both IPC
    // scopes. UDP and non-Unix sockets are killed by seccomp, not best effort.
    let attr = Ruleset {
        fs: FS_HANDLED,
        net: 3,
        scoped: 3,
    };
    let fd = checked(
        unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &attr,
                std::mem::size_of::<Ruleset>(),
                0,
            )
        },
        "create Landlock ruleset",
    )?;
    let ruleset = unsafe { File::from_raw_fd(fd as i32) };
    for artifact in cfg.artifacts() {
        grant(&ruleset, &artifact.path, READ)?;
    }
    // Only the owned private instance may create regular files/directories and
    // pathname sockets. No executable, device-node, FIFO or symlink creation.
    let instance = READ
        | WRITE
        | READ_DIR
        | (1 << 4)
        | (1 << 5)
        | (1 << 7)
        | (1 << 8)
        | (1 << 9)
        | (1 << 13)
        | (1 << 14);
    grant(&ruleset, &cfg.instance_dir, instance)?;
    grant(&ruleset, Path::new("/dev/kvm"), READ | WRITE | IOCTL_DEV)?;
    for path in ["/dev/null", "/dev/zero"] {
        grant(&ruleset, Path::new(path), READ | WRITE | IOCTL_DEV)?;
    }
    for path in ["/dev/random", "/dev/urandom"] {
        grant(&ruleset, Path::new(path), READ)?;
    }
    checked(
        unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) }.into(),
        "no_new_privs",
    )?;
    checked(
        unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset.as_raw_fd(), 0) },
        "restrict Landlock",
    )?;
    drop(ruleset);
    #[cfg(target_arch = "aarch64")]
    seccompiler::apply_filter(&filter).map_err(|e| format!("install seccomp: {e}"))?;
    eprintln!(
        "{}",
        serde_json::json!({"event":"linux-confinement-applied", "landlock_abi":abi,
        "seccomp_mode":2,"threads_before":threads,"default_action":"kill_process","inheritance":"before all VMM/device/thread constructors"})
    );
    Ok(())
}

pub fn probe(cfg: &Launch, name: &str) -> Result<(), String> {
    // Avoid core files from deliberately killed, separately launched probes.
    let limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) } != 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    apply(cfg)?;
    unsafe {
        match name {
            "parent-pid" => {
                libc::syscall(libc::SYS_getppid);
            }
            "tcp-socket" => {
                libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
            }
            "ptrace" => {
                libc::syscall(libc::SYS_ptrace, 0, 0, 0, 0);
            }
            "unix-connect" => {
                let _ = std::os::unix::net::UnixStream::connect(
                    cfg.instance_dir.join("outside-guard.sock"),
                );
            }
            "clone-process" => {
                libc::syscall(libc::SYS_clone, libc::SIGCHLD, 0, 0, 0, 0);
            }
            "exec" => {
                let args = [c"/bin/true".as_ptr(), std::ptr::null()];
                let env = [std::ptr::null::<libc::c_char>()];
                libc::execve(args[0], args.as_ptr(), env.as_ptr());
            }
            "io-uring" => {
                let mut params = [0_u64; 32];
                libc::syscall(libc::SYS_io_uring_setup, 1, params.as_mut_ptr());
            }
            _ => return Err("unknown fixed syscall probe".into()),
        }
    }
    Err("forbidden syscall unexpectedly returned".into())
}

pub fn selftest(cfg: &Launch, outside: &Path) -> Result<(), String> {
    use std::fs::OpenOptions;
    use std::io::{Read, Write};
    // This dedicated selftest has not started any threads or VMM resources.
    // Fork only here, before restriction, to reuse already-hashed immutable
    // inputs. A cold exec per probe would repeatedly hash multi-GiB toolchains.
    if std::fs::read_dir("/proc/self/task")
        .map_err(|e| e.to_string())?
        .count()
        != 1
    {
        return Err("syscall probes require a single-threaded parent".into());
    }
    let guard = cfg.instance_dir.join("outside-guard.sock");
    let listener = std::os::unix::net::UnixListener::bind(&guard).map_err(|e| e.to_string())?;
    std::os::unix::net::UnixStream::connect(&guard).map_err(|e| e.to_string())?;
    drop(std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?);
    let mut probes = Vec::new();
    for name in [
        "parent-pid",
        "tcp-socket",
        "ptrace",
        "unix-connect",
        "clone-process",
        "exec",
        "io-uring",
    ] {
        let pid = unsafe { libc::fork() };
        checked(pid.into(), "fork fixed syscall probe")?;
        if pid == 0 {
            // The fixed child receives no ambient preopened descriptors.
            let result = crate::sandbox::close_inherited_fds().and_then(|_| probe(cfg, name));
            if let Err(error) = result {
                eprintln!("{error}");
            }
            unsafe {
                libc::_exit(70);
            }
        }
        let mut status = 0;
        loop {
            let result = unsafe { libc::waitpid(pid, &mut status, 0) };
            if result == pid {
                break;
            }
            if result < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!(
                "wait syscall probe: {}",
                io::Error::last_os_error()
            ));
        }
        let signal = libc::WIFSIGNALED(status).then(|| libc::WTERMSIG(status));
        probes.push(
            serde_json::json!({"name":name,"signal":signal,"killed":signal==Some(libc::SIGSYS)}),
        );
    }
    drop(listener);
    std::fs::remove_file(&guard).map_err(|e| e.to_string())?;
    // Positive controls before restriction prevent missing fixtures from
    // masquerading as successful denials.
    File::open(outside).map_err(|e| e.to_string())?;
    OpenOptions::new()
        .write(true)
        .open(&cfg.runtime.path)
        .map_err(|e| e.to_string())?;
    let tcp_denied = probes.iter().all(|p| p["killed"] == true);
    apply(cfg)?;
    let outside_denied =
        matches!(File::open(outside), Err(e) if e.kind()==io::ErrorKind::PermissionDenied);
    let runtime_write_denied = matches!(OpenOptions::new().write(true).open(&cfg.runtime.path), Err(e) if e.kind()==io::ErrorKind::PermissionDenied);
    let mut signature = [0; 20];
    let readonly_input_works = File::open(&cfg.runtime.path)
        .and_then(|mut f| f.read_exact(&mut signature))
        .is_ok()
        && &signature == b"DEVCROFT_RUNTIME_V1\0";
    let writable_state_works = File::create(cfg.instance_dir.join("sandbox-positive"))
        .and_then(|mut f| f.write_all(b"positive-control"))
        .is_ok();
    let unix_works = [cfg.control_socket(), cfg.keeper_socket(), cfg.ssh_socket()]
        .into_iter()
        .all(|p| std::os::unix::net::UnixListener::bind(p).is_ok());
    let thread_works = std::thread::spawn(|| 7).join().ok() == Some(7);
    let passed = outside_denied
        && runtime_write_denied
        && tcp_denied
        && readonly_input_works
        && writable_state_works
        && unix_works
        && thread_works;
    println!(
        "{}",
        serde_json::json!({"outside_denied":outside_denied,"runtime_write_denied":runtime_write_denied,
        "tcp_denied":tcp_denied,"readonly_input_works":readonly_input_works,"writable_state_works":writable_state_works,
        "unix_works":unix_works,"thread_works":thread_works,"passed":passed,"forbidden_syscall_probes":probes})
    );
    if passed {
        Ok(())
    } else {
        Err("Linux confinement selftest failed".into())
    }
}
