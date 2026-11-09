//! An in-process `up` must not keep the keeper's listening sockets open in
//! its caller.
//!
//! Written after a test run hung for ten minutes. `up` cleared
//! `FD_CLOEXEC` on the control and SSH listeners in the calling process and
//! then `mem::forget`-ed them. For a one-shot `devcroft up` that changed
//! nothing, since the process exits. For a caller that stays alive and runs
//! several `up`s at once — the test suite's parallel threads — every
//! sandbox's listeners stayed open in the caller *and* were inherited by
//! every subprocess it spawned. A removed sandbox's control socket then went
//! on accepting connections nobody would ever answer, and `devcroft exec`,
//! listing known sandboxes through `ps`, blocked forever on one.
//!
//! The fix clears the flag in the forked child only (`pass_fds`) and closes
//! the caller's copies once the keeper has its own. This asserts the second
//! half directly. The first is what makes the second safe: without it,
//! closing the copies would not undo inheritance by a child spawned in the
//! meantime.
//!
//! **Neutral surface** (`test-runtime-fixture`): runs on whichever row is
//! selected. Linux only: it maps this process's fds to socket paths through
//! `/proc/net/unix`.

mod common;

use common::for_each_row;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down};

/// Socket inodes bound at `path`, from this network namespace's table.
/// `up` binds in the caller, before any namespace, so the caller's table
/// has them.
#[cfg(target_os = "linux")]
fn inodes_bound_at(path: &std::path::Path) -> Vec<String> {
    let table = std::fs::read_to_string("/proc/net/unix").unwrap();
    let path = path.to_string_lossy();
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            (cols.len() >= 8 && cols[7] == path).then(|| cols[6].to_string())
        })
        .collect()
}

/// This process's fds that refer to one of `inodes`.
#[cfg(target_os = "linux")]
fn own_fds_on(inodes: &[String]) -> Vec<String> {
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .flatten()
        .filter_map(|e| {
            let target = std::fs::read_link(e.path()).ok()?;
            let target = target.to_string_lossy();
            let inode = target.strip_prefix("socket:[")?.strip_suffix(']')?;
            inodes
                .iter()
                .any(|i| i == inode)
                .then(|| format!("fd {} -> {target}", e.file_name().to_string_lossy()))
        })
        .collect()
}

#[cfg(target_os = "linux")]
#[test]
fn an_in_process_up_leaves_no_listener_open_in_its_caller() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    for_each_row("listeners", |fx| {
        let paths = StatePaths::new(fx.sandbox_name()).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);
        let _ = std::fs::remove_file(&paths.lifecycle_lock);
        assert_eq!(
            fx.bring_up(&UpOptions::default())
                .unwrap_or_else(|e| panic!("row {}: up failed: {e}", fx.name())),
            UpOutcome::Started
        );

        for socket in [&paths.socket, &paths.ssh_socket] {
            let inodes = inodes_bound_at(socket);
            // The control: the keeper is healthy, so the socket is bound
            // and listed. Without this an empty table would pass the
            // assertion below having checked nothing.
            assert!(
                !inodes.is_empty(),
                "row {}: {} is not in /proc/net/unix, so this test cannot see it",
                fx.name(),
                socket.display()
            );
            let held = own_fds_on(&inodes);
            assert!(
                held.is_empty(),
                "row {}: `up` returned but its caller still holds the keeper's listener \
                 {}: {held:?}. Every subprocess this caller spawns inherits it, and the \
                 socket keeps accepting after the keeper is gone.",
                fx.name(),
                socket.display()
            );
        }

        down(fx.sandbox_name()).unwrap();
    });
}
