//! `devcroft proxy` (ssh spec's "ProxyCommand bridging" requirement, task
//! 6.2), end to end: a real `up`, a real `devcroft proxy <name>.devcroft`
//! subprocess, and a real SSH client handshake carried entirely over that
//! subprocess's piped stdio — proving the byte bridge actually works in
//! both directions, not just that the process starts. See
//! `tests/lifecycle_up.rs` for why this needs `CARGO_BIN_EXE_devcroft`
//! and why each such test lives in its own file/process.

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, client_key_paths, down, up};
use russh::client;
use russh::keys::PrivateKey;
use russh::keys::key::PrivateKeyWithHashAlg;
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

struct AcceptAnyServerKey;

impl client::Handler for AcceptAnyServerKey {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// Glues a child process's stdin+stdout into one `AsyncRead + AsyncWrite`
/// — what `russh::client::connect_stream` needs as its transport, and
/// exactly the role a real ssh client's own process plumbing plays around
/// `ProxyCommand` in practice.
struct ChildPipe {
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
}

impl AsyncRead for ChildPipe {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stdout).poll_read(cx, buf)
    }
}

impl AsyncWrite for ChildPipe {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().stdin).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stdin).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stdin).poll_shutdown(cx)
    }
}

#[tokio::test]
async fn proxy_bridges_a_real_ssh_handshake_through_its_stdio() {
    if Command::new("nono").arg("--version").output().is_err()
        || Command::new("flox").arg("--version").output().is_err()
    {
        eprintln!("skipping: nono and/or flox not on PATH");
        return;
    }

    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    let project_root =
        std::env::temp_dir().join(format!("devcroft-proxy-up-e2e-{}", std::process::id()));
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
        return;
    }

    let sandbox_name = format!("e2eproxy{}", std::process::id());
    let (manifest, _) = parse(&format!("[sandbox]\nname = {sandbox_name:?}\n")).unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    let mut child = tokio::process::Command::new(devcroft_bin)
        .arg("proxy")
        .arg("--no-up")
        .arg(format!("{sandbox_name}.devcroft"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let pipe = ChildPipe {
        stdin: child.stdin.take().unwrap(),
        stdout: child.stdout.take().unwrap(),
    };

    let (client_private_path, _) = client_key_paths().unwrap();
    let client_key = PrivateKey::read_openssh_file(&client_private_path).unwrap();

    let config = Arc::new(client::Config::default());
    let mut handle = client::connect_stream(config, pipe, AcceptAnyServerKey)
        .await
        .expect("ssh handshake must complete through the proxy bridge");
    let result = handle
        .authenticate_publickey(
            "devcroft",
            PrivateKeyWithHashAlg::new(Arc::new(client_key), None),
        )
        .await
        .unwrap();
    assert!(
        result.success(),
        "the real client key must authenticate through the proxy bridge"
    );

    // Dropping `handle` alone doesn't close the transport — the
    // connection's background task retains ownership of the `ChildPipe`
    // regardless, so the child would never see EOF on its stdin and
    // `child.wait()` would hang forever. This test is only about proving
    // the bridge relays a real handshake correctly, not graceful
    // teardown, so just kill it outright.
    drop(handle);
    let _ = child.start_kill();
    let _ = child.wait().await;

    down(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}

#[tokio::test]
async fn proxy_reports_a_clear_error_for_an_unknown_sandbox() {
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");
    let out = tokio::process::Command::new(devcroft_bin)
        .arg("proxy")
        .arg("--no-up")
        .arg("no-such-sandbox-devcroft-proxy-test.devcroft")
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not up"),
        "expected a clear 'not up' error, got {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test]
async fn proxy_rejects_a_host_without_the_devcroft_suffix() {
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");
    let out = tokio::process::Command::new(devcroft_bin)
        .arg("proxy")
        .arg("not-a-devcroft-host")
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

/// Task 5.3's auto-up, exercised through `proxy` specifically (exec spec's
/// "unless auto-up applies", ssh spec's "ProxyCommand bridging" — the two
/// requirements meet here). `cli_proxy` reuses the exact same
/// `maybe_auto_up` helper `exec`/`shell` already prove out in
/// `tests/exec_auto_up.rs`; this only needs to confirm the wiring, not
/// re-prove the mechanism, so it stops as soon as the sandbox is
/// confirmed healthy rather than driving a full handshake.
#[tokio::test]
async fn proxy_brings_up_a_cold_sandbox_automatically() {
    if Command::new("nono").arg("--version").output().is_err()
        || Command::new("flox").arg("--version").output().is_err()
    {
        eprintln!("skipping: nono and/or flox not on PATH");
        return;
    }

    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    let project_root =
        std::env::temp_dir().join(format!("devcroft-proxy-auto-up-e2e-{}", std::process::id()));
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
        return;
    }

    let sandbox_name = format!("e2eproxyauto{}", std::process::id());
    std::fs::write(
        project_root.join("devcroft.toml"),
        format!("[sandbox]\nname = {sandbox_name:?}\n"),
    )
    .unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    // No `up` anywhere above: a cold sandbox, `proxy` targets it without
    // `--no-up`.
    let mut child = tokio::process::Command::new(devcroft_bin)
        .arg("proxy")
        .arg(format!("{sandbox_name}.devcroft"))
        .current_dir(&project_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdin.take()); // immediate EOF; nothing else this test needs to send

    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(10);
    loop {
        if let devcroft::lifecycle::Health::Healthy(_) =
            devcroft::lifecycle::health(&paths).unwrap()
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "sandbox never became healthy via proxy's auto-up"
        );
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // The bridge itself (real handshake through it) is already covered by
    // `proxy_bridges_a_real_ssh_handshake_through_its_stdio` — this test
    // is only about auto-up firing, so tear the child down directly
    // rather than depending on the connection unwinding cleanly on its
    // own.
    let _ = child.start_kill();
    let _ = child.wait().await;

    down(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}
