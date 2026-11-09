use crate::config::Launch;

pub fn selftest(cfg: &Launch, outside: &std::path::Path) -> Result<(), String> {
    use std::fs::{File, OpenOptions};
    use std::io::{ErrorKind, Write};
    // This sentinel is created by the controller, never a user's real secret.
    File::open(outside).map_err(|e| format!("backend: unreadable baseline sentinel: {e}"))?;
    apply(cfg)?;
    let outside_denied =
        matches!(File::open(outside), Err(e) if e.kind() == ErrorKind::PermissionDenied);
    let runtime_write_denied = matches!(OpenOptions::new().write(true).open(&cfg.runtime.path),
        Err(e) if e.kind() == ErrorKind::PermissionDenied);
    let tcp_denied = matches!(std::net::TcpListener::bind("127.0.0.1:0"),
        Err(e) if e.kind() == ErrorKind::PermissionDenied);
    let readonly_input_works = File::open(&cfg.runtime.path).is_ok();
    let path = cfg.instance_dir.join("sandbox-positive");
    let writable_state_works = File::create(&path)
        .and_then(|mut f| f.write_all(b"positive-control"))
        .is_ok();
    let unix_works = std::os::unix::net::UnixListener::bind(cfg.control_socket()).is_ok()
        && std::os::unix::net::UnixListener::bind(cfg.keeper_socket()).is_ok()
        && std::os::unix::net::UnixListener::bind(cfg.ssh_socket()).is_ok();
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
        serde_json::json!({"outside_denied":outside_denied,
        "runtime_write_denied":runtime_write_denied,"tcp_denied":tcp_denied,
        "readonly_input_works":readonly_input_works,"writable_state_works":writable_state_works,
        "unix_works":unix_works,"thread_works":thread_works,"passed":passed})
    );
    if passed {
        Ok(())
    } else {
        Err("backend: confinement selftest failed".into())
    }
}

pub fn close_inherited_fds() -> Result<(), String> {
    // Collect before closing: otherwise the directory iterator's own descriptor
    // would be closed underneath it. The helper is single-threaded here.
    let entries = std::fs::read_dir("/dev/fd")
        .map_err(|e| format!("backend: enumerate inherited descriptors: {e}"))?;
    let fds: Vec<i32> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str()?.parse().ok())
        .filter(|fd| *fd > 2)
        .collect();
    for fd in fds {
        unsafe {
            libc::close(fd);
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn apply(cfg: &Launch) -> Result<(), String> {
    use std::ffi::{CStr, CString};
    unsafe extern "C" {
        fn sandbox_init(
            profile: *const libc::c_char,
            flags: u64,
            error: *mut *mut libc::c_char,
        ) -> libc::c_int;
        fn sandbox_free_error(error: *mut libc::c_char);
    }
    let profile = CString::new(profile(cfg)?).map_err(|e| e.to_string())?;
    let mut error = std::ptr::null_mut();
    // Irreversible and process-wide, before libkrun creates any worker/vCPU.
    let rc = unsafe { sandbox_init(profile.as_ptr(), 0, &mut error) };
    if rc == 0 {
        return Ok(());
    }
    let detail = if error.is_null() {
        format!("return code {rc}")
    } else {
        let detail = unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned();
        unsafe {
            sandbox_free_error(error);
        }
        detail
    };
    Err(format!(
        "backend: pre-build Seatbelt confinement failed: {detail}"
    ))
}

#[cfg(not(target_os = "macos"))]
pub fn apply(_: &Launch) -> Result<(), String> {
    Err("backend: pre-vCPU host confinement is not implemented for this OS; refusing boot".into())
}

#[cfg(target_os = "macos")]
fn quoted(path: &std::path::Path) -> Result<String, String> {
    let path = path.to_str().ok_or("backend: non-UTF-8 confinement path")?;
    if path.chars().any(char::is_control) {
        return Err("backend: control character in confinement path".into());
    }
    Ok(format!(
        "\"{}\"",
        path.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

#[cfg(target_os = "macos")]
fn profile(cfg: &Launch) -> Result<String, String> {
    // Unlike hvi's post-resource-open profile this must also allow libkrun to
    // acquire its boot resources. There are no host filesystem shares, TCP/UDP
    // grants, process-exec/fork grants, or access to the operator's home.
    let mut p = String::from("(version 1)\n(deny default)\n(allow sysctl-read)\n");
    for a in cfg.artifacts() {
        p.push_str(&format!(
            "(allow file-read* (literal {}))\n",
            quoted(&a.path)?
        ));
    }
    p.push_str(&format!(
        "(allow file-read* file-write* (subpath {}))\n",
        quoted(&cfg.instance_dir)?
    ));
    // Socket creation/bind is constrained to the per-instance Unix pathname.
    for socket in [cfg.control_socket(), cfg.keeper_socket(), cfg.ssh_socket()] {
        p.push_str(&format!(
            "(allow network-bind network-inbound network-outbound (local unix-socket (path {})))\n",
            quoted(&socket)?
        ));
    }
    for path in ["/dev/null", "/dev/random", "/dev/urandom"] {
        p.push_str(&format!("(allow file-read* (literal \"{path}\"))\n"));
    }
    Ok(p)
}
