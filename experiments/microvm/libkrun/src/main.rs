mod config;
#[cfg(target_os = "linux")]
mod linux;
mod sandbox;

use config::{CONTROL_PORT, KEEPER_PORT, Launch, REVISION, SSH_PORT};
use krun::api::{
    BalloonDevice, BlockDevice, ConsoleDevice, DiskFormat, FreePageReportingMode, KernelFormat,
    MmioDeviceManager, Payload, RngDevice, SyncMode, TsiFlags, VmmBuilder, VsockDevice,
};
use std::path::Path;

struct GuestLog;
impl log::Log for GuestLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }
    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("{}", record.args());
        }
    }
    fn flush(&self) {}
}
static GUEST_LOG: GuestLog = GuestLog;

fn main() {
    log::set_logger(&GUEST_LOG).expect("single logger initialization");
    log::set_max_level(log::LevelFilter::Warn);
    // The controller owns stdout/stderr. No inherited terminal or ambient file
    // descriptors are guest devices. umask also covers libkrun's Unix socket.
    unsafe {
        libc::umask(0o077);
    }
    if let Err(error) = entry() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn entry() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().collect();
    #[cfg(target_os = "macos")]
    if args.len() == 5 && args[1] == "controller-test" {
        let cfg = Launch::read(Path::new(&args[2]))?;
        sandbox::close_inherited_fds()?;
        return sandbox::controller_test(&cfg, Path::new(&args[3]), Path::new(&args[4]));
    }
    if args.len() == 2 && args[1] == "version" {
        println!("libkrun 2.0.0-dev {REVISION}");
        return Ok(());
    }
    if args.len() == 2 && args[1] == "provenance" {
        let provenance = serde_json::json!({"revision":REVISION,
            "patch_sha256":env!("DEVCROFT_KRUN_PATCH_SHA256")});
        #[cfg(target_os = "linux")]
        let provenance = {
            let mut p = provenance;
            p["linux_seccomp_sha256"] = env!("DEVCROFT_KRUN_LINUX_FILTER_SHA256").into();
            p["linux_host_boundary"] =
                "Landlock ABI >=6 + inherited kill-process seccomp before VMM construction".into();
            p
        };
        println!("{provenance}");
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if args.len() == 4 && args[1] == "syscall-probe" {
        let cfg = Launch::read(Path::new(&args[2]))?;
        sandbox::close_inherited_fds()?;
        return linux::probe(&cfg, args[3].to_str().ok_or("non-UTF-8 probe")?);
    }
    let selftest = args.len() == 4 && args[1] == "sandbox-test";
    if !selftest && (args.len() != 3 || (args[1] != "run" && args[1] != "check")) {
        return Err(
            "usage: devcroft-krun-spike version | check LAUNCH.json | run LAUNCH.json | sandbox-test LAUNCH.json OUTSIDE_SENTINEL".into(),
        );
    }
    let cfg = Launch::read(Path::new(&args[2]))?;
    if args[1] == "check" {
        println!(
            "{}",
            serde_json::json!({"schema":1,"revision":REVISION,
            "control_socket":cfg.control_socket(), "vcpus":cfg.vcpus,
            "memory_mib":cfg.memory_mib, "nic":false,"tsi":false,
            "status":"inputs-validated","qualification":"not-run"})
        );
        return Ok(());
    }

    // No VMM/device constructor, worker thread or payload loading before this
    // restriction. At the pin build() starts vCPUs on both Linux and macOS.
    sandbox::close_inherited_fds()?;
    if selftest {
        return sandbox::selftest(&cfg, Path::new(&args[3]));
    }
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    let mut disk_locks = Vec::new();
    for path in std::iter::once(&cfg.state).chain(cfg.data.iter()) {
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(|e| format!("backend: lock state: {e}"))?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("backend: private disk is already in use".into());
        }
        disk_locks.push(lock);
    }
    sandbox::apply(&cfg)?;
    eprintln!(
        "{}",
        serde_json::json!({"event":"confinement-applied",
        "before":"payload-and-vmm-build","revision":REVISION})
    );

    // Hash again under restriction immediately before handing paths to libkrun.
    // These are experiment-owned immutable inputs, not attacker-writable paths.
    for artifact in cfg.artifacts() {
        config::verify(artifact)?;
    }
    boot(&cfg)
}

fn text_path(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| "config: libkrun requires UTF-8 paths".into())
}

fn boot(cfg: &Launch) -> Result<(), String> {
    let map = |e| format!("backend: libkrun: {e}");
    let format = if cfg!(target_arch = "aarch64") {
        KernelFormat::Raw
    } else {
        KernelFormat::Elf
    };
    let payload = Payload::load_external(
        text_path(&cfg.kernel.path)?,
        format,
        Some(text_path(&cfg.initramfs.path)?),
        if cfg.workload.is_some() {
            "console=hvc0 rdinit=/init panic=-1 reboot=k"
        } else {
            "console=hvc0 rdinit=/init panic=-1 reboot=k quiet"
        },
    )
    .map_err(map)?;
    let mut devices = MmioDeviceManager::new();
    let mut console = ConsoleDevice::builder();
    // A plain in/out port has no terminal properties and does not create hvc0.
    // The default console supplies a fixed-size terminal and sends guest logs
    // through the logger to the already-owned, harness-bounded stderr pipe.
    console.add_default_console(None, None, None).map_err(map)?;
    devices.add(console.build().map_err(map)?);
    let mut runtime =
        BlockDevice::new("runtime", text_path(&cfg.runtime.path)?, DiskFormat::Raw).map_err(map)?;
    runtime.set_read_only(true);
    runtime.set_sync_mode(SyncMode::Full);
    devices.add(runtime);
    let mut state =
        BlockDevice::new("state", text_path(&cfg.state)?, DiskFormat::Raw).map_err(map)?;
    state.set_sync_mode(SyncMode::Full);
    devices.add(state);
    if let (Some(workload), Some(data)) = (&cfg.workload, &cfg.data) {
        let mut tools = BlockDevice::new("toolchain", text_path(&workload.path)?, DiskFormat::Raw)
            .map_err(map)?;
        tools.set_read_only(true);
        tools.set_sync_mode(SyncMode::Full);
        devices.add(tools);
        let mut private =
            BlockDevice::new("workspace", text_path(data)?, DiskFormat::Raw).map_err(map)?;
        private.set_sync_mode(SyncMode::Full);
        devices.add(private);
    }
    let mut vsock = VsockDevice::new(3, TsiFlags::empty()).map_err(map)?;
    vsock.add_unix_port(CONTROL_PORT, text_path(&cfg.control_socket())?, true);
    vsock.add_unix_port(KEEPER_PORT, text_path(&cfg.keeper_socket())?, true);
    vsock.add_unix_port(SSH_PORT, text_path(&cfg.ssh_socket())?, true);
    devices.add(vsock);
    devices.add(RngDevice::new().map_err(map)?);
    let mut balloon = BalloonDevice::new().map_err(map)?;
    if cfg!(target_os = "macos") {
        balloon.set_free_page_reporting_mode(FreePageReportingMode::Replace);
    }
    devices.add(balloon);
    eprintln!(
        "{}",
        serde_json::json!({"event":"vmm-build-enter",
        "nic":false,"tsi":false,"state_sync":"full"})
    );
    let vm = VmmBuilder::new()
        .vcpus(cfg.vcpus)
        .map_err(map)?
        .ram_mib(cfg.memory_mib)
        .map_err(map)?
        .payload(payload)
        .set_kernel_console("hvc0")
        .devices(devices)
        .build()
        .map_err(map)?;
    eprintln!("{}", serde_json::json!({"event":"vmm-build-returned"}));
    vm.run();
    // Event-loop failure must never become a successful smoke result.
    Err("backend: VMM event loop returned without guest shutdown".into())
}
