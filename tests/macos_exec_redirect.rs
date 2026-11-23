//! The exec redirect in a real sandbox (`add-macos-exec-redirect` 2, 3).
#![cfg(target_os = "macos")]

use std::path::Path;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_devcroft");

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

#[test]
fn absolute_paths_resolve_as_names_do_and_the_keeper_stays_clean() {
    if !devcroft::policy::backend_supported()
        || !devcroft::provider::host_can_build_nix_closures()
        || Command::new("nix").arg("--version").output().is_err()
    {
        eprintln!("skipping: no usable Seatbelt or nix here");
        return;
    }
    let name = format!("redir{}", std::process::id());
    let root = std::env::temp_dir().join(format!("dc-{name}"));
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/nix-flake-sample");
    for f in ["flake.nix", "flake.lock"] {
        std::fs::copy(sample.join(f), root.join(f)).unwrap();
    }
    std::fs::write(
        root.join("devcroft.toml"),
        format!(
            "[sandbox]\nname = \"{name}\"\n[env]\nprovider = \"nix\"\n\
             [filesystem]\nallow = [\".\"]\n[hooks]\npost_create = './s.sh > hook.out'\n"
        ),
    )
    .unwrap();
    let exe = |f: &str, t: &str| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(root.join(f), t).unwrap();
        std::fs::set_permissions(root.join(f), std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    exe("s.sh", "#!/bin/sh\necho shebang-sh\n");
    exe("e.sh", "#!/usr/bin/env sh\necho shebang-env\n");
    std::fs::write(
        root.join("p.c"),
        "#include <stdio.h>\n#include <stdlib.h>\nint main(void){char b[64]=\"\";\
         system(\"echo via-system\");FILE*p=popen(\"echo via-popen\",\"r\");\
         if(p){if(!fgets(b,64,p))b[0]=0;pclose(p);}printf(\"%s\",b);return 0;}\n",
    )
    .unwrap();
    let built = Command::new("cc")
        .current_dir(&root)
        .args(["-o", "p", "p.c"])
        .status();
    let have_c = built.is_ok_and(|s| s.success());

    let up = dc(&root, &["up"]);
    let up_text = text(&up);
    let sh = |script: &str| text(&dc(&root, &["exec", "--", "sh", "-c", script]));
    let mut failures = Vec::new();
    let mut want = |what: &str, got: String, ok: &dyn Fn(&str) -> bool| {
        println!("{what}: {got:?}");
        if !ok(&got) {
            failures.push(format!("{what}: {got}"));
        }
    };
    if up.status.success() {
        want("shebang sh", sh("./s.sh"), &|g| g == "shebang-sh\n");
        want("shebang env", sh("./e.sh"), &|g| g == "shebang-env\n");
        let hook = std::fs::read_to_string(root.join("hook.out")).unwrap_or_default();
        want("hook", hook, &|g| g == "shebang-sh\n");
        if have_c {
            want("system and popen", sh("./p"), &|g| {
                g.contains("via-system") && g.contains("via-popen")
            });
        } else {
            eprintln!("skipping system()/popen(): no C compiler");
        }
        // Rule B: GNU from the closure by absolute path.
        want("/bin/ls is GNU", sh("/bin/ls --version"), &|g| {
            g.contains("GNU coreutils")
        });
        // devcroft's own `env` keeps the redirect over a cleared environment;
        // the closure's GNU `env` does not (published limit), and what
        // follows is a refusal, never the host's shell.
        let own = root.join(".devcroft").join(&name).join("runtime/env");
        want(
            "runtime env -i",
            sh(&format!("{} -i /bin/sh -c 'echo still'", own.display())),
            &|g| g == "still\n",
        );
        want("GNU env -i", sh("env -i /bin/sh -c 'echo host'"), &|g| {
            g.contains("not permitted")
        });
        for p in ["/usr/bin/make", "/usr/bin/python3"] {
            want(p, sh(&format!("{p} --version; echo rc=$?")), &|g| {
                g.contains("rc=126") && g.to_lowercase().contains("not permitted")
            });
        }
        want(
            "no keeper plumbing in a session",
            sh("env | grep DEVCROFT_REDIRECT; true"),
            &|g| g.is_empty(),
        );
        let render = text(&dc(&root, &["policy", "--render"]));
        want("render", render, &|g| {
            ["/ns ", "/redirect "].iter().all(|d| {
                g.lines()
                    .any(|l| l.contains(d) && l.contains("runtime:gen-1"))
            })
        });
        // The load-bearing one: dyld runs a library before `main`, so before
        // the keeper's self-restriction, and the sandbox can write the file.
        let pids = Command::new("pgrep").args(["-f", BIN]).output().unwrap();
        let mut keepers = 0;
        for pid in String::from_utf8_lossy(&pids.stdout).split_whitespace() {
            let cwd = Command::new("lsof").args(["-p", pid]).output().unwrap();
            let maps = String::from_utf8_lossy(&cwd.stdout).into_owned();
            if maps.contains(root.to_str().unwrap()) {
                keepers += 1;
                want("keeper maps", maps, &|g| {
                    !g.contains("libdevcroft-redirect")
                });
            }
        }
        want("a keeper was inspected", keepers.to_string(), &|g| g != "0");
    }
    let _ = dc(&root, &["rm", "--yes"]);
    let _ = std::fs::remove_dir_all(&root);
    assert!(up.status.success(), "{up_text}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
