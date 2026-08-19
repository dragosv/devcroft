//! add-hardened-tier task 5.2: the `ssh` delta spec's "Client cannot
//! tell the difference" scenario — the same client workflow (an `exec`
//! round trip, then an SSH handshake through `devcroft proxy`) driven
//! against a `process`-tier sandbox and a `hardened`-tier sandbox,
//! asserted identical. Requires both tiers' real tooling (`nono`+`flox`
//! for `process`, a functionally usable `runsc` for `hardened` — see
//! `tests/gvisor_hardened_e2e.rs`'s `gvisor_available` for why binary
//! presence alone isn't enough) and self-skips otherwise, same as every
//! other real-tooling test in this suite. See `tests/lifecycle_up.rs`
//! for why this needs `CARGO_BIN_EXE_devcroft` and why each such test
//! lives in its own file/process.

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

fn process_tier_available() -> bool {
    Command::new("nono").arg("--version").output().is_ok()
        && Command::new("flox").arg("--version").output().is_ok()
}

/// Mirrors `tests/gvisor_hardened_e2e.rs`'s `gvisor_available` exactly —
/// kept as its own copy rather than a shared module since this crate's
/// integration tests have no shared-code convention (every `*_up.rs`/
/// `*_e2e.rs` file duplicates its own small helpers instead).
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

/// What the spec's scenario says must be identical across tiers:
/// "authentication, session behavior, and channel features" — sampled
/// here as an exec round trip's stdout/exit code, plus whether an SSH
/// client key authenticates through `devcroft proxy`.
#[derive(Debug, PartialEq)]
struct ClientWorkflowResult {
    exec_stdout: String,
    exec_exit_code: Option<i32>,
    ssh_auth_succeeded: bool,
}

async fn run_client_workflow(
    sandbox_name: &str,
    devcroft_bin: &str,
    project_root: &std::path::Path,
) -> ClientWorkflowResult {
    let out = Command::new(devcroft_bin)
        .arg("exec")
        .arg(sandbox_name)
        .arg("--")
        .arg("sh")
        .arg("-c")
        .arg("echo hi; exit 7")
        // The hardened tier only mounts the project root (plus
        // `/nix/store`) inside the sandbox — unlike the process tier,
        // where the exec'd command just inherits whatever directory
        // happens to exist on the shared host filesystem, an unrelated
        // cwd here fails outright ("no such file or directory") rather
        // than silently running in the wrong place. See
        // `tests/gvisor_hardened_e2e.rs`'s matching fix.
        .current_dir(project_root)
        .output()
        .unwrap();
    let exec_stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let exec_exit_code = out.status.code();

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
    let ssh_auth_succeeded = match client::connect_stream(config, pipe, AcceptAnyServerKey).await {
        Ok(mut handle) => handle
            .authenticate_publickey(
                "devcroft",
                PrivateKeyWithHashAlg::new(Arc::new(client_key), None),
            )
            .await
            .map(|r| r.success())
            .unwrap_or(false),
        Err(_) => false,
    };

    let _ = child.start_kill();
    let _ = child.wait().await;

    ClientWorkflowResult {
        exec_stdout,
        exec_exit_code,
        ssh_auth_succeeded,
    }
}

#[tokio::test]
async fn client_workflow_is_identical_across_process_and_hardened_tiers() {
    if !process_tier_available() {
        eprintln!("skipping: nono and/or flox not on PATH");
        return;
    }
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

    // process tier: needs a real `flox init`, same as every other
    // process-tier e2e test in this suite.
    let process_root = std::env::temp_dir().join(format!(
        "devcroft-tier-parity-process-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&process_root);
    std::fs::create_dir_all(&process_root).unwrap();
    let init = Command::new("flox")
        .arg("init")
        .current_dir(&process_root)
        .output()
        .unwrap();
    if !init.status.success() {
        eprintln!(
            "skipping: flox init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        return;
    }
    let process_name = format!("tierparityproc{}", std::process::id());
    let (process_manifest, _) = parse(&format!("[sandbox]\nname = {process_name:?}\n")).unwrap();
    let process_paths = StatePaths::new(&process_name).unwrap();
    let _ = std::fs::remove_dir_all(&process_paths.root);
    assert_eq!(
        up(&process_manifest, &process_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );
    let process_result = run_client_workflow(&process_name, devcroft_bin, &process_root).await;
    down(&process_name).unwrap();
    let _ = std::fs::remove_dir_all(&process_paths.root);
    let _ = std::fs::remove_dir_all(&process_root);

    // hardened tier: no `nono` involved, but `up`'s shared prefix
    // resolves the environment provider for every isolation tier
    // unconditionally, so `flox init` is still required here — see
    // `tests/gvisor_hardened_e2e.rs`'s module doc for why an earlier
    // version of this comment's "no flox involved at all" claim was
    // wrong.
    let hardened_root = std::env::temp_dir().join(format!(
        "devcroft-tier-parity-hardened-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&hardened_root);
    std::fs::create_dir_all(&hardened_root).unwrap();
    let hardened_init = Command::new("flox")
        .arg("init")
        .current_dir(&hardened_root)
        .output()
        .unwrap();
    if !hardened_init.status.success() {
        eprintln!(
            "skipping: flox init failed: {}",
            String::from_utf8_lossy(&hardened_init.stderr)
        );
        return;
    }
    // See `tests/gvisor_hardened_e2e.rs`'s matching comment: the
    // sandbox's PID 1 (`oci_spec::INIT_COMMAND`) needs `sh`/`sleep` on
    // the activated environment's own `PATH`, which a bare `flox init`
    // does not provide.
    let hardened_pkgs = Command::new("flox")
        .args(["install", "bash", "coreutils"])
        .current_dir(&hardened_root)
        .output()
        .unwrap();
    if !hardened_pkgs.status.success() {
        eprintln!(
            "skipping: flox install bash coreutils failed: {}",
            String::from_utf8_lossy(&hardened_pkgs.stderr)
        );
        return;
    }
    let hardened_name = format!("tierparityhard{}", std::process::id());
    let (hardened_manifest, _) = parse(&format!(
        "[sandbox]\nname = {hardened_name:?}\nisolation = \"hardened\"\n"
    ))
    .unwrap();
    let hardened_paths = StatePaths::new(&hardened_name).unwrap();
    let _ = std::fs::remove_dir_all(&hardened_paths.root);
    assert_eq!(
        up(&hardened_manifest, &hardened_root, &UpOptions::default()).unwrap(),
        UpOutcome::Started
    );
    let hardened_result = run_client_workflow(&hardened_name, devcroft_bin, &hardened_root).await;
    down(&hardened_name).unwrap();
    let _ = std::fs::remove_dir_all(&hardened_paths.root);
    let _ = std::fs::remove_dir_all(&hardened_root);

    assert_eq!(
        process_result, hardened_result,
        "the same client workflow must behave identically regardless of isolation tier"
    );
    assert!(
        process_result.ssh_auth_succeeded,
        "sanity check: the workflow must have actually authenticated, not just agreed by both failing"
    );
}
