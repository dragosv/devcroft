//! `embed-brush-runtime` 1.4: the runtime's shell, reached as a sandbox
//! reaches it, through a link named `sh` to the devcroft binary.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

struct Shell(PathBuf);

impl Shell {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("dc-sh-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).unwrap();
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_devcroft"), dir.join("sh")).unwrap();
        Shell(dir.canonicalize().unwrap())
    }

    fn command(&self) -> Command {
        let mut c = Command::new(self.0.join("sh"));
        c.current_dir(&self.0).env("HOME", self.0.join("home"));
        c
    }

    fn c(&self, script: &str) -> Output {
        self.command().args(["-c", script]).output().unwrap()
    }

    fn stdin(&self, script: &str) -> Output {
        let mut child = self
            .command()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(script.as_bytes()).unwrap();
        drop(stdin);
        child.wait_with_output().unwrap()
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn nothing_is_evaluated_before_the_command() {
    let s = Shell::new("startup");
    let home = s.0.join("home");
    std::fs::create_dir_all(home.join(".config/brush")).unwrap();
    let mark = |from: &str| format!("echo {from} >> {}/marker\n", s.0.display());
    for f in [
        ".profile",
        ".bash_profile",
        ".bash_login",
        ".bashrc",
        ".brushrc",
        ".shrc",
    ] {
        std::fs::write(home.join(f), mark(f)).unwrap();
    }
    std::fs::write(s.0.join("env.sh"), mark("ENV")).unwrap();
    std::fs::write(home.join(".config/brush/config.toml"), "not toml [").unwrap();
    for (argv0, args) in [
        ("sh", vec!["-c", "true"]),
        ("-sh", vec!["-c", "true"]),
        ("sh", vec!["-l", "-c", "true"]),
        ("sh", vec!["-i", "-c", "true"]),
        ("sh", vec![]),
    ] {
        use std::os::unix::process::CommandExt;
        let o = s
            .command()
            .arg0(argv0)
            .args(&args)
            .env("ENV", s.0.join("env.sh"))
            .env("BASH_ENV", s.0.join("env.sh"))
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(o.status.success(), "{argv0} {args:?}: {o:?}");
    }
    assert!(
        !s.0.join("marker").exists(),
        "{:?}",
        std::fs::read_to_string(s.0.join("marker"))
    );
}

#[test]
fn it_is_posix_and_takes_a_shells_arguments() {
    let s = Shell::new("args");
    let set = out(&s.c("set -o"));
    assert!(
        set.lines()
            .any(|l| l.starts_with("posix") && l.trim_end().ends_with("on")),
        "{set}"
    );
    let o = s
        .command()
        .args(["-c", "echo $0 \"$1\" $#", "nm", "a b"])
        .output()
        .unwrap();
    assert_eq!(out(&o), "nm a b 1\n");
    std::fs::write(s.0.join("x.sh"), "echo \"$0\" \"$@\"; exit 4\n").unwrap();
    let o = s.command().args(["x.sh", "p", "q r"]).output().unwrap();
    assert_eq!(
        (out(&o).as_str(), o.status.code()),
        ("x.sh p q r\n", Some(4))
    );
    for (args, code) in [
        (vec!["missing.sh"], 127),
        (vec!["-e", "-c", "false; exit 0"], 1),
        (vec!["-Z"], 2),
        (vec!["-c", "exit 7"], 7),
        (vec!["-c", "no_such_command_zz"], 127),
    ] {
        let o = s.command().args(&args).output().unwrap();
        assert_eq!(o.status.code(), Some(code), "{args:?}: {o:?}");
    }
    // A descriptor it was given reaches a redirection.
    let o = s.c("exec 3>f; echo three >&3; cat f");
    assert_eq!(out(&o), "three\n");
}

#[test]
fn standard_input_is_read_a_complete_command_at_a_time() {
    let s = Shell::new("stdin");
    let o = s.stdin("if true\nthen echo multi\nfi\ncat <<X\nhere $((1+1))\nX\necho a \\\n b\n");
    assert_eq!(out(&o), "multi\nhere 2\na b\n", "{o:?}");
    // What a command reads is not swallowed by the shell.
    assert_eq!(out(&s.stdin("cat\nrest\n")), "rest\n");
}

#[test]
fn a_substitution_larger_than_a_pipe_does_not_hang() {
    let s = Shell::new("big");
    let script = "x=$(i=0; while [ $i -lt 20000 ]; do echo $i; i=$((i+1)); done); echo ${#x}";
    let mut child = s
        .command()
        .args(["-c", script])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed().as_secs() > 60 {
            child.kill().unwrap();
            panic!("hung");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(out(&child.wait_with_output().unwrap()), "108889\n");
}
