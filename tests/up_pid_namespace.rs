//! `add-up-pid-namespace`: where the host can mount a fresh procfs in a user
//! and PID namespace, an `up` sandbox runs under a PID 1 of its own, sees
//! only its own processes in `/proc`, is granted `/proc` for that, and `down`
//! ends everything in it.
//!
//! Driven through the provider seam, as `provisioning_confinement.rs` is: a
//! row whose closure is a store shell.

#![cfg(feature = "test-support")]

use devcroft::config::parse;
use devcroft::lifecycle::{StatePaths, UpOptions, UpOutcome, down};
use devcroft::provider::{ProviderEntry, ProviderError, Resolution, ServiceSupport, Tier};
use devcroft::test_support::up_with_provider;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn store_path_with_a_shell() -> Option<PathBuf> {
    std::fs::read_dir("/nix/store")
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.join("bin/sh").is_file() && p.to_string_lossy().contains("bash"))
}

struct Row {
    store: PathBuf,
}

impl ProviderEntry for Row {
    fn resolve(&self, _: &Path) -> Result<Resolution, ProviderError> {
        Ok(Resolution {
            env: BTreeMap::from([(
                "PATH".to_owned(),
                self.store.join("bin").display().to_string(),
            )]),
            unset: Vec::new(),
            read_only_grants: vec!["/nix/store".to_owned()],
            activation_script: None,
            services: ServiceSupport::Unsupported,
            ran_activation_hook: false,
        })
    }
    fn fingerprint(&self, _: &Path) -> Result<String, ProviderError> {
        Ok("up-pid-namespace-row".to_owned())
    }
    fn static_name(&self) -> &'static str {
        "nix"
    }
    fn tier(&self) -> Tier {
        Tier::Closure
    }
}

/// Host pids whose command line contains `marker`.
fn host_pids_with(marker: &str) -> Vec<u32> {
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            std::fs::read(format!("/proc/{pid}/cmdline"))
                .is_ok_and(|c| String::from_utf8_lossy(&c).contains(marker))
        })
        .collect()
}

#[test]
fn a_sandbox_sees_only_its_own_processes_and_down_ends_them() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    let exe = Path::new(env!("CARGO_BIN_EXE_devcroft"));
    if !matches!(devcroft::fleet::mount::probe_pid_namespace(exe), Ok(true)) {
        eprintln!("skipping: this host cannot mount a fresh /proc in a user+PID namespace");
        return;
    }
    let Some(store) = store_path_with_a_shell() else {
        eprintln!("skipping: no /nix/store path with a shell on this host");
        return;
    };
    // SAFETY: this process runs a single test.
    unsafe { std::env::set_var("DEVCROFT_KEEPER_EXE", exe) };

    let pid = std::process::id();
    let project = PathBuf::from(format!("/tmp/dcuppidns{pid}"));
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();
    let project = project.canonicalize().unwrap();
    let name = format!("e2euppidns{pid}");
    let (manifest, _) = parse(&format!(
        "[sandbox]\nname = {name:?}\n\n[network]\ndefault = \"deny\"\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_file(&paths.lifecycle_lock);

    // A process outside the sandbox, by this test's own pid: it must not be
    // listed inside.
    let outside = std::process::id();
    let outcome = up_with_provider(&manifest, &project, &UpOptions::default(), &Row { store });
    let exec = |script: &str| {
        Command::new(exe)
            .current_dir(&project)
            .args(["exec", &name, "--", "sh", "-c", script])
            .output()
            .unwrap()
    };
    let listing = exec(
        "for p in /proc/[0-9]*; do read -r c < \"$p/comm\" && echo \"${p#/proc/} $c\"; done; \
         if read -r l < /proc/self/status; then echo SELF_READ; fi",
    );
    // Left running in the sandbox, detached from the session, so only the
    // namespace ending can end it.
    let marker = format!("dcpidns-marker-{pid}");
    let _ = Command::new(exe)
        .current_dir(&project)
        .args([
            "exec",
            &name,
            "--",
            "sh",
            "-c",
            // Its output away from the session's, or `exec` would wait for
            // it to close them.
            "(exec -a \"$0\" sh -c 'while :; do read -t 1 x < /dev/zero; done' \
             < /dev/null > /dev/null 2>&1 &)",
            &marker,
        ])
        .output();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut before = host_pids_with(&marker);
    while before.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        before = host_pids_with(&marker);
    }
    let meta = devcroft::lifecycle::read_meta(&paths.meta).ok().flatten();
    let artifact = std::fs::read_to_string(devcroft::services::policy_artifact(&project, &name))
        .unwrap_or_default();
    let log = std::fs::read_to_string(&paths.log).unwrap_or_default();
    let downed = down(&name);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut after = host_pids_with(&marker);
    while !after.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        after = host_pids_with(&marker);
    }
    for pid in &after {
        // SAFETY: cleaning up what the test started, if `down` did not.
        unsafe { libc::kill(*pid as i32, libc::SIGKILL) };
    }
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_file(&paths.lifecycle_lock);
    let _ = std::fs::remove_dir_all(&project);

    assert_eq!(
        outcome.unwrap_or_else(|e| panic!("up: {e}\nlog:\n{log}")),
        UpOutcome::Started
    );
    let out = String::from_utf8_lossy(&listing.stdout);
    assert!(
        listing.status.success(),
        "{out}{}",
        String::from_utf8_lossy(&listing.stderr)
    );
    // 3.1: its own processes are listed (so the check below is not vacuous),
    // and it reads its own procfs entries.
    assert!(out.lines().any(|l| l.ends_with(" sh")), "{out}");
    assert!(out.contains("SELF_READ"), "{out}");
    // PID 1 is the init helper, the keeper is under it.
    assert!(out.lines().any(|l| l == "1 devcroft"), "{out}");
    // 3.2: nothing else. The host is running cargo and this test at least.
    for line in out.lines().filter(|l| *l != "SELF_READ") {
        let comm = line.split_once(' ').map_or("", |(_, c)| c);
        assert!(
            ["devcroft", "sh"].contains(&comm),
            "a process outside the sandbox is visible: {line:?} in {out}"
        );
        assert_ne!(line.split(' ').next(), Some(outside.to_string().as_str()));
    }
    // 2.4: granted, and recorded for `--render`.
    assert!(meta.is_some_and(|m| m.own_procfs));
    let policy: devcroft::policy::CompiledPolicy = serde_json::from_str(&artifact).unwrap();
    assert!(
        policy
            .filesystem_read
            .iter()
            .any(|g| g.value == "/proc" && g.origin == devcroft::policy::Origin::PidNamespace),
        "{:?}",
        policy.filesystem_read
    );
    // 3.3: the detached process existed, and `down` ended it.
    downed.unwrap_or_else(|e| panic!("down: {e}"));
    assert!(
        !before.is_empty(),
        "the detached process never started\nlog:\n{log}"
    );
    assert!(after.is_empty(), "still running after down: {after:?}");
}

/// A session in a sandbox with its own user namespace is the user, not
/// root. The setup maps `0 → uid`, since only uid 0 keeps its capabilities
/// across `execve` there, and every session used to inherit that and report
/// uid 0. `mariadbd` refuses to run as root, and so does postgres's
/// `initdb` (`fleet::mount::return_to_host_identity`). Checked from both
/// sides: the uid a session reports, and the owner a file it creates has on
/// the host.
#[test]
fn a_session_is_the_users_own_uid_not_root() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    let exe = Path::new(env!("CARGO_BIN_EXE_devcroft"));
    if !matches!(devcroft::fleet::mount::probe_pid_namespace(exe), Ok(true)) {
        eprintln!("skipping: this host cannot mount a fresh /proc in a user+PID namespace");
        return;
    }
    let Some(store) = store_path_with_a_shell() else {
        eprintln!("skipping: no /nix/store path with a shell on this host");
        return;
    };
    // SAFETY: this process runs a single test.
    unsafe { std::env::set_var("DEVCROFT_KEEPER_EXE", exe) };
    // SAFETY: plain getters.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    if uid == 0 {
        eprintln!("skipping: run as root, whose own identity is 0");
        return;
    }

    let pid = std::process::id();
    let project = PathBuf::from(format!("/tmp/dcupuid{pid}"));
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();
    let project = project.canonicalize().unwrap();
    let name = format!("e2eupuid{pid}");
    let (manifest, _) = parse(&format!(
        "[sandbox]\nname = {name:?}\n\n[network]\ndefault = \"deny\"\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_file(&paths.lifecycle_lock);

    let outcome = up_with_provider(&manifest, &project, &UpOptions::default(), &Row { store });
    let session = Command::new(exe)
        .current_dir(&project)
        .args([
            "exec",
            &name,
            "--",
            "sh",
            "-c",
            "echo \"$UID $EUID\"; : > made-inside",
        ])
        .output()
        .unwrap();
    let made = std::fs::metadata(project.join("made-inside"));
    let log = std::fs::read_to_string(&paths.log).unwrap_or_default();
    let downed = down(&name);
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_file(&paths.lifecycle_lock);
    let _ = std::fs::remove_dir_all(&project);

    assert_eq!(
        outcome.unwrap_or_else(|e| panic!("up: {e}\nlog:\n{log}")),
        UpOutcome::Started
    );
    downed.unwrap_or_else(|e| panic!("down: {e}"));
    let out = String::from_utf8_lossy(&session.stdout);
    assert!(
        session.status.success(),
        "{out}{}",
        String::from_utf8_lossy(&session.stderr)
    );
    assert_eq!(
        out.trim(),
        format!("{uid} {uid}"),
        "a session must report the user's own uid, not 0"
    );
    use std::os::unix::fs::MetadataExt;
    let made = made.expect("the session's file must exist on the host");
    assert_eq!((made.uid(), made.gid()), (uid, gid));
}

/// The view names its user (`fleet::mount::Identity`): an `/etc/passwd`
/// with the user's own uid, the sandbox's home and devcroft's shell; an
/// `/etc/hosts` resolving `localhost`; `/bin/sh` runnable. Each one is a
/// rule with origin `identity` in the policy artifact. A path the manifest
/// grants itself is the host's, not written: here `/etc/hosts`.
#[test]
fn the_view_names_its_user_and_a_granted_path_stays_the_hosts() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    let exe = Path::new(env!("CARGO_BIN_EXE_devcroft"));
    if !matches!(devcroft::fleet::mount::probe_pid_namespace(exe), Ok(true)) {
        eprintln!("skipping: this host cannot mount a fresh /proc in a user+PID namespace");
        return;
    }
    let Some(store) = store_path_with_a_shell() else {
        eprintln!("skipping: no /nix/store path with a shell on this host");
        return;
    };
    // SAFETY: this process runs a single test.
    unsafe { std::env::set_var("DEVCROFT_KEEPER_EXE", exe) };
    // SAFETY: a plain getter.
    let uid = unsafe { libc::getuid() };

    let pid = std::process::id();
    let project = PathBuf::from(format!("/tmp/dcupident{pid}"));
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();
    let project = project.canonicalize().unwrap();
    let name = format!("e2eupident{pid}");
    let (manifest, _) = parse(&format!(
        "[sandbox]\nname = {name:?}\n\n[network]\ndefault = \"deny\"\n\n\
         [filesystem]\nread = [\"/etc/hosts\"]\n"
    ))
    .unwrap();
    let paths = StatePaths::new(&name).unwrap();
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_file(&paths.lifecycle_lock);

    let outcome = up_with_provider(&manifest, &project, &UpOptions::default(), &Row { store });
    let session = Command::new(exe)
        .current_dir(&project)
        .args([
            "exec",
            &name,
            "--",
            "sh",
            "-c",
            "while read -r l; do echo \"passwd $l\"; done < /etc/passwd; \
             while read -r l; do echo \"hosts $l\"; done < /etc/hosts; \
             /bin/sh -c 'echo bin-sh-ran'",
        ])
        .output()
        .unwrap();
    let artifact = std::fs::read_to_string(devcroft::services::policy_artifact(&project, &name))
        .unwrap_or_default();
    let log = std::fs::read_to_string(&paths.log).unwrap_or_default();
    let downed = down(&name);
    let _ = std::fs::remove_dir_all(&paths.root);
    let _ = std::fs::remove_file(&paths.lifecycle_lock);
    let _ = std::fs::remove_dir_all(&project);

    assert_eq!(
        outcome.unwrap_or_else(|e| panic!("up: {e}\nlog:\n{log}")),
        UpOutcome::Started
    );
    downed.unwrap_or_else(|e| panic!("down: {e}"));
    let out = String::from_utf8_lossy(&session.stdout);
    assert!(
        session.status.success(),
        "{out}{}",
        String::from_utf8_lossy(&session.stderr)
    );
    let home = devcroft::services::artifact_dir(&project, &name).join("home");
    let user_line = out
        .lines()
        .find_map(|l| l.strip_prefix("passwd "))
        .expect("an /etc/passwd line");
    let fields: Vec<&str> = user_line.split(':').collect();
    assert_eq!(fields[2], uid.to_string(), "{user_line}");
    assert_eq!(fields[5], home.display().to_string(), "{user_line}");
    assert!(fields[6].starts_with("/nix/store/"), "{user_line}");
    assert!(out.contains("bin-sh-ran"), "{out}");
    // Granted by the manifest, so the host's file, unchanged.
    let host_hosts: Vec<String> = std::fs::read_to_string("/etc/hosts")
        .unwrap()
        .lines()
        .map(|l| format!("hosts {l}"))
        .collect();
    let seen: Vec<&str> = out.lines().filter(|l| l.starts_with("hosts ")).collect();
    assert_eq!(seen, host_hosts, "{out}");

    let policy: devcroft::policy::CompiledPolicy = serde_json::from_str(&artifact).unwrap();
    let identity: Vec<&str> = policy
        .filesystem_read
        .iter()
        .filter(|g| g.origin == devcroft::policy::Origin::Identity)
        .map(|g| g.value.as_str())
        .collect();
    assert!(identity.contains(&"/etc/passwd"), "{identity:?}");
    assert!(identity.contains(&"/bin/sh"), "{identity:?}");
    assert!(
        !identity.contains(&"/etc/hosts"),
        "a path the manifest granted is not written: {identity:?}"
    );
}
