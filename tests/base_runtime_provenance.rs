//! `add-runtime-provenance` 3.1: the base-runtime record `up` writes
//! matches what actually runs inside the sandbox, for every base name in
//! both lookup forms, on every provider row this run selects; and on a row
//! whose grants are all store paths, nothing resolves to the host.
//!
//! The record is computed at `up` without spawning anything
//! (`runtime::provenance`), so this is the test that keeps it honest: a
//! report that says `absent` for something that runs, or names a different
//! file than the one the sandbox finds, fails here naming the base name
//! and the lookup form.

mod common;

use common::for_each_row;
use devcroft::keeper::protocol::{self, Frame, SpawnRequest};
use devcroft::lifecycle::{StatePaths, UpOptions};
use devcroft::runtime::provenance::BY_NAME;
use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::path::Path;

/// Runs `script` through the sandbox's own shell and returns its stdout.
fn run_in_sandbox(paths: &StatePaths, shell: &str, cwd: &Path, script: &str) -> String {
    let mut client = UnixStream::connect(&paths.socket).unwrap();
    protocol::write_frame(
        &mut client,
        &Frame::Spawn(SpawnRequest {
            cmd: shell.to_owned(),
            args: vec!["-c".to_owned(), script.to_owned()],
            cwd: cwd.to_string_lossy().into_owned(),
            env: BTreeMap::new(),
            pty: None,
        }),
    )
    .unwrap();
    let mut out = Vec::new();
    loop {
        match protocol::read_frame(&mut client).unwrap() {
            Frame::SpawnOk { .. } | Frame::Stderr(_) => {}
            Frame::Stdout(bytes) => out.extend(bytes),
            Frame::Exit(_) => break,
            Frame::SpawnErr { message } => panic!("spawn failed: {message}"),
            other => panic!("unexpected frame: {other:?}"),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One probe line per entry: `<name>|<lookup>|<found>|<same>`.
///
/// By name, `<found>` is the first executable on the sandbox's `PATH` (a
/// search, not `command -v`, which reports builtins such as `echo` instead
/// of a file). By absolute path, it is the exit status of running it:
/// 126/127 is "did not execute".
///
/// `<same>` is whether that file is the one recorded, decided *inside* the
/// sandbox with `-ef` (device and inode, a shell builtin). Comparing
/// canonical paths on the host instead is blind to the view: it found the
/// host's `/usr/bin/sh` → dash where the sandbox runs the closure's bash
/// through the view's `/bin/sh`, and so agreed with a record that said
/// `host:` (measured, a flox sandbox).
fn probe_script(record: &devcroft::runtime::provenance::Record) -> String {
    let mut s = String::from("exec </dev/null\nnl='\n'\n");
    for e in &record.entries {
        let recorded = e.provenance.path().unwrap_or("/nonexistent");
        if e.lookup == BY_NAME {
            s.push_str(&format!(
                "p=; oldifs=$IFS; IFS=:; for d in $PATH; do \
                 if [ -z \"$p\" ] && [ -f \"$d/{n}\" ] && [ -x \"$d/{n}\" ]; then p=\"$d/{n}\"; fi; \
                 done; IFS=$oldifs; \
                 if [ -n \"$p\" ] && [ \"$p\" -ef '{recorded}' ]; then same=1; else same=0; fi; \
                 echo \"{n}|{BY_NAME}|$p|$same\"\n",
                n = e.name
            ));
        } else {
            let args = match e.name.as_str() {
                "sh" => "-c true",
                // BSD `nohup --version` runs `--version`, and reports 127.
                "nohup" => "true",
                _ => "--version",
            };
            // BSD `yes --version` prints forever (the swift row, where the
            // host's runs); with no stdout it fails its first write.
            let out = if e.name == "yes" { "&-" } else { "/dev/null" };
            s.push_str(&format!(
                "v=$(DYLD_PRINT_LIBRARIES=1 {abs} {args} 2>&1 >{out}); rc=$?; \
                 if [ '{abs}' -ef '{recorded}' ]; then same=1; else same=${{v%%\"$nl\"*}}; fi; \
                 echo \"{n}|{abs}|$rc|$same\"\n",
                abs = e.lookup,
                n = e.name
            ));
        }
    }
    s
}

/// On macOS an exec of an absolute path is redirected, so the file *at* that
/// path (`-ef`) is not the one that ran. dyld's first `DYLD_PRINT_LIBRARIES`
/// line names the program it actually loaded: `dyld[pid]: <uuid> /path`.
fn ran_recorded(first_line: &str, recorded: Option<&str>) -> bool {
    let ran = first_line
        .strip_prefix("dyld[")
        .and_then(|l| l.split_once("> "))
        .map(|(_, path)| Path::new(path));
    match (ran, recorded.map(Path::new)) {
        (Some(ran), Some(recorded)) => match (ran.canonicalize(), recorded.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        },
        _ => false,
    }
}

#[test]
fn the_base_runtime_record_matches_what_runs_inside() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    // SAFETY: this process runs a single test.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }

    for_each_row("baseruntime", |fx| {
        let name = fx.sandbox_name().to_string();
        let root = fx.project_root().to_path_buf();
        fx.bring_up(&UpOptions::default()).unwrap();
        let paths = StatePaths::new(&name).unwrap();
        let meta = devcroft::lifecycle::read_meta(&paths.meta)
            .unwrap()
            .unwrap();
        let record = meta
            .base_runtime
            .clone()
            .expect("`up` records the base runtime");
        let shell = meta.shell.clone().expect("`up` records the shell");

        let output = run_in_sandbox(&paths, &shell, &root, &probe_script(&record));
        let observed: BTreeMap<(String, String), (String, String)> = output
            .lines()
            .filter_map(|l| {
                let mut parts = l.splitn(4, '|');
                Some((
                    (parts.next()?.to_owned(), parts.next()?.to_owned()),
                    (parts.next()?.to_owned(), parts.next()?.to_owned()),
                ))
            })
            .collect();

        let mut mismatches = Vec::new();
        for e in &record.entries {
            let Some((seen, same)) = observed.get(&(e.name.clone(), e.lookup.clone())) else {
                mismatches.push(format!("{} {}: no probe output", e.name, e.lookup));
                continue;
            };
            let same = same == "1" || ran_recorded(same, e.provenance.path());
            let recorded_runs = e.provenance.path().is_some();
            if e.lookup == BY_NAME {
                let found = !seen.is_empty();
                if found != recorded_runs || (found && !same) {
                    mismatches.push(format!(
                        "{} by name: recorded {}, the sandbox found {seen:?} (same file: {same})",
                        e.name, e.provenance
                    ));
                }
            } else {
                let ran = !matches!(seen.as_str(), "126" | "127");
                if ran != recorded_runs || (ran && !same) {
                    mismatches.push(format!(
                        "{}: recorded {}, but exit status inside was {seen} (same file: {same})",
                        e.lookup, e.provenance
                    ));
                }
            }
        }
        eprintln!(
            "base runtime [{}]: {}; host: [{}]; absent by name: [{}]; resolving by absolute path: [{}]",
            fx.sandbox_name(),
            record
                .counts()
                .iter()
                .map(|(k, n)| format!("{k} {n}"))
                .collect::<Vec<_>>()
                .join(", "),
            record
                .host_entries()
                .map(|e| format!("{} {}", e.name, e.lookup))
                .collect::<Vec<_>>()
                .join(", "),
            record
                .entries
                .iter()
                .filter(|e| e.lookup == BY_NAME && e.provenance.path().is_none())
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            record
                .entries
                .iter()
                .filter(|e| e.lookup != BY_NAME && e.provenance.path().is_some())
                .map(|e| e.lookup.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        );
        // 3.3: a shebang script runs exactly when its interpreter path is
        // recorded as running — the kernel resolves the shebang, so these
        // are the cases `system()`, every shell script and every
        // `#!/usr/bin/env <interpreter>` script depend on. The `env` form
        // also needs `sh` by name, which `env` looks up on `PATH`.
        let runs = |lookup: &str| {
            record
                .entries
                .iter()
                .find(|e| e.lookup == lookup)
                .unwrap_or_else(|| panic!("{lookup} is a base lookup"))
                .provenance
                .path()
                .is_some()
        };
        let sh_by_name = record
            .entries
            .iter()
            .any(|e| e.name == "sh" && e.lookup == BY_NAME && e.provenance.path().is_some());
        for (file, line, expected) in [
            ("dc-shebang-sh.sh", "#!/bin/sh", runs("/bin/sh")),
            (
                "dc-shebang-env.sh",
                "#!/usr/bin/env sh",
                runs("/usr/bin/env") && sh_by_name,
            ),
        ] {
            let script = root.join(file);
            std::fs::write(&script, format!("{line}\nexit 0\n")).unwrap();
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let status = run_in_sandbox(
                &paths,
                &shell,
                &root,
                &format!("{} </dev/null >/dev/null 2>&1; echo $?", script.display()),
            );
            if (status.trim() == "0") != expected {
                mismatches.push(format!(
                    "`{line}` script: the record says it {}, but it exited {}",
                    if expected { "runs" } else { "cannot run" },
                    status.trim()
                ));
            }
        }

        assert!(
            mismatches.is_empty(),
            "the base-runtime record does not match the sandbox ({} mismatch(es)):\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );

        // Rule B (`add-own-userland` 4.5): `/bin/<name>` and
        // `<name>` are one file, or neither runs. Both halves were just
        // checked against the sandbox, so the record can be compared with
        // itself. Not in native mode, nor on macOS with a host shell, where
        // the host's files are granted and the redirect does not apply.
        if !record.entries.iter().any(|e| {
            let kind = e.provenance.kind();
            kind == "native" || (kind == "host" && cfg!(target_os = "macos"))
        }) {
            let file = |p: &devcroft::runtime::provenance::Provenance| {
                p.path().map(|p| {
                    let p = Path::new(p);
                    // A runtime link's name selects the utility, so it is
                    // compared as a path: all of them reach one binary.
                    if p.parent().is_some_and(|d| d.ends_with("runtime")) {
                        p.to_path_buf()
                    } else {
                        p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
                    }
                })
            };
            let mut differ = Vec::new();
            for e in record.entries.iter().filter(|e| e.lookup != BY_NAME) {
                let by_name = record
                    .entries
                    .iter()
                    .find(|n| n.name == e.name && n.lookup == BY_NAME)
                    .unwrap();
                if file(&e.provenance) != file(&by_name.provenance) {
                    differ.push(format!(
                        "{} is {}, but by name {}",
                        e.lookup, e.provenance, by_name.provenance
                    ));
                }
            }
            assert!(differ.is_empty(), "{}", differ.join("\n"));
        }

        // The invariant, where it already holds: a row whose every
        // provider grant is a store path has no business resolving shell
        // or userland to the host.
        let store_only = !meta.read_only_grants.is_empty()
            && meta
                .read_only_grants
                .iter()
                .all(|g| g.starts_with("/nix/store"));
        if store_only {
            let hosts: Vec<String> = record
                .host_entries()
                .map(|e| format!("{} {} → {}", e.name, e.lookup, e.provenance))
                .collect();
            assert!(
                hosts.is_empty(),
                "a store-backed row resolved shell or userland to the host:\n{}",
                hosts.join("\n")
            );
            // An `env` the closure supplies by name is the one the view puts
            // at `/usr/bin/env`. Every flox sandbox used to miss it, because
            // flox's `PATH` reaches the store through a symlink and the
            // view's guard compared uncanonicalized paths.
            let env_by_name = record
                .entries
                .iter()
                .any(|e| e.name == "env" && e.lookup == BY_NAME && e.provenance.path().is_some());
            assert!(
                !env_by_name || runs("/usr/bin/env"),
                "the closure supplies `env` by name, but the view has no /usr/bin/env, \
                 so every `#!/usr/bin/env` script fails"
            );
        }
        let _ = devcroft::lifecycle::down(&name);
    });
}
