// The launch contract is deliberately identical to the libkrun experiment.
#[allow(dead_code)]
#[path = "../../libkrun/src/config.rs"]
mod config;

use config::Launch;
use hvi::config::{BootConfig, DiskConfig, Stop};
use hvi::plugin::{CpuHandle, Plugin, VmHandle};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::FileExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const REVISION: &str = env!("DEVCROFT_HVI_REVISION");

fn close_inherited_fds() -> Result<(), String> {
    let entries = std::fs::read_dir("/dev/fd").map_err(|e| e.to_string())?;
    let fds: Vec<i32> = entries
        .filter_map(Result::ok)
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

fn denied<T>(result: io::Result<T>) -> bool {
    matches!(result, Err(error) if error.kind() == io::ErrorKind::PermissionDenied)
}

// Inspect the capabilities hvi retains after resource acquisition. Inherited
// descriptors above stderr were closed before any VMM resources were opened.
fn descriptor_inventory(disks: &[(PathBuf, i32)]) -> io::Result<serde_json::Value> {
    let fds: Vec<i32> = std::fs::read_dir("/dev/fd")?
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str()?.parse().ok())
        .collect();
    let mut records = Vec::new();
    for fd in fds {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 {
            continue;
        }
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd, &mut stat) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut path = [0_i8; libc::PATH_MAX as usize];
        let path = if unsafe { libc::fcntl(fd, libc::F_GETPATH, path.as_mut_ptr()) } == 0 {
            Some(
                unsafe { std::ffi::CStr::from_ptr(path.as_ptr()) }
                    .to_string_lossy()
                    .into_owned(),
            )
        } else {
            None
        };
        records.push(
            serde_json::json!({"fd":fd, "mode":stat.st_mode & libc::S_IFMT,
            "access":flags & libc::O_ACCMODE, "path":path}),
        );
    }
    for (path, access) in disks {
        if !records.iter().any(|r| {
            r["path"].as_str() == path.to_str() && r["access"].as_i64() == Some(i64::from(*access))
        }) {
            return Err(io::Error::other(format!(
                "missing disk descriptor/access: {}",
                path.display()
            )));
        }
    }
    // No unexpected regular file may silently become an ambient capability.
    if records.iter().any(|r| {
        r["fd"].as_i64().is_some_and(|fd| fd > 2)
            && r["mode"].as_u64() == Some(u64::from(libc::S_IFREG))
            && !disks.iter().any(|(p, _)| r["path"].as_str() == p.to_str())
    }) {
        return Err(io::Error::other(
            "unexpected retained regular-file descriptor",
        ));
    }
    Ok(
        serde_json::json!({"event":"hvi-descriptor-inventory", "disks_access_checked":true,
        "unexpected_regular_files":false,"descriptors":records}),
    )
}

struct BeforeGuest {
    checked: AtomicBool,
    sentinel: PathBuf,
    runtime: PathBuf,
    state: PathBuf,
    disks: Vec<(PathBuf, i32)>,
}

impl Plugin for BeforeGuest {
    fn on_attach(&self, vm: Arc<dyn VmHandle>) -> io::Result<()> {
        if vm.has_net() {
            return Err(io::Error::other("unexpected NIC"));
        }
        eprintln!("{}", descriptor_inventory(&self.disks)?);
        File::open(&self.sentinel)?;
        OpenOptions::new().write(true).open(&self.runtime)?;
        OpenOptions::new().write(true).open(&self.state)?;
        drop(std::net::TcpListener::bind("127.0.0.1:0")?);
        eprintln!(
            "{}",
            serde_json::json!({"event":"hvi-before-confinement-positive-controls","passed":true})
        );
        Ok(())
    }

    fn on_safepoint(&self, _cpu: &dyn CpuHandle) {
        if self.checked.swap(true, Ordering::SeqCst) {
            return;
        }
        let outside = denied(File::open(&self.sentinel));
        let runtime = denied(OpenOptions::new().write(true).open(&self.runtime));
        let state = denied(OpenOptions::new().write(true).open(&self.state));
        let tcp = denied(std::net::TcpListener::bind("127.0.0.1:0"));
        let passed = outside && runtime && state && tcp;
        eprintln!(
            "{}",
            serde_json::json!({"event":"hvi-before-first-guest-entry",
            "passed":passed,"outside_denied":outside,"runtime_reopen_denied":runtime,
            "state_reopen_denied":state,"tcp_denied":tcp})
        );
        if !passed {
            std::process::exit(70);
        }
    }
}

fn selftest(cfg: &Launch, outside: &Path) -> Result<(), String> {
    let mut runtime = File::open(&cfg.runtime.path).map_err(|e| e.to_string())?;
    File::open(outside).map_err(|e| e.to_string())?;
    let state = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&cfg.state)
        .map_err(|e| e.to_string())?;
    let pos = state.metadata().map_err(|e| e.to_string())?.len() - 1;
    let mut original = [0];
    state
        .read_exact_at(&mut original, pos)
        .map_err(|e| e.to_string())?;
    let listener = UnixListener::bind(cfg.control_socket()).map_err(|e| e.to_string())?;
    let mut peer = UnixStream::connect(cfg.control_socket()).map_err(|e| e.to_string())?;
    hvi::sandbox::seatbelt::enter().map_err(|e| e.to_string())?;
    let outside_denied = denied(File::open(outside));
    let runtime_write_denied = denied(OpenOptions::new().write(true).open(&cfg.runtime.path));
    let tcp_denied = denied(std::net::TcpListener::bind("127.0.0.1:0"));
    let mut signature = [0; 20];
    let readonly_input_works =
        runtime.read_exact(&mut signature).is_ok() && &signature == b"DEVCROFT_RUNTIME_V1\0";
    let writable_state_works = state
        .write_all_at(&[original[0] ^ 1], pos)
        .and_then(|_| state.write_all_at(&original, pos))
        .and_then(|_| state.sync_all())
        .is_ok();
    let unix_works = peer.write_all(b"x").is_ok()
        && listener
            .accept()
            .and_then(|(mut incoming, _)| {
                let mut byte = [0];
                incoming.read_exact(&mut byte)?;
                if byte == [b'x'] {
                    Ok(())
                } else {
                    Err(io::Error::other("Unix byte mismatch"))
                }
            })
            .is_ok();
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
        "tcp_denied":tcp_denied,"readonly_input_works":readonly_input_works,
        "writable_state_works":writable_state_works,"unix_works":unix_works,"thread_works":thread_works,
        "passed":passed,"positive_controls":"retained pre-opened descriptors; all path reopening remains denied"})
    );
    if passed {
        Ok(())
    } else {
        Err("hvi Seatbelt selftest failed".into())
    }
}

fn entry() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() == 2 && args[1] == "provenance" {
        println!(
            "{}",
            serde_json::json!({"adapter":"hvi","revision":REVISION,"core_version":hvi::CORE_VERSION,"patches":[{"file":"hvi-vsock-fixed-ports.patch", "sha256":env!("DEVCROFT_HVI_PATCH_SHA256")}]})
        );
        return Ok(());
    }
    if args.len() == 2 && args[1] == "version" {
        println!("hvi {REVISION}");
        return Ok(());
    }
    let test = args.len() == 4 && args[1] == "sandbox-test";
    if !test && (args.len() != 3 || (args[1] != "run" && args[1] != "check")) {
        return Err("usage: devcroft-hvi-spike provenance | check LAUNCH | run LAUNCH | sandbox-test LAUNCH SENTINEL".into());
    }
    let cfg = Launch::read(Path::new(&args[2]))?;
    if args[1] == "check" {
        println!(
            "{}",
            serde_json::json!({"schema":1,"revision":REVISION,"status":"inputs-validated",
            "qualification":"not-run","nic":false,"tsi":false,"vcpus":cfg.vcpus,"memory_mib":cfg.memory_mib})
        );
        return Ok(());
    }
    close_inherited_fds()?;
    if test {
        return selftest(&cfg, Path::new(&args[3]));
    }
    // hvi opens and exclusively locks writable disks itself before sandboxing.
    // A second wrapper lock would conflict with its separate file description.
    let sentinel = cfg.instance_dir.join("before-confinement-sentinel");
    std::fs::write(&sentinel, b"owned control").map_err(|e| e.to_string())?;
    // Inputs are private experiment-owned paths. Recheck immediately before hvi
    // opens its own descriptors and enters its own process-wide profile.
    for artifact in cfg.artifacts() {
        config::verify(artifact)?;
    }
    let utf8 = |path: &Path| {
        path.to_str()
            .map(str::to_owned)
            .ok_or("disk path is not UTF-8")
    };
    let mut disks = vec![
        DiskConfig::ro(utf8(&cfg.runtime.path)?),
        DiskConfig::rw(utf8(&cfg.state)?),
    ];
    if let (Some(tools), Some(data)) = (&cfg.workload, &cfg.data) {
        disks.extend([
            DiskConfig::ro(utf8(&tools.path)?),
            DiskConfig::rw(utf8(data)?),
        ]);
    }
    let expected_disks = std::iter::once((cfg.runtime.path.clone(), libc::O_RDONLY))
        .chain(std::iter::once((cfg.state.clone(), libc::O_RDWR)))
        .chain(
            cfg.workload
                .iter()
                .map(|a| (a.path.clone(), libc::O_RDONLY)),
        )
        .chain(cfg.data.iter().map(|p| (p.clone(), libc::O_RDWR)))
        .collect();
    let cfg = BootConfig {
        kernel: std::fs::read(&cfg.kernel.path).map_err(|e| e.to_string())?,
        initramfs: Some(std::fs::read(&cfg.initramfs.path).map_err(|e| e.to_string())?),
        mem_bytes: u64::from(cfg.memory_mib) << 20,
        cmdline: "console=ttyAMA0 earlycon=pl011,0x0c000000 rdinit=/init panic=-1 reboot=k quiet"
            .into(),
        disks,
        fs_shares: vec![],
        fs: vec![],
        net: false,
        net_gateway: None,
        net_tap: None,
        net_mac: None,
        events: None,
        sandbox_id: "devcroft-experiment".into(),
        vcpus: u32::from(cfg.vcpus),
        agent_sock: Some(
            cfg.control_socket()
                .to_str()
                .ok_or("Unix path is not UTF-8")?
                .into(),
        ),
        agent_ports: vec![
            (utf8(&cfg.keeper_socket())?, 1025),
            (utf8(&cfg.ssh_socket())?, 1026),
        ],
        plugin: Some(Arc::new(BeforeGuest {
            checked: AtomicBool::new(false),
            sentinel,
            runtime: cfg.runtime.path.clone(),
            state: cfg.state.clone(),
            disks: expected_disks,
        })),
        sandbox: true,
        stop: None,
    };
    match hvi::boot(cfg).map_err(|e| e.to_string())? {
        Stop::SystemOff => Ok(()),
        other => Err(format!("unexpected hvi stop: {other:?}")),
    }
}

fn main() {
    unsafe {
        libc::umask(0o077);
    }
    if let Err(error) = entry() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
