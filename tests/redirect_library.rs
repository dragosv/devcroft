//! The macOS exec-redirect library on its own, outside any sandbox
//! (`add-macos-exec-redirect` 1.3): the mapping table, shebang parsing, and
//! error passthrough.
//!
//! Outside a sandbox the host's `/bin` is reachable, so an exec that is not
//! redirected runs the host's file; that is what makes "passed through"
//! observable here. Namespace entries are small bash scripts that announce
//! themselves and their arguments. The exec comes from devcroft's own `env`
//! applet (a link named `env` to the devcroft binary, which dyld loads the
//! library into) and from a C program calling `posix_spawn`.
#![cfg(target_os = "macos")]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Fixture {
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn write_exec(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir()
            .join(format!("dc-redirect-{tag}-{}", std::process::id()))
            .canonicalize()
            .unwrap_or_else(|_| {
                let p =
                    std::env::temp_dir().join(format!("dc-redirect-{tag}-{}", std::process::id()));
                std::fs::create_dir_all(&p).unwrap();
                p.canonicalize().unwrap()
            });
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("ns")).unwrap();
        std::fs::create_dir_all(root.join("rt")).unwrap();
        devcroft::runtime::redirect::materialize(&root.join("lib.dylib")).unwrap();
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_devcroft"), root.join("rt/env")).unwrap();
        // An entry that says it ran, with its arguments.
        for name in ["echo", "sh"] {
            write_exec(
                &root.join("ns").join(name),
                &format!("#!/bin/bash\necho \"REDIRECTED:{name}:$*\"\n"),
            );
        }
        // An entry that is not executable maps to nothing.
        std::fs::write(root.join("ns/date"), "not a program").unwrap();
        Fixture { root }
    }

    fn command(&self, program: &Path) -> Command {
        let mut c = Command::new(program);
        c.env("DYLD_INSERT_LIBRARIES", self.root.join("lib.dylib"))
            .env("DEVCROFT_NS", self.root.join("ns"));
        c
    }

    /// `env <args>`: devcroft's applet execs `args[0]`.
    fn env(&self, args: &[&str]) -> Output {
        self.command(&self.root.join("rt/env"))
            .args(args)
            .output()
            .unwrap()
    }

    fn script(&self, name: &str, text: &[u8]) -> String {
        let path = self.root.join(name);
        std::fs::write(&path, text).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn a_base_name_path_with_an_entry_runs_the_entry() {
    let f = Fixture::new("map");
    for path in ["/bin/echo", "/usr/bin/echo"] {
        let o = f.env(&[path, "a", "b"]);
        assert_eq!(stdout(&o), "REDIRECTED:echo:a b\n", "{path}: {o:?}");
    }
    let o = f.env(&["/bin/sh", "-c", "true"]);
    assert_eq!(stdout(&o), "REDIRECTED:sh:-c true\n", "{o:?}");
}

#[test]
fn everything_else_passes_through_unchanged() {
    let f = Fixture::new("pass");
    // A base name with no entry, and one whose entry is not executable: the
    // host's file runs (outside a sandbox; inside, Seatbelt refuses it).
    let o = f.env(&["/bin/cat", "/dev/null"]);
    assert!(o.status.success() && stdout(&o).is_empty(), "{o:?}");
    let o = f.env(&["/bin/date", "+%Y"]);
    assert!(stdout(&o).trim().parse::<u32>().is_ok(), "{o:?}");
    // Not a base-name path: a deeper path, a relative one.
    let o = f.env(&["/usr/bin/true"]);
    assert!(
        o.status.success() && !stdout(&o).contains("REDIRECTED"),
        "{o:?}"
    );
    // Without the namespace variable, nothing maps.
    let o = Command::new(f.root.join("rt/env"))
        .env("DYLD_INSERT_LIBRARIES", f.root.join("lib.dylib"))
        .env_remove("DEVCROFT_NS")
        .args(["/bin/echo", "host"])
        .output()
        .unwrap();
    assert_eq!(stdout(&o), "host\n", "{o:?}");
}

#[test]
fn a_script_whose_interpreter_maps_starts_with_the_entry_kernel_style() {
    let f = Fixture::new("shebang");
    let cases: &[(&str, &[u8], &str)] = &[
        ("with-arg", b"#!/bin/sh -eu\necho no\n", "-eu"),
        ("no-arg", b"#!/bin/sh\necho no\n", ""),
        ("spaces", b"#!  /bin/sh   -x  \necho no\n", "-x"),
        ("crlf", b"#!/bin/sh -e\r\necho no\r\n", "-e"),
        ("no-newline", b"#!/bin/sh -e", "-e"),
        // The kernel's convention: everything after the interpreter is one
        // argument.
        ("two-words", b"#!/bin/sh -e -u\n", "-e -u"),
    ];
    for (name, text, arg) in cases {
        let script = f.script(name, text);
        let o = f.env(&[&script, "x", "y"]);
        let want = if arg.is_empty() {
            format!("REDIRECTED:sh:{script} x y\n")
        } else {
            format!("REDIRECTED:sh:{arg} {script} x y\n")
        };
        assert_eq!(stdout(&o), want, "{name}: {o:?}");
    }
    // `#!/usr/bin/env bash`: this namespace has no `env` entry, so the
    // library leaves the script to the kernel and the host's env runs it.
    let script = f.script("env-form", b"#!/usr/bin/env bash\necho NOT-REDIRECTED\n");
    let o = f.env(&[&script]);
    assert_eq!(stdout(&o), "NOT-REDIRECTED\n", "{o:?}");
}

#[test]
fn an_unparseable_or_unmapped_script_is_left_to_the_kernel() {
    let f = Fixture::new("unparsed");
    let script = f.script("bash", b"#!/bin/bash\necho NOT-REDIRECTED\n");
    let o = f.env(&[&script]);
    assert_eq!(stdout(&o), "NOT-REDIRECTED\n", "{o:?}");
    for (name, text) in [
        ("empty", &b"#!\n"[..]),
        ("blank", b"#!   \n"),
        ("long", &[b"#!/bin/sh ".as_slice(), &[b'x'; 600]].concat()),
    ] {
        let script = f.script(name, text);
        let o = f.env(&[&script]);
        assert!(!stdout(&o).contains("REDIRECTED"), "{name}: {o:?}");
    }
}

#[test]
fn errors_pass_through_with_their_own_errno() {
    let f = Fixture::new("errors");
    // ENOENT: a base name with no entry and no host file.
    let o = f.env(&["/bin/devcroft-no-such-tool"]);
    assert_eq!(o.status.code(), Some(127), "{o:?}");
    // EACCES: a file that is not executable, not a base-name path.
    let plain = f.root.join("plain");
    std::fs::write(&plain, "x").unwrap();
    let o = f.env(&[plain.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(126), "{o:?}");
    // E2BIG through a mapped path is in `posix_spawn_is_redirected_too`:
    // an argument too large for the kernel is too large for this test's own
    // spawn of `env` as well.
}

#[test]
fn posix_spawn_is_redirected_too() {
    let f = Fixture::new("spawn");
    let src = f.root.join("spawn.c");
    std::fs::write(
        &src,
        r#"#include <spawn.h>
#include <stdio.h>
#include <sys/wait.h>
extern char **environ;
#include <stdlib.h>
#include <string.h>
int main(int argc, char **argv) {
  pid_t pid; int st = 0;
  if (argc > 2 && !strcmp(argv[1], "--big")) {
    /* An argument larger than ARG_MAX, built here so only the inner spawn
       carries it. */
    size_t n = 4u << 20;
    char *big = malloc(n + 1); memset(big, 'x', n); big[n] = 0;
    char *nv[] = {argv[2], big, NULL};
    int err = posix_spawn(&pid, argv[2], NULL, NULL, nv, environ);
    printf("spawn err %d\n", err);
    return 0;
  }
  int err = posix_spawn(&pid, argv[1], NULL, NULL, argv + 1, environ);
  if (err) { printf("spawn err %d\n", err); return 1; }
  waitpid(pid, &st, 0);
  return WIFEXITED(st) ? WEXITSTATUS(st) : 1;
}
"#,
    )
    .unwrap();
    let bin = f.root.join("spawn");
    let built = Command::new("cc").arg("-o").arg(&bin).arg(&src).status();
    if !built.is_ok_and(|s| s.success()) {
        eprintln!("skipping: no C compiler to build the posix_spawn probe");
        return;
    }
    let o = f.command(&bin).args(["/bin/echo", "z"]).output().unwrap();
    assert_eq!(stdout(&o), "REDIRECTED:echo:z\n", "{o:?}");
    let script = f.script("s", b"#!/bin/sh\n");
    let o = f.command(&bin).arg(&script).output().unwrap();
    assert_eq!(stdout(&o), format!("REDIRECTED:sh:{script}\n"), "{o:?}");
    // E2BIG from a mapped path keeps its own error code.
    let o = f
        .command(&bin)
        .args(["--big", "/bin/echo"])
        .output()
        .unwrap();
    assert_eq!(stdout(&o), format!("spawn err {}\n", libc::E2BIG), "{o:?}");
}
