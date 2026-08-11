//! `devcroft ssh-config` (ssh spec's "ssh-config emission" requirement,
//! task 6.2), through the real built binary: no `nono`/`flox` needed here
//! — `HOME` is pointed at a scratch dir so both the client keypair and
//! `~/.ssh/config` land somewhere disposable.

use std::path::PathBuf;
use std::process::Command;

fn run(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_devcroft"))
        .args(args)
        .env("HOME", home)
        .output()
        .unwrap()
}

fn scratch_home(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "devcroft-ssh-config-cli-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn ssh_config_prints_the_wildcard_block_and_generates_keys_on_first_use() {
    let home = scratch_home("print");

    let out = run(&home, &["ssh-config"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Host *.devcroft"));
    assert!(stdout.contains("ProxyCommand devcroft proxy %n"));
    assert!(stdout.contains("StrictHostKeyChecking no"));
    assert!(
        stdout.contains(
            &home
                .join(".local/share/devcroft/id_ed25519")
                .to_string_lossy()
                .into_owned()
        )
    );

    // ssh spec's "First run generates keys" scenario.
    assert!(home.join(".local/share/devcroft/id_ed25519").exists());
    assert!(home.join(".local/share/devcroft/id_ed25519.pub").exists());

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn ssh_config_write_is_idempotent_through_the_real_cli() {
    let home = scratch_home("write");
    let config_path = home.join(".ssh/config");

    let first = run(&home, &["ssh-config", "--write"]);
    assert!(first.status.success());
    let first_contents = std::fs::read_to_string(&config_path).unwrap();
    assert!(first_contents.contains("Host *.devcroft"));

    let second = run(&home, &["ssh-config", "--write"]);
    assert!(second.status.success());
    let second_contents = std::fs::read_to_string(&config_path).unwrap();

    assert_eq!(
        first_contents, second_contents,
        "~/.ssh/config must be identical after running --write twice"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn ssh_config_write_preserves_a_pre_existing_config_file() {
    let home = scratch_home("preserve");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    let config_path = home.join(".ssh/config");
    std::fs::write(&config_path, "Host example\n  HostName example.com\n").unwrap();

    let out = run(&home, &["ssh-config", "--write"]);
    assert!(out.status.success());

    let contents = std::fs::read_to_string(&config_path).unwrap();
    assert!(contents.contains("Host example\n  HostName example.com\n"));
    assert!(contents.contains("Host *.devcroft"));

    let _ = std::fs::remove_dir_all(&home);
}
