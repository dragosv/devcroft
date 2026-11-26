#![cfg(target_os = "macos")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_devcroft");

struct Project {
    root: PathBuf,
    names: [String; 2],
}

impl Project {
    fn copy(tag: &str, sample: &str, files: &[&str]) -> Self {
        let pid = std::process::id();
        let root = PathBuf::from(format!("/private/tmp/dc-{tag}-{pid}"));
        fs::create_dir(&root).unwrap();
        let names = [format!("{tag}a{pid}"), format!("{tag}b{pid}")];
        let result = Self { root, names };
        let sample = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("samples")
            .join(sample);
        for name in files {
            let target = result.root.join(name);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(sample.join(name), target).unwrap();
        }
        result
    }

    fn manifest(&self, provider: &str, extra: &str) {
        fs::write(
            self.root.join("devcroft.toml"),
            format!(
                "[sandbox]\nname = {:?}\n[env]\nprovider = {provider:?}\n{extra}",
                self.names[0]
            ),
        )
        .unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(BIN)
            .args(args)
            .current_dir(&self.root)
            .env("DEVCROFT_KEEPER_EXE", BIN)
            .output()
            .unwrap()
    }

    fn start(&self, index: usize) -> String {
        checked(self.run(&["up", "--name", &self.names[index]]))
    }

    fn sh(&self, index: usize, script: &str) -> String {
        checked(self.run(&["exec", &self.names[index], "--", "sh", "-exc", script]))
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        for name in &self.names {
            let _ = self.run(&["rm", name, "--yes"]);
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn checked(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout).unwrap()
}

fn available(provider: &str) -> bool {
    let ready = devcroft::policy::backend_supported()
        && devcroft::provider::host_can_build_nix_closures()
        && Command::new(provider)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());
    if !ready {
        eprintln!("skipping: no usable Seatbelt, Nix store or {provider}");
    }
    ready
}

#[test]
fn temp_is_local_and_home_survives_restart() {
    if !available("flox") {
        return;
    }
    let project = Project::copy(
        "localtmp",
        "flox-services-sample",
        &[
            ".flox/env.json",
            ".flox/env/manifest.toml",
            ".flox/env/manifest.lock",
        ],
    );
    let sentinel = format!("{}.host", project.names[0]);
    struct Sentinel(PathBuf);
    impl Drop for Sentinel {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let host = Sentinel(PathBuf::from("/private/tmp").join(&sentinel));
    fs::write(&host.0, "host\n").unwrap();
    project.manifest("flox", "[identity]\nuser = \"dcscratch\"\n");
    project.start(0);
    project.start(1);
    let first = project.sh(
        0,
        &format!(
            "test \"$TMP\" = \"$TMPDIR\"; test \"$TEMP\" = \"$TMPDIR\"; \
        echo home > \"$HOME/keep\"; echo local > /tmp/../tmp/local; \
        test \"$(cat /private/tmp/./local)\" = local; \
        touch \"{}/workspace\"; test -f /tmp/{}/workspace; \
        test \"$(id -un)\" = dcscratch; test \"$(id -u)\" -gt 0; \
        name=$(mktemp); test -f \"$name\"; rm \"$name\"; \
        printf '%s\\n' \"$TMPDIR\"",
            project.root.display(),
            project.root.file_name().unwrap().to_str().unwrap(),
        ),
    );
    let first = PathBuf::from(first.trim());
    assert_eq!(
        fs::metadata(&first).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(fs::read_to_string(first.join("local")).unwrap(), "local\n");
    project.sh(
        1,
        &format!(
            "test ! -e /tmp/local; test ! -e /private/tmp/local; \
        ! cat /tmp/{sentinel}; ! cat /private/tmp/{sentinel}; \
        ! rm /tmp/{sentinel}; ! rm /private/tmp/{sentinel}; \
        ! cat '{}/local'",
            first.display(),
        ),
    );
    assert_eq!(fs::read_to_string(&host.0).unwrap(), "host\n");

    project.manifest("flox", "[filesystem]\nallow = [\".\", \"/tmp\"]\n");
    checked(project.run(&["up", "--recreate", "--yes", "--name", &project.names[1]]));
    project.sh(
        1,
        &format!(
            "test \"$(cat /tmp/{sentinel})\" = host; ! cat '{}/local'",
            first.display(),
        ),
    );
    let rendered = checked(project.run(&["policy", "--render", &project.names[1]]));
    assert!(
        rendered
            .lines()
            .any(|line| line.contains("/tmp ") && line.contains("manifest:filesystem.allow")),
        "{rendered}"
    );

    project.manifest("flox", "[identity]\nuser = \"dcscratch\"\n");
    checked(project.run(&["down", &project.names[0]]));
    assert_eq!(fs::read_dir(&first).unwrap().count(), 0);
    project.start(0);
    project.sh(0, "test -f \"$HOME/keep\"; test ! -e \"$TMPDIR/local\"");
    checked(project.run(&["rm", &project.names[0], "--yes"]));
    assert!(!first.exists());
}

#[test]
fn two_devenv_sandboxes_keep_their_own_run_links() {
    if !available("devenv") {
        return;
    }
    let project = Project::copy(
        "localdev",
        "devenv-sample",
        &["devenv.nix", "devenv.yaml", "devenv.lock"],
    );
    project.manifest("devenv", "");
    project.start(0);
    let path = project.sh(
        0,
        "echo first > \"$DEVENV_DOTFILE/run/probe\"; echo state > \"$DEVENV_STATE/probe\"; echo \"$DEVENV_RUNTIME\"",
    );
    project.start(1);
    let other = project.sh(
        1,
        "echo second > \"$DEVENV_RUNTIME/probe\"; echo \"$DEVENV_RUNTIME\"",
    );
    assert_ne!(path, other);
    project.sh(0, "test \"$(cat \"$DEVENV_DOTFILE/run/probe\")\" = first");
    project.sh(1, "test \"$(cat \"$DEVENV_DOTFILE/run/probe\")\" = second");
    assert!(!project.root.join(".devenv/run").exists());
    checked(project.run(&["down", &project.names[0]]));
    project.start(0);
    project.sh(0, "test -d \"$DEVENV_DOTFILE/run\"; test ! -e \"$DEVENV_RUNTIME/probe\"; test \"$(cat \"$DEVENV_STATE/probe\")\" = state");
}

#[test]
fn compiled_library_handles_path_calls_and_passwd() {
    if !available("flox") {
        return;
    }
    let project = Project::copy(
        "tempcall",
        "flox-services-sample",
        &[
            ".flox/env.json",
            ".flox/env/manifest.toml",
            ".flox/env/manifest.lock",
        ],
    );
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/macos-local-temp.c");
    checked(
        Command::new("cc")
            .args(["-Wall", "-Wextra", "-Werror", "-o"])
            .arg(project.root.join("temp-probe"))
            .arg(source)
            .output()
            .unwrap(),
    );
    project.manifest("flox", "[network]\ndefault = \"allow\"\n");
    project.start(0);
    project.sh(0, "cp ./temp-probe /tmp/temp-probe");
    checked(project.run(&["exec", &project.names[0], "--", "/tmp/temp-probe"]));

    checked(project.run(&[
        "exec",
        &project.names[0],
        "--",
        "python3",
        "-c",
        r#"
import os, tempfile
for directory in (None, '/tmp', '/private/tmp'):
    with tempfile.NamedTemporaryFile(dir=directory) as f:
        physical = os.path.join(os.environ['TMPDIR'], os.path.basename(f.name))
        assert os.path.samestat(os.fstat(f.fileno()), os.stat(physical))
        f.write(b'scratch')
        f.flush()
        assert open(f.name, 'rb').read() == b'scratch'
"#,
    ]));

    if let Ok(path) = std::env::var("DEVCROFT_TEST_NODE") {
        checked(project.run(&[
            "exec",
            &project.names[0],
            "--",
            &path,
            "-e",
            r#"
const fs = require('node:fs'), os = require('node:os');
for (const prefix of [os.tmpdir(), '/tmp', '/private/tmp']) {
    const path = fs.mkdtempSync(prefix + '/devcroft-node-');
    const physical = process.env.TMPDIR + '/' + path.split('/').at(-1);
    fs.writeFileSync(path + '/x', 'node');
    if (fs.realpathSync.native(path) !== physical ||
        fs.readFileSync(physical + '/x', 'utf8') !== 'node') throw Error('temp');
    fs.rmSync(path, {recursive: true});
}
"#,
        ]));
    } else {
        eprintln!("skipping: set DEVCROFT_TEST_NODE to a closure-provided Node executable");
    }

    if Command::new("go")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("spikes/macos-local-temp/probe.go");
        checked(
            Command::new("go")
                .arg("build")
                .arg("-o")
                .arg(project.root.join("go-probe"))
                .arg(source)
                .env("CGO_ENABLED", "0")
                .env("GOCACHE", project.root.join("go-cache"))
                .output()
                .unwrap(),
        );
        project.sh(0, "DC_TAG=devcroft-go ./go-probe");
    } else {
        eprintln!("skipping: no Go compiler for the temp and socket probe");
    }
}
