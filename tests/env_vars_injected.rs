//! `[env.vars]`, end to end (config spec, "Static environment
//! variables"): a table of literal variables "injected into every
//! session, applied AFTER provider resolution so they can override
//! provider-set values".
//!
//! The requirement was specified and parsed from the start -- with a
//! validation warning for `$` interpolation and a config-level test that
//! arbitrary keys survive the schema check -- and then nothing consumed
//! it. `env.vars` was read nowhere outside `src/config/`, so it never
//! reached the keeper and never reached a session: setting it was a
//! silent no-op, which is the failure mode a parse-level test cannot
//! see. This file drives the real binary instead, and asserts what a
//! session actually observes.
//!
//! Uses the nix provider rather than flox because its dev shell sets a
//! distinctive variable of its own (`DEVCROFT_NIX_E2E`), which is what
//! makes "overrides a provider-set value" testable rather than merely
//! "is present". Same fixture shape as `tests/nix_provider_e2e.rs`.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn devcroft_bin() -> &'static str {
    env!("CARGO_BIN_EXE_devcroft")
}

fn nix_available() -> bool {
    Command::new("nix")
        .arg("flake")
        .arg("--help")
        .output()
        .is_ok_and(|o| o.status.success())
        // `nix flake --help` succeeds without a usable store, which is
        // the capability-not-binary rule the whole suite follows.
        && devcroft::provider::host_can_build_nix_closures()
}

/// Exports `DEVCROFT_NIX_E2E=present`, the value the manifest below
/// overrides.
const FLAKE_NIX: &str = r#"
{
  description = "devcroft env.vars e2e fixture";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      shellFor = system: (import nixpkgs { inherit system; }).mkShell {
        DEVCROFT_NIX_E2E = "present";
      };
    in {
      devShells = builtins.listToAttrs (map (system: {
        name = system;
        value = { default = shellFor system; };
      }) systems);
    };
}
"#;

struct Sandbox {
    name: String,
    project_root: PathBuf,
}

impl Sandbox {
    fn new(tag: &str, env_vars: &str) -> Option<Self> {
        if !nix_available() {
            eprintln!("skipping: a flakes-enabled nix is not on PATH");
            return None;
        }
        unsafe {
            std::env::set_var("DEVCROFT_KEEPER_EXE", devcroft_bin());
        }

        let project_root =
            std::env::temp_dir().join(format!("devcroft-envvars-e2e-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&project_root);
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::write(project_root.join("flake.nix"), FLAKE_NIX).unwrap();

        let lock = Command::new("nix")
            .arg("flake")
            .arg("lock")
            .arg(&project_root)
            .output()
            .unwrap();
        if !lock.status.success() {
            eprintln!(
                "skipping: nix flake lock failed (likely no network for nixpkgs): {}",
                String::from_utf8_lossy(&lock.stderr)
            );
            return None;
        }

        let name = format!("e2eenvvars{tag}{}", std::process::id());
        std::fs::write(
            project_root.join("devcroft.toml"),
            format!("[sandbox]\nname = {name:?}\n\n[env]\nprovider = \"nix\"\n{env_vars}"),
        )
        .unwrap();

        Some(Sandbox { name, project_root })
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(devcroft_bin())
            .args(args)
            .current_dir(&self.project_root)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn echo(&self, var: &str) -> String {
        let out = self.run(&["exec", "--", "sh", "-c", &format!("echo ${var}")]);
        assert!(out.status.success(), "{out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn state_root(&self) -> PathBuf {
        devcroft::lifecycle::StatePaths::new(&self.name)
            .unwrap()
            .root
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = self.run(&["rm", "--yes"]);
        let _ = std::fs::remove_dir_all(self.state_root());
        let _ = std::fs::remove_dir_all(&self.project_root);
    }
}

/// The config spec's own scenario, verbatim: "WHEN the provider sets
/// `FOO=a` and the manifest sets `vars = { FOO = "b" }` THEN sessions see
/// `FOO=b`".
#[test]
fn a_manifest_var_overrides_the_providers_own_value_and_a_new_one_is_injected() {
    let Some(sandbox) = Sandbox::new(
        "override",
        "\n[env.vars]\nDEVCROFT_NIX_E2E = \"overridden\"\nDEVCROFT_FRESH = \"injected\"\n",
    ) else {
        return;
    };
    assert!(sandbox.run(&["up"]).status.success());

    assert_eq!(
        sandbox.echo("DEVCROFT_NIX_E2E"),
        "overridden",
        "[env.vars] must be applied after provider resolution and win over \
         the value the flake's dev shell set (config spec: \"applied AFTER \
         provider resolution so they can override provider-set values\"). \
         Reading `present` means the manifest table reached nothing -- the \
         no-op this test exists to catch."
    );
    assert_eq!(
        sandbox.echo("DEVCROFT_FRESH"),
        "injected",
        "a key the provider never set must still reach the session"
    );
}

/// Values are literal. `config::validate` warns on a `$` rather than
/// expanding it, because reading the host's environment here would leak
/// exactly the non-reproducible state devcroft exists to exclude -- so
/// the session must observe the dollar sign, not an expansion and not an
/// empty string.
#[test]
fn values_are_literal_and_never_interpolated_against_the_host() {
    let Some(sandbox) = Sandbox::new("literal", "\n[env.vars]\nTOKEN = \"$HOME\"\n") else {
        return;
    };
    assert!(sandbox.run(&["up"]).status.success());

    // Single-quoted so the assertion is about what devcroft injected,
    // not about what the session's own shell would expand.
    let out = sandbox.run(&["exec", "--", "sh", "-c", "printf '%s' \"$TOKEN\""]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "$HOME",
        "the literal string must survive into the session; expanding it \
         host-side would inject non-reproducible host state"
    );
}
