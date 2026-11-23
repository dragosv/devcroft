//! `add-own-userland` 1.4: the runtime's applets, dispatched on `argv[0]`.
//!
//! Each case runs the real `devcroft` binary through a link named after
//! the applet, the way a sandbox's namespace and `PATH` will reach it.

use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Links(PathBuf);

impl Links {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("dc-applets-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for applet in devcroft::runtime::applets::INVENTORY {
            std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_devcroft"), dir.join(applet.name))
                .unwrap();
        }
        Links(dir)
    }

    fn cmd(&self, name: &str) -> Command {
        Command::new(self.0.join(name))
    }
}

impl Drop for Links {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn every_applet_dispatches_on_its_link_name() {
    let links = Links::new("each");
    for applet in devcroft::runtime::applets::INVENTORY {
        if applet.name == "test" {
            // POSIX `test` with one non-empty argument is simply true; GNU
            // reads `--version` only when invoked as `[`. Its status is the
            // evidence instead.
            assert!(
                links
                    .cmd("test")
                    .args(["1", "-eq", "1"])
                    .status()
                    .unwrap()
                    .success()
            );
            assert_eq!(
                links
                    .cmd("test")
                    .args(["1", "-eq", "2"])
                    .status()
                    .unwrap()
                    .code(),
                Some(1)
            );
            continue;
        }
        let out = links.cmd(applet.name).arg("--version").output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        // `true` and `false` exit with their own status whatever the flag;
        // every applet still prints its upstream version line.
        assert!(
            text.contains("(uutils coreutils) ")
                && text.contains(devcroft::runtime::applets::VERSION),
            "{} --version printed {text:?} (stderr {:?})",
            applet.name,
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !text.contains("devcroft —"),
            "{} fell through to devcroft's own usage",
            applet.name
        );
    }
}

#[test]
fn devcroft_ls_is_still_an_unknown_command() {
    let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .arg("ls")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown command \"ls\""));
}

#[test]
fn a_closed_pipe_ends_ls_by_sigpipe_without_a_panic() {
    let links = Links::new("pipe");
    // Far more than a pipe buffer (64 KiB on Linux), so `ls` is still
    // writing when `head` quits: 5000 short names fit, and `ls` then exits 0
    // having never seen the closed pipe (measured).
    let many = links.0.join("many");
    std::fs::create_dir(&many).unwrap();
    for i in 0..20000 {
        std::fs::write(
            many.join(format!("entry-{i:05}-padding-padding-padding")),
            "",
        )
        .unwrap();
    }
    let mut ls = links
        .cmd("ls")
        .args(["-1", &many.to_string_lossy()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let ls_out = ls.stdout.take().unwrap();
    let head = links
        .cmd("head")
        .args(["-n", "1"])
        .stdin(ls_out)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&head.stdout),
        "entry-00000-padding-padding-padding\n"
    );
    let status = ls.wait_with_output().unwrap();
    assert_eq!(
        status.status.signal(),
        Some(libc::SIGPIPE),
        "ls should die by SIGPIPE, got {:?}",
        status.status
    );
    let stderr = String::from_utf8_lossy(&status.stderr);
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[test]
fn cat_on_a_missing_file_reports_text_not_a_translation_key() {
    let links = Links::new("cat");
    let out = links
        .cmd("cat")
        .arg("/nonexistent/devcroft-test")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("No such file or directory"),
        "untranslated diagnostic: {stderr:?}"
    );
}

#[test]
fn help_is_real_text() {
    let links = Links::new("help");
    let out = links.cmd("ls").arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    // A missing bundle prints message ids such as `ls-about`.
    assert!(text.contains("List directory contents"), "{text}");
    assert!(!text.contains("ls-about"), "{text}");
}

#[test]
fn env_runs_a_command_found_on_path() {
    let links = Links::new("env");
    let out = links
        .cmd("env")
        .args(["-i", &format!("PATH={}", links.0.display()), "echo", "hi"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hi\n");
    assert!(Path::new(&links.0.join("echo")).exists());
}
