//! The pixi provider, end to end (`add-pixi-provider` group 5), driven
//! through the real binary against `samples/pixi-sample`.
//!
//! Skips unless the host is a qualified runtime target (native ARM64, glibc
//! on Linux), the process tier is enforceable, and the `pixi` on `PATH` is at
//! least the qualified release: an older pixi is refused by the provider, so
//! running against it would measure the refusal rather than the provider.
//!
//! One test drives the sequence that shares the real artifact store under
//! `~/.local/share/devcroft-artifacts/pixi`, because the steps that remove
//! objects must not interleave with the steps that read them. The first
//! `up` downloads the locked conda-forge packages.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const QUALIFIED: (u32, u32, u32) = (0, 84, 2);

/// Every test here shares the real artifact store, and a sandbox one test
/// brings up can reference the object another test expects its last `rm` to
/// remove (same recipe, same object, measured on Linux). They run one at a
/// time.
static STORE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serialized() -> std::sync::MutexGuard<'static, ()> {
    STORE.lock().unwrap_or_else(|e| e.into_inner())
}

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

fn qualified_host() -> bool {
    let native = matches!(
        (std::env::consts::OS, std::env::consts::ARCH),
        ("macos", "aarch64")
    ) || (std::env::consts::OS == "linux"
        && std::env::consts::ARCH == "aarch64"
        && cfg!(target_env = "gnu"));
    if !native {
        eprintln!("skipping: pixi runtime is qualified only on native ARM64");
        return false;
    }
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: the process-tier backend is not enforceable on this host");
        return false;
    }
    let version = Command::new("pixi")
        .arg("--version")
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", std::env::temp_dir())
        .env("PIXI_NO_CONFIG", "true")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let parsed = version.as_deref().and_then(|v| {
        let word = v.split_whitespace().nth(1)?;
        let mut parts = word.split('.');
        Some((
            parts.next()?.parse::<u32>().ok()?,
            parts.next()?.parse::<u32>().ok()?,
            parts.next()?.parse::<u32>().ok()?,
        ))
    });
    match parsed {
        Some(v) if v >= QUALIFIED => true,
        _ => {
            eprintln!("skipping: pixi >= 0.84.2 is not on PATH (found {version:?})");
            false
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

fn project(name: &str, devcroft_toml_extra: &str) -> Project {
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/pixi-sample");
    let dir = std::env::temp_dir().join(format!("devcroft-pixi-e2e-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("scripts")).unwrap();
    let dir = dir.canonicalize().unwrap();
    for file in ["pixi.toml", "pixi.lock", "check.py", "scripts/activate.sh"] {
        std::fs::copy(sample.join(file), dir.join(file)).unwrap();
    }
    std::fs::write(
        dir.join("devcroft.toml"),
        format!(
            "[sandbox]\nname = \"{name}\"\n[env]\nprovider = \"pixi\"\n\
             [filesystem]\nallow = [\".\"]\n[network]\ndefault = \"deny\"\n{devcroft_toml_extra}"
        ),
    )
    .unwrap();
    Project(dir)
}

fn up(p: &Project) -> String {
    let o = devcroft(&p.0, &["up"]);
    assert!(o.status.success(), "up failed: {}", text(&o));
    assert!(text(&o).contains("guarantee: artifact"), "{}", text(&o));
    text(&o)
}

fn sh(p: &Project, script: &str) -> Output {
    devcroft(&p.0, &["exec", "--", "sh", "-c", script])
}

/// The environment's `python`, and the object id it lives in.
fn installed_python(p: &Project) -> (String, String) {
    let o = sh(p, "command -v python");
    assert!(o.status.success(), "{}", text(&o));
    let python = String::from_utf8_lossy(&o.stdout).trim().to_owned();
    let id = python
        .split("/devcroft-artifacts/pixi/")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_else(|| panic!("python is not in the artifact store: {python}"))
        .to_owned();
    (python, id)
}

fn store() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap()).join(".local/share/devcroft-artifacts/pixi")
}

/// The record's line for one lookup.
fn entry<'a>(render: &'a str, lookup: &str) -> &'a str {
    render
        .lines()
        .find(|l| l.trim_start().starts_with(&format!("{lookup} ")))
        .unwrap_or_default()
}

/// `text` as an executable file, run by name from the project.
fn script(p: &Project, text: &str) -> Output {
    use std::os::unix::fs::PermissionsExt;
    let file = p.0.join("shebang-probe");
    std::fs::write(&file, text).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    let o = devcroft(&p.0, &["exec", "--", "./shebang-probe"]);
    std::fs::remove_file(&file).unwrap();
    o
}

/// `embed-brush-runtime` 2.5: an environment with a `bash` of its own keeps
/// it, as `sh` too, by name and by path.
#[test]
fn an_environment_with_bash_has_that_bash_as_its_shell() {
    let _store = serialized();
    if !qualified_host() {
        return;
    }
    let name = format!("pixish{}", std::process::id());
    let dir = std::env::temp_dir().join(format!("devcroft-pixi-e2e-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pixi-bash");
    for file in ["pixi.toml", "pixi.lock"] {
        std::fs::copy(fixture.join(file), dir.join(file)).unwrap();
    }
    std::fs::write(
        dir.join("devcroft.toml"),
        format!(
            "[sandbox]\nname = \"{name}\"\n[env]\nprovider = \"pixi\"\n\
             [filesystem]\nallow = [\".\"]\n[network]\ndefault = \"deny\"\n"
        ),
    )
    .unwrap();
    let p = Project(dir);
    let up = up(&p);
    assert!(!up.contains("no shell"), "{up}");
    let render = text(&devcroft(&p.0, &["policy", "--render"]));
    for lookup in ["sh (by name)", "/bin/sh"] {
        let line = entry(&render, lookup);
        assert!(line.contains("provider:pixi:"), "{lookup}: {render}");
    }
    assert!(render.contains(" host 0,"), "{render}");
    for command in ["sh", "/bin/sh"] {
        let o = devcroft(
            &p.0,
            &[
                "exec",
                "--",
                "sh",
                "-c",
                &format!("{command} -c 'echo $BASH_VERSION'"),
            ],
        );
        assert!(text(&o).starts_with("5."), "{command}: {}", text(&o));
    }
    for shebang in ["#!/bin/sh", "#!/usr/bin/env bash"] {
        let o = script(&p, &format!("{shebang}\necho v$BASH_VERSION\n"));
        assert!(text(&o).starts_with("v5."), "{shebang}: {}", text(&o));
    }
    // `/bin/bash` is no base name: nothing is there, and nothing answers.
    let o = script(&p, "#!/bin/bash\necho ran-bash\n");
    assert!(!text(&o).contains("ran-bash"), "{}", text(&o));
}

#[test]
fn locked_restore_activation_boundaries_sharing_and_reclamation() {
    let _store = serialized();
    if !qualified_host() {
        return;
    }
    let pid = std::process::id();
    let a = project(&format!("pixia{pid}"), "");
    let lock_before = std::fs::read(a.0.join("pixi.lock")).unwrap();

    // 5.2 cold (or warm) locked restore; inputs unchanged; nothing written
    // into the project's own `.pixi`.
    up(&a);
    assert_eq!(std::fs::read(a.0.join("pixi.lock")).unwrap(), lock_before);
    assert!(!a.0.join(".pixi").exists(), "up wrote the project's .pixi");
    let o = devcroft(&a.0, &["exec", "--", "python", "check.py"]);
    let out = text(&o);
    assert!(o.status.success(), "{out}");
    assert!(out.contains("numpy") && out.contains("sum 45"), "{out}");

    // 5.4 activation ran confined, on the real project, and sessions got it.
    let root = a.0.display().to_string();
    assert!(
        out.contains(&format!("activation: {root} | hello from [activation].env")),
        "{out}"
    );
    let o = sh(&a, "echo \"$PIXI_PROJECT_ROOT|${PIXI_EXE-unset}\"");
    assert_eq!(text(&o).trim(), format!("{root}|unset"));

    // 5.3 the prefix is read-only and the host toolchain is absent.
    let (python, id) = installed_python(&a);
    let o = sh(&a, &format!("echo x >> '{python}'"));
    assert!(!o.status.success(), "a write to {python} succeeded");
    for tool in ["cc", "gcc", "ld", "git", "pixi"] {
        let o = sh(&a, &format!("command -v {tool}"));
        assert!(!o.status.success(), "{tool} resolves: {}", text(&o));
    }
    for tool in ["cc", "gcc", "ld", "git"] {
        let o = sh(&a, &format!("/usr/bin/{tool} --version"));
        assert!(!o.status.success(), "/usr/bin/{tool} ran: {}", text(&o));
    }
    let render = text(&devcroft(&a.0, &["policy", "--render"]));
    assert!(render.contains("provider:pixi"), "{render}");
    for line in render.lines() {
        let path = line.split_whitespace().next().unwrap_or("");
        assert!(
            path != "/usr/bin" && path != "/usr",
            "host program directory granted: {line}"
        );
    }

    // `embed-brush-runtime` 2.5: no shell in this environment, so `sh` is the
    // runtime's, by name and by path, its activation script ran in it, and no
    // `bash` answers for one.
    for lookup in ["sh (by name)", "/bin/sh"] {
        let line = entry(&render, lookup);
        assert!(line.contains("runtime:gen-2:"), "{lookup}: {render}");
    }
    assert!(render.contains(" host 0,"), "{render}");
    let o = script(&a, "#!/bin/sh\necho from-shebang\n");
    assert!(text(&o).contains("from-shebang"), "{}", text(&o));
    for shebang in ["#!/bin/bash", "#!/usr/bin/env bash"] {
        let o = script(&a, &format!("{shebang}\necho ran-bash\n"));
        assert!(
            !o.status.success() && !text(&o).contains("ran-bash"),
            "{shebang}: {}",
            text(&o)
        );
    }

    // 5.4 staleness on an edited and on a deleted activation script.
    let script = a.0.join("scripts/activate.sh");
    let original = std::fs::read(&script).unwrap();
    std::fs::write(&script, b"export SAMPLE_ACTIVATED_IN=edited\n").unwrap();
    assert!(text(&devcroft(&a.0, &["status"])).contains("env: stale"));
    std::fs::remove_file(&script).unwrap();
    assert!(text(&devcroft(&a.0, &["status"])).contains("env: stale"));
    std::fs::write(&script, &original).unwrap();
    assert!(text(&devcroft(&a.0, &["status"])).contains("env: fresh"));

    // 5.2 two sandboxes of one recipe share one object, each with its own
    // home; the first `rm` keeps it.
    let b = project(&format!("pixib{pid}"), "");
    up(&b);
    assert_eq!(installed_python(&b).1, id, "same recipe, different object");
    let home = |p: &Project| text(&sh(p, "echo $HOME")).trim().to_owned();
    assert_ne!(home(&a), home(&b));
    let o = devcroft(&a.0, &["rm", "--yes"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(store().join(&id).join("complete.sha256").is_file());
    let o = devcroft(&b.0, &["exec", "--", "python", "check.py"]);
    assert!(text(&o).contains("sum 45"), "{}", text(&o));

    // The last `rm` removes it, with its lock file.
    let o = devcroft(&b.0, &["rm", "--yes"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(
        !store().join(&id).exists(),
        "object {id} survived its last reference"
    );
    assert!(!store().join(format!(".{id}.lock")).exists());

    // 5.2 an interrupted installation (no completion record) is removed
    // under the object lock and installed again.
    let half = store().join(&id).join(".pixi/envs/default/bin");
    std::fs::create_dir_all(&half).unwrap();
    std::fs::write(half.join("python"), "#!/bin/sh\necho forged\n").unwrap();
    up(&a);
    let o = devcroft(&a.0, &["exec", "--", "python", "check.py"]);
    assert!(text(&o).contains("sum 45"), "{}", text(&o));
}

#[test]
fn refusals_that_need_no_installation() {
    let _store = serialized();
    if !qualified_host() {
        return;
    }
    let pid = std::process::id();

    // 5.3 a write grant over the artifact store fails at config.
    let share = PathBuf::from(std::env::var("HOME").unwrap()).join(".local/share");
    let p = project(&format!("pixiw{pid}"), "");
    let manifest = std::fs::read_to_string(p.0.join("devcroft.toml")).unwrap();
    std::fs::write(
        p.0.join("devcroft.toml"),
        manifest.replace(
            "allow = [\".\"]",
            &format!("allow = [\".\", \"{}\"]", share.display()),
        ),
    )
    .unwrap();
    let o = devcroft(&p.0, &["up"]);
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(text(&o).contains("devcroft-artifacts"), "{}", text(&o));

    // 5.1 `[env.pixi]` under another provider, and an unknown key, fail at config.
    let q = project(&format!("pixic{pid}"), "");
    std::fs::write(
        q.0.join("devcroft.toml"),
        format!("[sandbox]\nname = \"pixic{pid}\"\n[env]\nprovider = \"flox\"\n[env.pixi]\nenvironment = \"default\"\n"),
    )
    .unwrap();
    let o = devcroft(&q.0, &["up"]);
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(text(&o).contains("env.pixi"), "{}", text(&o));
    std::fs::write(
        q.0.join("devcroft.toml"),
        format!("[sandbox]\nname = \"pixic{pid}\"\n[env]\nprovider = \"pixi\"\n[env.pixi]\nenviroment = \"default\"\n"),
    )
    .unwrap();
    let o = devcroft(&q.0, &["up"]);
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(text(&o).contains("env.pixi.enviroment"), "{}", text(&o));

    // 5.1 a PyPI dependency in the lock is refused at provider, by name.
    let r = project(&format!("pixip{pid}"), "");
    let lock = std::fs::read_to_string(r.0.join("pixi.lock")).unwrap();
    let platform = if cfg!(target_os = "macos") {
        "osx-arm64"
    } else {
        "linux-aarch64"
    };
    let marker = format!("      {platform}:\n");
    let pypi = "      - pypi: https://files.pythonhosted.org/packages/d9/5a/e7c31adbe875f2abbb91bd84cf2dc52d792b5a01506781dbcf25c91daf11/six-1.16.0-py2.py3-none-any.whl\n";
    std::fs::write(
        r.0.join("pixi.lock"),
        lock.replacen(&marker, &format!("{marker}{pypi}"), 1),
    )
    .unwrap();
    let o = devcroft(&r.0, &["up"]);
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    assert!(
        text(&o).contains("six-1.16.0-py2.py3-none-any.whl: PyPI packages are refused"),
        "{}",
        text(&o)
    );

    // 5.5 a declared service port under pixi fails at config.
    let s = project(
        &format!("pixis{pid}"),
        "[network.services.web]\nport = 8080\n",
    );
    let o = devcroft(&s.0, &["up"]);
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));

    // 5.5 a flox manifest's services beside a pixi selection fail at
    // provider, naming them, unless `--skip-hooks` says they would not start.
    let f = project(&format!("pixif{pid}"), "");
    std::fs::create_dir_all(f.0.join(".flox/env")).unwrap();
    std::fs::write(
        f.0.join(".flox/env/manifest.toml"),
        "version = 1\n[services]\nweb.command = \"true\"\n",
    )
    .unwrap();
    let o = devcroft(&f.0, &["up"]);
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    assert!(text(&o).contains("web"), "{}", text(&o));
    let o = devcroft(&f.0, &["up", "--skip-hooks"]);
    assert!(o.status.success(), "{}", text(&o));
}

/// `samples/pixi-go-sample`: a Go service with its whole toolchain from the
/// environment (go, golangci-lint, make, git, curl, jq, coreutils). `make
/// check` lints, vets, tests and builds; `make smoke` serves on the declared
/// loopback port and queries it. No host tool is reachable.
#[test]
fn go_sample_builds_lints_tests_and_serves_from_the_environment_alone() {
    let _store = serialized();
    if !qualified_host() {
        return;
    }
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/pixi-go-sample");
    let name = format!("pixigo{}", std::process::id());
    let dir = std::env::temp_dir().join(format!("devcroft-pixi-e2e-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    let status = Command::new("cp")
        .arg("-R")
        .arg(&sample)
        .arg(&dir)
        .status()
        .unwrap();
    assert!(status.success());
    let dir = dir.canonicalize().unwrap();
    for leftover in [".devcroft", ".bin", ".pixi"] {
        let _ = std::fs::remove_dir_all(dir.join(leftover));
    }
    let manifest = std::fs::read_to_string(dir.join("devcroft.toml")).unwrap();
    std::fs::write(
        dir.join("devcroft.toml"),
        manifest.replace("name = \"pixigo\"", &format!("name = \"{name}\"")),
    )
    .unwrap();
    let p = Project(dir);

    up(&p);
    let o = devcroft(&p.0, &["exec", "--", "make", "tools"]);
    let tools = text(&o);
    assert!(o.status.success(), "{tools}");
    for line in tools.lines() {
        assert!(
            line.contains("/devcroft-artifacts/pixi/"),
            "tool outside the prefix: {line}"
        );
    }
    let o = devcroft(&p.0, &["exec", "--", "make", "check"]);
    assert!(o.status.success(), "make check: {}", text(&o));
    assert!(text(&o).contains("0 issues."), "{}", text(&o));
    let o = devcroft(&p.0, &["exec", "--", "make", "smoke"]);
    assert!(text(&o).contains("smoke ok:"), "make smoke: {}", text(&o));
    let o = sh(&p, "/usr/bin/git --version");
    assert!(!o.status.success(), "host git ran: {}", text(&o));
}
