//! `add-own-userland` 2.4: the project pins the runtime generation.
//!
//! The first two run the CLI against a manifest alone: the pin is checked,
//! and the unpinned line printed, before any provider resolves anything,
//! so neither needs Nix. The third needs a live keeper, and takes it from
//! the provider row.

mod common;

use std::process::{Command, Stdio};

fn devcroft(cwd: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// A project whose provider will fail to resolve (an empty directory has
/// no flake), so `up` never gets as far as starting anything.
fn project(tag: &str, runtime: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dc-gen-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("devcroft.toml"),
        format!(
            "[sandbox]\nname = \"dcgen{tag}{}\"\n\n[env]\nprovider = \"nix\"\n{runtime}",
            std::process::id()
        ),
    )
    .unwrap();
    dir
}

#[test]
fn a_mismatched_pin_is_refused_at_layer_config_naming_both() {
    let other = devcroft::runtime::GENERATION + 1;
    let dir = project("mismatch", &format!("\n[runtime]\ngeneration = {other}\n"));
    let out = devcroft(&dir, &["up"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let _ = devcroft(&dir, &["rm", "--yes"]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("config"), "names its layer: {stderr}");
    assert!(
        stderr.contains(&format!("generation {other}"))
            && stderr.contains(&format!("generation {}", devcroft::runtime::GENERATION)),
        "names both generations: {stderr}"
    );
}

#[test]
fn an_unpinned_project_is_told_its_generation_and_how_to_pin_it() {
    let dir = project("unpinned", "");
    let out = devcroft(&dir, &["up"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let _ = devcroft(&dir, &["rm", "--yes"]);
    let _ = std::fs::remove_dir_all(&dir);
    let g = devcroft::runtime::GENERATION;
    assert!(
        stderr.contains(&format!("runtime generation {g}"))
            && stderr.contains(&format!("generation = {g}")),
        "{stderr}"
    );
    // It is the provider that fails here, not the pin.
    assert_ne!(out.status.code(), Some(2), "{stderr}");
}

#[test]
fn a_live_keeper_keeps_its_generation_and_status_says_so() {
    if !devcroft::policy::backend_supported() {
        eprintln!("skipping: this host has no usable Landlock/Seatbelt support");
        return;
    }
    // SAFETY: this process runs a single test that sets it.
    unsafe {
        std::env::set_var("DEVCROFT_KEEPER_EXE", env!("CARGO_BIN_EXE_devcroft"));
    }
    common::for_each_row("rtgen", |fx| {
        fx.bring_up(&devcroft::lifecycle::UpOptions::default())
            .unwrap();
        let paths = devcroft::lifecycle::StatePaths::new(fx.sandbox_name()).unwrap();
        let mut meta = devcroft::lifecycle::read_meta(&paths.meta)
            .unwrap()
            .unwrap();
        let g = devcroft::runtime::GENERATION;
        assert_eq!(meta.runtime_generation, Some(g), "`up` records it");

        let out = devcroft(fx.project_root(), &["status"]);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains(&format!("runtime: generation {g}\n")),
            "{stdout}"
        );

        // An older keeper, as if the binary had been upgraded under it.
        let older = g + 7;
        meta.runtime_generation = Some(older);
        std::fs::write(&paths.meta, serde_json::to_vec_pretty(&meta).unwrap()).unwrap();
        let out = devcroft(fx.project_root(), &["status"]);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains(&format!("generation {older}"))
                && stdout.contains(&format!("embeds {g}"))
                && stdout.contains("up --recreate"),
            "{stdout}"
        );
        // And `up` against the live keeper does not rewrite it.
        fx.bring_up(&devcroft::lifecycle::UpOptions::default())
            .unwrap();
        let meta = devcroft::lifecycle::read_meta(&paths.meta)
            .unwrap()
            .unwrap();
        assert_eq!(meta.runtime_generation, Some(older));
        let _ = devcroft::lifecycle::down(fx.sandbox_name());
    });
}
