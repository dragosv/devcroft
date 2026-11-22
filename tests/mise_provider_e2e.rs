//! The mise provider, end to end (`add-mise-provider` tasks 6.2–6.4), driven
//! through the real binary against `samples/mise-sample`.
//!
//! Skips unless the host is a qualified runtime target (native ARM64, glibc
//! on Linux), the process tier is enforceable, and the `mise` on `PATH` is at
//! least the qualified release. The guard is on that capability, not on the
//! binary: an older mise is refused by the provider, so a test that ran
//! against it would measure the refusal rather than the provider.
//!
//! One test drives the whole sequence, because every step shares the real
//! artifact store under `~/.local/share/devcroft-artifacts/mise`, and the
//! steps that remove objects must not interleave with the steps that read
//! them. The first `up` downloads the locked Go archive from dl.google.com.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const QUALIFIED: (u32, u32, u32) = (2026, 11, 9);

fn devcroft(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .args(args)
        .current_dir(dir)
        .env("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"))
        .output()
        .expect("running devcroft")
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn qualified_host() -> Option<&'static str> {
    let variant = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macos-arm64",
        ("linux", "aarch64") if cfg!(target_env = "gnu") => "",
        _ => {
            eprintln!("skipping: mise runtime is qualified only on native ARM64");
            return None;
        }
    };
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: the process-tier backend is not enforceable on this host");
        return None;
    }
    let version = Command::new("mise")
        .arg("--version")
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", std::env::temp_dir())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let parsed = version.as_deref().and_then(|v| {
        let mut parts = v.split_whitespace().next()?.split('.');
        Some((
            parts.next()?.parse::<u32>().ok()?,
            parts.next()?.parse::<u32>().ok()?,
            parts.next()?.parse::<u32>().ok()?,
        ))
    });
    match parsed {
        Some(v) if v >= QUALIFIED => Some(variant),
        _ => {
            eprintln!("skipping: mise >= 2026.11.9 is not on PATH (found {version:?})");
            None
        }
    }
}

struct Project(PathBuf);
impl Drop for Project {
    fn drop(&mut self) {
        let _ = devcroft(&self.0, &["rm", "--yes"]);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn project(variant: &str, name: &str) -> Project {
    let sample = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("samples/mise-sample")
        .join(variant);
    let dir = std::env::temp_dir().join(format!("devcroft-mise-e2e-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // `{{config_root}}` renders the canonical root (`/private/var` on macOS).
    let dir = dir.canonicalize().unwrap();
    for file in ["mise.toml", "mise.lock", "go.mod", "main.go"] {
        std::fs::copy(sample.join(file), dir.join(file)).unwrap();
    }
    std::fs::write(
        dir.join("devcroft.toml"),
        format!(
            "[sandbox]\nname = \"{name}\"\n[env]\nprovider = \"mise\"\n\
             [filesystem]\nallow = [\".\"]\n[network]\ndefault = \"deny\"\n"
        ),
    )
    .unwrap();
    Project(dir)
}

fn up(p: &Project) {
    let o = devcroft(&p.0, &["up"]);
    assert!(o.status.success(), "up failed: {}", text(&o));
    assert!(text(&o).contains("guarantee: artifact"), "{}", text(&o));
}

fn sh(p: &Project, script: &str) -> Output {
    devcroft(&p.0, &["exec", "--", "sh", "-c", script])
}

/// The installed `go`, and the object id it lives in.
fn installed_go(p: &Project) -> (String, String) {
    let o = sh(p, "command -v go");
    assert!(o.status.success(), "{}", text(&o));
    let go = String::from_utf8_lossy(&o.stdout).trim().to_owned();
    let id = go
        .split("/devcroft-artifacts/mise/")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_else(|| panic!("go is not in the artifact store: {go}"))
        .to_owned();
    (go, id)
}

fn store() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap()).join(".local/share/devcroft-artifacts/mise")
}

#[test]
fn locked_restore_runtime_boundaries_sharing_and_reclamation() {
    let Some(variant) = qualified_host() else {
        return;
    };
    let pid = std::process::id();
    let a = project(variant, &format!("misea{pid}"));
    let lock_before = std::fs::read(a.0.join("mise.lock")).unwrap();

    // 6.2 cold (or warm) locked restore; inputs unchanged.
    up(&a);
    assert_eq!(std::fs::read(a.0.join("mise.lock")).unwrap(), lock_before);
    let o = devcroft(
        &a.0,
        &[
            "exec",
            "--",
            "go",
            "build",
            "-buildvcs=false",
            "-o",
            "hello",
            ".",
        ],
    );
    assert!(o.status.success(), "build: {}", text(&o));
    let o = devcroft(&a.0, &["exec", "--", "./hello"]);
    assert!(
        text(&o).contains("built with locked mise Go"),
        "{}",
        text(&o)
    );

    // 6.4 `[env]` ran confined, anchored at the project.
    let o = sh(&a, "echo \"$CGO_ENABLED|$GOCACHE\"");
    let root = a.0.display().to_string();
    assert_eq!(text(&o).trim(), format!("0|{root}/.cache/go"));

    // 6.3 the installation is read-only and the host toolchain is absent.
    let (go, id) = installed_go(&a);
    let o = sh(&a, &format!("echo x >> '{go}'"));
    assert!(!o.status.success(), "a write to {go} succeeded");
    for tool in ["cc", "gcc", "ld", "git"] {
        let o = sh(&a, &format!("command -v {tool}"));
        assert!(!o.status.success(), "{tool} resolves: {}", text(&o));
        let o = sh(&a, &format!("/usr/bin/{tool} --version"));
        assert!(!o.status.success(), "/usr/bin/{tool} ran: {}", text(&o));
    }
    let render = text(&devcroft(&a.0, &["policy", "--render"]));
    assert!(render.contains("provider:mise"), "{render}");
    for line in render.lines() {
        assert!(
            !line.trim_start().starts_with("/usr/bin ") && line.trim() != "/usr/bin",
            "/usr/bin is granted: {line}"
        );
    }

    // 6.4 staleness: a newly consumed dotenv, then its removal.
    let manifest = std::fs::read_to_string(a.0.join("mise.toml")).unwrap();
    std::fs::write(a.0.join(".env.extra"), "EXTRA=1\n").unwrap();
    std::fs::write(
        a.0.join("mise.toml"),
        manifest.replace("[env]\n", "[env]\n_.file = \".env.extra\"\n"),
    )
    .unwrap();
    let status = text(&devcroft(&a.0, &["status"]));
    assert!(status.contains("env: stale"), "{status}");
    std::fs::write(a.0.join("mise.toml"), &manifest).unwrap();
    std::fs::remove_file(a.0.join(".env.extra")).unwrap();
    let status = text(&devcroft(&a.0, &["status"]));
    assert!(status.contains("env: fresh"), "{status}");

    // 6.2 two consumers of one object; the first `rm` keeps it.
    let b = project(variant, &format!("miseb{pid}"));
    up(&b);
    assert_eq!(installed_go(&b).1, id, "same recipe, different object");
    let o = devcroft(&a.0, &["rm", "--yes"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(store().join(&id).join("complete.sha256").is_file());
    let o = devcroft(&b.0, &["exec", "--", "go", "version"]);
    assert!(text(&o).contains("go1.24.0"), "{}", text(&o));

    // The last `rm` removes it, with its lock file.
    let o = devcroft(&b.0, &["rm", "--yes"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(
        !store().join(&id).exists(),
        "object {id} survived its last reference"
    );
    assert!(!store().join(format!(".{id}.lock")).exists());

    // 6.2 an interrupted installation (no completion record) is removed
    // under the object lock and installed again.
    let half = store().join(&id).join("data/installs/http-go/1.24.0/bin");
    std::fs::create_dir_all(&half).unwrap();
    std::fs::write(half.join("go"), "#!/bin/sh\necho forged\n").unwrap();
    up(&a);
    let o = devcroft(&a.0, &["exec", "--", "go", "version"]);
    assert!(text(&o).contains("go1.24.0"), "{}", text(&o));
}

#[test]
fn a_write_grant_over_the_artifact_store_is_refused_at_config() {
    let Some(variant) = qualified_host() else {
        return;
    };
    let p = project(variant, &format!("misew{}", std::process::id()));
    let share = PathBuf::from(std::env::var("HOME").unwrap()).join(".local/share");
    std::fs::write(
        p.0.join("devcroft.toml"),
        format!(
            "[sandbox]\nname = \"misew{}\"\n[env]\nprovider = \"mise\"\n\
             [filesystem]\nallow = [\".\", \"{}\"]\n[network]\ndefault = \"deny\"\n",
            std::process::id(),
            share.display()
        ),
    )
    .unwrap();
    let o = devcroft(&p.0, &["up"]);
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(text(&o).contains("config:"), "{}", text(&o));
    assert!(text(&o).contains("devcroft-artifacts/mise"), "{}", text(&o));
}
