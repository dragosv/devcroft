//! `sandbox-provisioning` 1.3: `why` answers for the provisioning context
//! too. The provider's activation script runs with the runtime filesystem
//! and only `[provisioning.network] allow`, nothing inherited from
//! `[network] allow`, so a host can be allowed in one context and refused
//! in the other. Each answer names the other when they differ.
//!
//! The real binary, a project and an empty home: no sandbox, so the
//! manifest is the whole policy, and no provider is needed.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

struct Fixture {
    base: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let base = std::env::temp_dir().join(format!("devcroft-whyprov-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("project")).unwrap();
        std::fs::create_dir_all(base.join("home")).unwrap();
        std::fs::write(
            base.join("project/devcroft.toml"),
            "[sandbox]\nname = \"whyprov\"\n\n[env]\nprovider = \"nix\"\n\n\
             [network]\ndefault = \"deny\"\nallow = [\"runtime.example\"]\n\n\
             [provisioning.network]\nallow = [\"registry.example\"]\n",
        )
        .unwrap();
        Fixture { base }
    }

    fn why(&self, args: &[&str]) -> (Option<i32>, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_devcroft"))
            .current_dir(self.base.join("project"))
            .env("HOME", self.base.join("home"))
            .arg("why")
            .args(args)
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.code(), text)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

#[test]
fn why_answers_for_the_provisioning_context_and_names_the_other() {
    let f = Fixture::new();

    // The registry: the script's, never a session's.
    let (code, out) = f.why(&["--host", "registry.example", "--provisioning"]);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.starts_with("ALLOWED\n"), "{out}");
    assert!(out.contains("manifest:provisioning.network.allow"), "{out}");
    assert!(
        out.contains("at runtime: DENIED") && out.contains("sessions never get"),
        "{out}"
    );
    let (code, out) = f.why(&["--host", "registry.example"]);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.starts_with("DENIED\n"), "{out}");
    assert!(
        out.contains("during provisioning: ALLOWED") && out.contains("never a session"),
        "{out}"
    );

    // The runtime's host: a session's, and not inherited by the script.
    let (_, out) = f.why(&["--host", "runtime.example", "--provisioning"]);
    assert!(out.starts_with("DENIED\n"), "{out}");
    assert!(
        out.contains("at runtime: ALLOWED") && out.contains("inherits nothing"),
        "{out}"
    );
    let (_, out) = f.why(&["--host", "runtime.example"]);
    assert!(out.starts_with("ALLOWED\n"), "{out}");
    assert!(
        out.contains("during provisioning: DENIED")
            && out.contains("declare it in [provisioning.network] allow"),
        "{out}"
    );

    // A host neither allows: denied in both, and nothing more to say.
    // Refused by devcroft's provisioning default, not by `[network] default`.
    let (_, out) = f.why(&["--host", "elsewhere.example", "--provisioning"]);
    assert!(out.starts_with("DENIED\ndenied by rule baseline:"), "{out}");
    assert!(!out.contains("at runtime:"), "{out}");

    // The filesystem is shared, and the answer says so.
    let (code, out) = f.why(&["--path", "/etc/hostname", "--op", "read", "--provisioning"]);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.contains("same filesystem as runtime"), "{out}");

    // `--env` answers for sessions only.
    let (code, out) = f.why(&["--env", "PATH", "--provisioning"]);
    assert_eq!(code, Some(2), "{out}");
}
