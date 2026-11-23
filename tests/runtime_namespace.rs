//! The absolute-path namespace and the runtime's write protection, in real
//! sandboxes on both platforms (`add-own-userland` 4.2, 4.4a, 4.6).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_devcroft");

struct Sandbox {
    name: String,
    root: PathBuf,
}

fn dc(root: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(root)
        .env("DEVCROFT_KEEPER_EXE", BIN)
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

impl Sandbox {
    /// A project of `sample`'s provider, already locked, brought up.
    fn up(tag: &str, provider: &str, sample: &str, files: &[&str]) -> Option<Sandbox> {
        if !devcroft::policy::backend_supported()
            || !devcroft::provider::host_can_build_nix_closures()
            || Command::new(provider).arg("--version").output().is_err()
        {
            eprintln!("skipping: no usable backend, nix store or {provider} here");
            return None;
        }
        // Short: a services sample's socket lives under this path.
        let name = format!("{tag}{}", std::process::id());
        let root = std::env::temp_dir().join(&name);
        for f in files {
            let to = root.join(f);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples");
            std::fs::copy(from.join(sample).join(f), to).unwrap();
        }
        let root = root.canonicalize().unwrap();
        std::fs::write(
            root.join("devcroft.toml"),
            format!(
                "[sandbox]\nname = \"{name}\"\n[env]\nprovider = \"{provider}\"\n\
                 [filesystem]\nallow = [\".\"]\n"
            ),
        )
        .unwrap();
        let s = Sandbox { name, root };
        let up = dc(&s.root, &["up"]);
        assert!(up.status.success(), "{}", text(&up));
        Some(s)
    }

    fn sh(&self, script: &str) -> String {
        text(&dc(&self.root, &["exec", "--", "sh", "-c", script]))
    }

    fn file(&self, name: &str, contents: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = self.root.join(name);
        std::fs::write(&path, contents).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = dc(&self.root, &["rm", "--yes"]);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn minimal_flox(tag: &str) -> Option<Sandbox> {
    Sandbox::up(
        tag,
        "flox",
        "flox-services-sample",
        &[
            ".flox/env.json",
            ".flox/env/manifest.toml",
            ".flox/env/manifest.lock",
        ],
    )
}

#[test]
fn a_minimal_environment_runs_the_runtime_at_absolute_paths() {
    let Some(s) = minimal_flox("na") else { return };
    s.file(
        "e.py",
        "#!/usr/bin/env python3\nimport os\nos.system('echo via-system')\n",
    );
    assert_eq!(s.sh("./e.py"), "via-system\n");
    for path in ["ls", "/bin/ls", "/usr/bin/ls"] {
        assert!(
            s.sh(&format!("{path} --version"))
                .starts_with("ls (uutils coreutils)"),
            "{path}"
        );
    }
    for path in [
        "/bin/awk",
        "/usr/bin/awk",
        "/usr/bin/python3",
        "/usr/bin/make",
    ] {
        let out = s.sh(&format!("{path} --version; echo rc=$?"));
        assert!(out.contains("rc=126") || out.contains("rc=127"), "{out}");
        if cfg!(target_os = "linux") {
            assert!(out.contains("rc=127"), "absent from the view: {out}");
        }
    }
    let render = text(&dc(&s.root, &["policy", "--render"]));
    for lookup in ["/bin/ls", "/usr/bin/env"] {
        assert!(
            render
                .lines()
                .any(|l| l.trim_start().starts_with(&format!("{lookup} "))
                    && l.contains("runtime:gen-1:")),
            "{lookup}: {render}"
        );
    }
    if cfg!(target_os = "linux") {
        let why = text(&dc(&s.root, &["why", "--path", "/bin/ls", "--op", "read"]));
        assert!(why.contains("ALLOWED"), "{why}");
    }
}

#[test]
fn the_closures_own_userland_is_what_absolute_paths_run() {
    let Some(s) = Sandbox::up(
        "nn",
        "nix",
        "nix-flake-sample",
        &["flake.nix", "flake.lock"],
    ) else {
        return;
    };
    s.file("e.sh", "#!/usr/bin/env sh\necho env-ran\n");
    assert_eq!(s.sh("./e.sh"), "env-ran\n");
    for path in ["ls", "/bin/ls", "/usr/bin/ls"] {
        assert!(
            s.sh(&format!("{path} --version")).contains("GNU coreutils"),
            "{path}"
        );
    }
}

#[test]
fn the_runtime_cannot_be_modified_from_inside() {
    let Some(s) = minimal_flox("nw") else { return };
    let art = s.root.join(".devcroft").join(&s.name);
    let before = |d: &str| {
        let mut v: Vec<_> = std::fs::read_dir(art.join(d))
            .unwrap()
            .map(|e| {
                let p = e.unwrap().path();
                (p.clone(), std::fs::read_link(&p).ok())
            })
            .collect();
        v.sort();
        v
    };
    let dirs: &[&str] = if cfg!(target_os = "macos") {
        &["runtime", "ns", "redirect"]
    } else {
        &["runtime", "ns"]
    };
    let saved: Vec<_> = dirs.iter().map(|d| before(d)).collect();
    let a = art.display();
    for attempt in [
        format!("ln -sf /tmp/x {a}/ns/ls"),
        format!("rm -rf {a}/runtime"),
        format!("rm -f {a}/runtime/ls"),
        format!("rm -rf {a}/ns"),
        format!("mv {a}/runtime {a}/moved"),
        format!("echo x > {a}/ns/new"),
        format!("chmod 000 {a}/runtime"),
        format!("rm -f {a}/{}/*", dirs[dirs.len() - 1]),
    ] {
        let out = s.sh(&format!("{attempt}; echo rc=$?"));
        println!("{attempt}: {out}");
        assert!(!out.ends_with("rc=0\n"), "{attempt}: {out}");
    }
    for (d, saved) in dirs.iter().zip(&saved) {
        assert_eq!(&before(d), saved, "{d}");
    }
    // Moving a parent aside (Linux allows it) changes no absolute path.
    let out = s.sh(&format!(
        "mv {a} {root}/.devcroft/away && mkdir -p {a}/ns {a}/runtime \
         && printf '#!/bin/sh\\necho planted\\n' > {a}/runtime/ls; \
         /bin/ls --version; echo rc=$?",
        root = s.root.display()
    ));
    println!("parent moved: {out}");
    assert!(out.contains("ls (uutils coreutils)"), "{out}");
    let away = s.root.join(".devcroft/away");
    if away.exists() {
        let _ = std::fs::remove_dir_all(&art);
        std::fs::rename(&away, &art).unwrap();
    }
    // The project around it stays writable, and the denial is shown.
    assert!(s.sh("echo x > here && echo ok").contains("ok"));
    let render = text(&dc(&s.root, &["policy", "--render"]));
    let denied = render
        .split("filesystem.write_deny:")
        .nth(1)
        .unwrap_or_else(|| panic!("{render}"));
    for d in dirs {
        assert!(denied.contains(&format!("/{d} ")), "{d}: {render}");
    }
    let why = text(&dc(
        &s.root,
        &["why", "--path", &format!("{a}/ns/ls"), "--op", "write"],
    ));
    assert!(
        why.contains("DENIED") && why.contains("runtime:gen-1"),
        "{why}"
    );
}
