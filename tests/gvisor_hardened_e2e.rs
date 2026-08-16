//! `isolation = "hardened"` (add-gvisor-backend task 4.3), end to end
//! against a real `runsc`: a real `up` that synthesizes and starts an
//! OCI bundle, a real `exec` session dispatched via `runsc exec` instead
//! of local fork/exec, and a real SSH handshake through `devcroft proxy`
//! against the host-side control server this tier starts instead of an
//! in-sandbox keeper. See `tests/lifecycle_up.rs` for why this needs
//! `CARGO_BIN_EXE_devcroft` and why each such test lives in its own
//! file/process; see `tests/proxy_up.rs` for the `ChildPipe`/handshake
//! pattern reused below.
//!
//! Availability is checked the same way `doctor_gvisor_backend` does
//! (`src/bin/devcroft.rs`): `runsc` on `PATH` is not enough by itself —
//! `runsc --rootless --platform <platform> do true` must actually
//! succeed. On this repo's own devcontainer (task group 8), it does not:
//! `runsc`'s re-exec into a fresh user namespace fails with the same
//! `EPERM` `unshare --user` reports here, so this test self-skips in
//! exactly the environment `src/gvisor/runner.rs`'s module doc already
//! describes as the platform boundary, not a bug — task 10.3's "honest,
//! not claimed" status extends to this test's own skip message.

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

/// Mirrors `doctor_gvisor_backend`'s probe exactly (binary presence is
/// not enough — the platform must actually run something), so this test
/// skips under precisely the same condition `devcroft doctor` reports
/// `[FAIL] gvisor-backend`, never a looser or stricter one.
fn gvisor_available() -> bool {
    let Some(runsc) = devcroft::gvisor::runsc_command::resolve() else {
        return false;
    };
    if devcroft::gvisor::runsc_command::probe_version(&runsc).is_none() {
        return false;
    }
    let platform = devcroft::gvisor::select_platform();
    Command::new(&runsc)
        .arg("--rootless")
        .arg("--platform")
        .arg(platform.runsc_flag())
        .arg("do")
        .arg("true")
        .output()
        .is_ok_and(|out| out.status.success())
}

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
/// — see `tests/proxy_up.rs`'s copy of this struct for the full rationale.
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
async fn hardened_tier_exec_and_ssh_round_trip_against_a_live_gvisor_sandbox() {
    if !gvisor_available() {
        eprintln!(
            "skipping: runsc not usable on this host (missing, or the selected platform \
             does not actually run — see `devcroft doctor`'s gvisor-backend line)"
        );
        return;
    }

    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    let devcroft_bin = env!("CARGO_BIN_EXE_devcroft");

    // No flox/nono needed at this tier: `up_hardened` synthesizes the OCI
    // bundle straight from `CompiledPolicy` and the project root itself
    // (no provider resolution required for a manifest with no `[env]`
    // section), unlike the process-tier tests this repo's other `*_up.rs`
    // files exercise.
    let project_root = std::env::temp_dir().join(format!(
        "devcroft-gvisor-hardened-e2e-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();

    let sandbox_name = format!("e2egvisor{}", std::process::id());
    let (manifest, _) = parse(&format!(
        "[sandbox]\nname = {sandbox_name:?}\nisolation = \"hardened\"\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);

    assert_eq!(
        up(&manifest, &project_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );

    // Exec round trip (add-hardened-tier's `SessionBackend` seam,
    // dispatched via `runsc exec` instead of local fork/exec) — same
    // exit-code-propagation assertion `tests/exec_up.rs` makes for the
    // process tier, proving the two tiers behave identically from the
    // client's point of view.
    let out = Command::new(devcroft_bin)
        .arg("exec")
        .arg(&sandbox_name)
        .arg("--")
        .arg("sh")
        .arg("-c")
        .arg("echo hi; exit 42")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "hi\n",
        "stderr was: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(42));

    // SSH round trip through `devcroft proxy` against the host-side
    // control server this tier starts (no keeper runs inside the
    // sandbox) — proves the "client cannot tell the difference" claim
    // add-hardened-tier's `ssh` delta spec makes, the same handshake
    // `tests/proxy_up.rs` proves for the process tier.
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
        .expect("ssh handshake must complete through the proxy bridge at the hardened tier");
    let result = handle
        .authenticate_publickey(
            "devcroft",
            PrivateKeyWithHashAlg::new(Arc::new(client_key), None),
        )
        .await
        .unwrap();
    assert!(
        result.success(),
        "the real client key must authenticate through the proxy bridge at the hardened tier"
    );

    drop(handle);
    let _ = child.start_kill();
    let _ = child.wait().await;

    down(&sandbox_name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_dir_all(&project_root);
}
