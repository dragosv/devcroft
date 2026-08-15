//! SSH channel handling (ssh spec's "SSH feature coverage for editors",
//! task 6.3), end to end through the *real* `ssh`/`scp`/`sftp`/`rsync` CLI
//! tools — not a russh test client — talking to a real `up`-started keeper
//! via a real `devcroft proxy` subprocess as `ProxyCommand`. This is as
//! close to "an editor connects" as a test gets: exec channel + exit
//! status, pty/shell, the env allowlist, `sftp` (which is what modern
//! OpenSSH `scp` speaks by default too), `-L` direct-tcpip forwarding, and
//! (task 6.5's rsync row in `docs/ssh-validation.md`) `rsync -e ssh`.
//!
//! See `tests/lifecycle_up.rs` for why this needs `CARGO_BIN_EXE_devcroft`
//! and why each such test lives in its own file/process.

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, client_key_paths, down, up};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Sandbox {
    name: String,
    project_root: PathBuf,
    paths: StatePaths,
}

impl Sandbox {
    fn up(tag: &str) -> Option<Self> {
        Self::up_with_manifest_extra(tag, "")
    }

    /// `manifest_extra` is appended verbatim to the `[sandbox]` block —
    /// e.g. a `[network]` section for the direct-tcpip test, which
    /// otherwise hits the manifest's own default of `network.default =
    /// "deny"` (config::Network's `Default` impl): the sandbox's own
    /// policy would correctly reject the forward, same as any other
    /// disallowed target, which is exactly right in general but not what
    /// that test is checking.
    fn up_with_manifest_extra(tag: &str, manifest_extra: &str) -> Option<Self> {
        if Command::new("nono").arg("--version").output().is_err()
            || Command::new("flox").arg("--version").output().is_err()
        {
            eprintln!("skipping: nono and/or flox not on PATH");
            return None;
        }
        unsafe {
            std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
        }

        let project_root = std::env::temp_dir().join(format!(
            "devcroft-ssh-channels-{tag}-e2e-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&project_root);
        std::fs::create_dir_all(&project_root).unwrap();
        let init = Command::new("flox")
            .arg("init")
            .current_dir(&project_root)
            .output()
            .unwrap();
        if !init.status.success() {
            eprintln!(
                "skipping: flox init failed: {}",
                String::from_utf8_lossy(&init.stderr)
            );
            return None;
        }

        let name = format!("e2ec{tag}{}", std::process::id());
        let (manifest, _) =
            parse(&format!("[sandbox]\nname = {name:?}\n{manifest_extra}")).unwrap();
        let paths = StatePaths::new(&name).unwrap();
        let _ = std::fs::remove_dir_all(&paths.root);
        assert_eq!(
            up(&manifest, &project_root, &UpOptions::default()).unwrap(),
            UpOutcome::Started
        );

        Some(Sandbox {
            name,
            project_root,
            paths,
        })
    }

    fn host(&self) -> String {
        format!("{}.devcroft", self.name)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = down(&self.name);
        let _ = std::fs::remove_dir_all(&self.paths.root);
        let _ = std::fs::remove_dir_all(&self.project_root);
    }
}

/// The `-o`/`-i` options every real ssh/scp/sftp invocation below needs to
/// reach a sandbox through `devcroft proxy` instead of a real network
/// connection, using the real generated client identity.
fn ssh_opts(identity: &std::path::Path) -> Vec<String> {
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");
    vec![
        "-o".to_string(),
        "StrictHostKeyChecking=no".to_string(),
        "-o".to_string(),
        "UserKnownHostsFile=/dev/null".to_string(),
        "-o".to_string(),
        "IdentitiesOnly=yes".to_string(),
        "-o".to_string(),
        format!("ProxyCommand={devcroft_bin} proxy --no-up %n"),
        "-i".to_string(),
        identity.to_string_lossy().into_owned(),
    ]
}

fn skip_if_no_real_ssh_tools() -> bool {
    for bin in ["ssh", "scp", "sftp"] {
        if Command::new(bin).arg("-V").output().is_err() {
            eprintln!("skipping: {bin} not on PATH");
            return true;
        }
    }
    false
}

/// Same options as [`ssh_opts`], but as the single shell-quoted string
/// rsync's `-e` wants — rsync never sees a real shell (this passes the
/// value straight through `Command::arg`, not `sh -c`), but its own `-e`
/// parser still splits on whitespace, so the `ProxyCommand` value (which
/// contains spaces) must be quoted to survive as one token.
fn rsync_dash_e(identity: &std::path::Path) -> String {
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");
    format!(
        "ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
         -o IdentitiesOnly=yes -o ProxyCommand='{devcroft_bin} proxy --no-up %n' \
         -i {}",
        identity.to_string_lossy()
    )
}

fn skip_if_no_real_rsync() -> bool {
    if Command::new("rsync").arg("--version").output().is_err() {
        eprintln!("skipping: rsync not on PATH");
        return true;
    }
    false
}

#[test]
fn exec_channel_runs_commands_and_propagates_exit_code() {
    if skip_if_no_real_ssh_tools() {
        return;
    }
    let Some(sandbox) = Sandbox::up("exec") else {
        return;
    };
    let (identity, _) = client_key_paths().unwrap();

    let out = Command::new("ssh")
        .args(ssh_opts(&identity))
        .arg(sandbox.host())
        .arg("echo")
        .arg("exec-channel-marker-hi")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "exec-channel-marker-hi"
    );
    assert_eq!(out.status.code(), Some(0));

    let out = Command::new("ssh")
        .args(ssh_opts(&identity))
        .arg(sandbox.host())
        .arg("exit 42")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(42));
}

#[test]
fn shell_channel_allocates_a_pty_and_runs_commands() {
    if skip_if_no_real_ssh_tools() {
        return;
    }
    let Some(sandbox) = Sandbox::up("shell") else {
        return;
    };
    let (identity, _) = client_key_paths().unwrap();

    let mut child = Command::new("ssh")
        .args(ssh_opts(&identity))
        .arg("-tt")
        .arg(sandbox.host())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"echo shell-channel-marker-hi\nexit\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("shell-channel-marker-hi"),
        "expected pty output to contain the marker, got stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn env_allowlist_passes_term_and_lang_but_not_arbitrary_vars() {
    if skip_if_no_real_ssh_tools() {
        return;
    }
    let Some(sandbox) = Sandbox::up("env") else {
        return;
    };
    let (identity, _) = client_key_paths().unwrap();

    let mut args = ssh_opts(&identity);
    args.push("-o".to_string());
    args.push(
        "SetEnv=TERM=devcroft-test-term LANG=en_US.UTF-8 DEVCROFT_NOT_ALLOWED=leaked".to_string(),
    );

    let out = Command::new("ssh")
        .args(args)
        .arg(sandbox.host())
        .arg("echo TERM=$TERM LANG=$LANG DEVCROFT_NOT_ALLOWED=${DEVCROFT_NOT_ALLOWED:-unset}")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("TERM=devcroft-test-term"),
        "TERM should pass the allowlist, got {stdout:?}"
    );
    assert!(
        stdout.contains("LANG=en_US.UTF-8"),
        "LANG should pass the allowlist, got {stdout:?}"
    );
    assert!(
        stdout.contains("DEVCROFT_NOT_ALLOWED=unset"),
        "an arbitrary var must not pass the allowlist, got {stdout:?}"
    );
}

#[test]
fn sftp_round_trips_a_file_and_lists_a_directory() {
    if skip_if_no_real_ssh_tools() {
        return;
    }
    let Some(sandbox) = Sandbox::up("sftp") else {
        return;
    };
    let (identity, _) = client_key_paths().unwrap();

    let local_src = sandbox.project_root.join("local-src.txt");
    std::fs::write(&local_src, b"devcroft sftp round-trip payload\n").unwrap();
    let remote_path = sandbox.project_root.join("via-sftp.txt");
    let local_dst = sandbox.project_root.join("local-dst.txt");

    // `put`/`get`/`ls` in one batch: open/write/close, open/read/close, and
    // opendir/readdir all in a single real sftp session.
    let batch = sandbox.project_root.join("sftp-batch.txt");
    std::fs::write(
        &batch,
        format!(
            "put {} {}\nget {} {}\nls {}\nbye\n",
            local_src.to_string_lossy(),
            remote_path.to_string_lossy(),
            remote_path.to_string_lossy(),
            local_dst.to_string_lossy(),
            sandbox.project_root.to_string_lossy(),
        ),
    )
    .unwrap();
    let mut sftp_args = ssh_opts(&identity);
    sftp_args.push("-b".to_string());
    sftp_args.push(batch.to_string_lossy().into_owned());
    sftp_args.push(sandbox.host());
    let out = Command::new("sftp").args(&sftp_args).output().unwrap();
    assert!(
        out.status.success(),
        "sftp batch session failed (status {:?}): stdout={} stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read(&remote_path).unwrap(),
        std::fs::read(&local_src).unwrap(),
        "the file written via sftp put must match the source"
    );
    assert_eq!(
        std::fs::read(&local_dst).unwrap(),
        b"devcroft sftp round-trip payload\n",
        "the file fetched back via sftp get must match what was uploaded"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("via-sftp.txt"),
        "sftp ls must list the file written earlier, got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// Modern OpenSSH `scp` (9.0+) speaks SFTP under the hood by default —
/// same subsystem, same `FsHandler` — so what's specific to `scp` is the
/// data it moves *and* the exit code it reports, both asserted here in
/// both directions.
///
/// The exit code is the part worth explaining. `scp` derives it from its
/// internal `ssh -s sftp` child, which needs the channel-level
/// exit-status request to arrive before it stops listening. This test
/// used to skip that assertion, on the reasoning that a real
/// `/usr/lib/openssh/sftp-server` subprocess wins the race by exiting
/// synchronously while `FsHandler` has no subprocess to synchronize
/// against. That framing was wrong: the problem was never timing but
/// **ordering**, and the server controls ordering. `russh_sftp`'s loop
/// ended at EOF and dropped the channel — which sends `close` — before a
/// separately-spawned exit-status send could land, and no client accepts
/// an exit-status after `close`. `ssh::server::NotifyOnEof` now withholds
/// the EOF until the exit-status has been sent, so `close` cannot
/// overtake it. Task 6.5 found this via Zed, which gates its remote
/// server startup on exactly this exit code.
#[test]
fn scp_round_trips_correct_data_and_reports_success() {
    if skip_if_no_real_ssh_tools() {
        return;
    }
    let Some(sandbox) = Sandbox::up("scp") else {
        return;
    };
    let (identity, _) = client_key_paths().unwrap();

    let local_src = sandbox.project_root.join("local-src.txt");
    std::fs::write(&local_src, b"devcroft scp round-trip payload\n").unwrap();
    let remote_path = sandbox.project_root.join("via-scp.txt");

    let mut scp_args = ssh_opts(&identity);
    scp_args.push(local_src.to_string_lossy().into_owned());
    scp_args.push(format!(
        "{}:{}",
        sandbox.host(),
        remote_path.to_string_lossy()
    ));
    let up = Command::new("scp").args(&scp_args).output().unwrap();
    assert!(
        up.status.success(),
        "scp upload must report success, got {:?}: {}",
        up.status,
        String::from_utf8_lossy(&up.stderr)
    );
    assert_eq!(
        std::fs::read(&remote_path).unwrap(),
        std::fs::read(&local_src).unwrap(),
        "the file scp uploaded must match the source"
    );

    let local_dst = sandbox.project_root.join("local-dst.txt");
    let mut scp_back_args = ssh_opts(&identity);
    scp_back_args.push(format!(
        "{}:{}",
        sandbox.host(),
        remote_path.to_string_lossy()
    ));
    scp_back_args.push(local_dst.to_string_lossy().into_owned());
    let down = Command::new("scp").args(&scp_back_args).output().unwrap();
    assert!(
        down.status.success(),
        "scp download must report success, got {:?}: {}",
        down.status,
        String::from_utf8_lossy(&down.stderr)
    );
    assert_eq!(
        std::fs::read(&local_dst).unwrap(),
        b"devcroft scp round-trip payload\n",
        "the file scp downloaded must match what was uploaded"
    );
}

/// Task 6.5's rsync row (docs/ssh-validation.md): `rsync -e ssh` runs
/// `rsync --server ...` on the remote end over a plain SSH **exec
/// channel** — the same path `exec_channel_runs_commands_and_propagates_exit_code`
/// exercises — so this needs no rsync-specific server support, only a
/// real `rsync` binary reachable on both ends. On this host that binary
/// is the system one (`/usr/bin/rsync` on macOS, distro rsync on Linux):
/// reachable inside the sandbox because the flox-activated `PATH` the
/// keeper inherits still contains the canonical system bin dirs
/// (`provider::flox`'s `CANONICAL_PATH`), not because any sample project
/// installs rsync itself.
#[test]
fn rsync_transfers_a_file_through_devcroft_proxy_over_a_plain_exec_channel() {
    if skip_if_no_real_ssh_tools() || skip_if_no_real_rsync() {
        return;
    }
    let Some(sandbox) = Sandbox::up("rsync") else {
        return;
    };
    let (identity, _) = client_key_paths().unwrap();
    let rsh = rsync_dash_e(&identity);

    let local_src = sandbox.project_root.join("local-src.txt");
    std::fs::write(&local_src, b"devcroft rsync round-trip payload\n").unwrap();
    let remote_path = sandbox.project_root.join("via-rsync.txt");

    let upload = Command::new("rsync")
        .arg("-e")
        .arg(&rsh)
        .arg(&local_src)
        .arg(format!(
            "{}:{}",
            sandbox.host(),
            remote_path.to_string_lossy()
        ))
        .output()
        .unwrap();
    assert!(
        upload.status.success(),
        "rsync upload failed (status {:?}): stdout={} stderr={}",
        upload.status.code(),
        String::from_utf8_lossy(&upload.stdout),
        String::from_utf8_lossy(&upload.stderr)
    );
    assert_eq!(
        std::fs::read(&remote_path).unwrap(),
        std::fs::read(&local_src).unwrap(),
        "the file rsync uploaded must match the source"
    );

    let local_dst = sandbox.project_root.join("local-dst.txt");
    let download = Command::new("rsync")
        .arg("-e")
        .arg(&rsh)
        .arg(format!(
            "{}:{}",
            sandbox.host(),
            remote_path.to_string_lossy()
        ))
        .arg(&local_dst)
        .output()
        .unwrap();
    assert!(
        download.status.success(),
        "rsync download failed (status {:?}): stdout={} stderr={}",
        download.status.code(),
        String::from_utf8_lossy(&download.stdout),
        String::from_utf8_lossy(&download.stderr)
    );
    assert_eq!(
        std::fs::read(&local_dst).unwrap(),
        b"devcroft rsync round-trip payload\n",
        "the file rsync downloaded must match what was uploaded"
    );
}

#[test]
fn direct_tcpip_forwarding_relays_a_real_connection() {
    if skip_if_no_real_ssh_tools() {
        return;
    }
    let Some(sandbox) = Sandbox::up_with_manifest_extra("fwd", "[network]\ndefault = \"allow\"\n")
    else {
        return;
    };
    let (identity, _) = client_key_paths().unwrap();

    // A plain echo server standing in for "something reachable from
    // inside the sandbox" — MVP has no network-namespace separation
    // (design.md decision 5), so the keeper reaches the same host network
    // this test process does.
    let echo_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let echo_port = echo_listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in echo_listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut buf = [0u8; 1024];
                loop {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if stream.write_all(&buf[..n]).is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }
    });

    let local_port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };

    let mut args = ssh_opts(&identity);
    args.push("-N".to_string());
    args.push("-o".to_string());
    args.push("ExitOnForwardFailure=yes".to_string());
    args.push("-L".to_string());
    args.push(format!("{local_port}:127.0.0.1:{echo_port}"));
    args.push(sandbox.host());
    let mut tunnel = Command::new("ssh")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut connected = None;
    while Instant::now() < deadline {
        if let Ok(stream) = std::net::TcpStream::connect(("127.0.0.1", local_port)) {
            connected = Some(stream);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut stream = connected.expect("local end of the -L tunnel never became connectable");

    stream.write_all(b"through-the-tunnel").unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    let mut received = Vec::new();
    stream.read_to_end(&mut received).unwrap();
    assert_eq!(received, b"through-the-tunnel");

    let _ = tunnel.kill();
    let _ = tunnel.wait();
}
