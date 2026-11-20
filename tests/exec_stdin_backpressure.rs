//! `devcroft exec` with stdin the session never reads, end to end: the real
//! binary, a real keeper, `sleep` as a child that leaves its stdin alone.
//!
//! Two failures this guards against. The client used to queue stdin without
//! bound, so `yes | devcroft exec sleep 100` grew the client as fast as `yes`
//! could write; and the keeper wrote stdin to the child inline, so once the
//! pipe was full it stopped reading the connection at all — Ctrl-C sat behind
//! the unread stdin, and a client that gave up left the child running.

mod common;

use common::for_each_row;
use devcroft::keeper::protocol::{self, Frame};
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[test]
fn unread_stdin_is_bounded_and_ctrl_c_still_ends_the_session() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    for_each_row("exec-stdin", |fx| {
        if !fx.capabilities().external_utils {
            eprintln!(
                "skipping exec-stdin on row {}: no external utilities in this row's environment",
                fx.name()
            );
            return;
        }
        let sandbox_name = fx.sandbox_name().to_string();
        let project_root = fx.project_root().to_path_buf();
        let paths = StatePaths::new(&sandbox_name).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);
        assert_eq!(
            fx.bring_up(&UpOptions::default()).unwrap(),
            UpOutcome::Started
        );

        let mut child = Command::new(devcroft_bin)
            .arg("exec")
            .arg(&sandbox_name)
            .arg("--")
            .arg("sleep")
            .arg("100")
            .current_dir(&project_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let accepted = Arc::new(AtomicUsize::new(0));
        let mut stdin = child.stdin.take().unwrap();
        let feeder = {
            let accepted = Arc::clone(&accepted);
            std::thread::spawn(move || {
                let chunk = vec![0u8; 64 * 1024];
                while stdin.write_all(&chunk).is_ok() {
                    accepted.fetch_add(chunk.len(), Ordering::SeqCst);
                }
            })
        };

        std::thread::sleep(Duration::from_secs(2));
        let after_two_seconds = accepted.load(Ordering::SeqCst);
        // Every queue on the way (client, socket, keeper, pipe) is bounded
        // well below this; an unbounded one takes hundreds of MiB in 2 s.
        assert!(
            after_two_seconds < 32 * 1024 * 1024,
            "the client took {after_two_seconds} bytes of stdin nothing reads"
        );

        // The keeper's own session registry, asked the way `status` asks it.
        let socket = paths.socket.clone();
        let listing = move |_: &str| {
            let mut conn = UnixStream::connect(&socket).unwrap();
            protocol::write_frame(&mut conn, &Frame::Query).unwrap();
            match protocol::read_frame(&mut conn).unwrap() {
                Frame::QueryResult(result) => result
                    .sessions
                    .iter()
                    .map(|s| s.command.clone())
                    .collect::<Vec<_>>()
                    .join("\n"),
                other => panic!("unexpected reply {other:?}"),
            }
        };
        // Positive control for the check below: `ps` shows the live session.
        let live = listing(&sandbox_name);
        assert!(
            live.contains("sleep 100"),
            "ps did not show the session:\n{live}"
        );

        let start = Instant::now();
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGINT);
        }
        let status = child.wait().unwrap();
        assert_eq!(status.code(), Some(130));
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "Ctrl-C took {:?} to end the client",
            start.elapsed()
        );
        feeder.join().unwrap();

        // The keeper notices the client left and ends the session.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let now = listing(&sandbox_name);
            if !now.contains("sleep 100") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the session outlived its client:\n{now}"
            );
            std::thread::sleep(Duration::from_millis(200));
        }

        down(&sandbox_name).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);
        let _ = std::fs::remove_dir_all(&project_root);
    });
}
