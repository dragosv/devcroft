//! `add-devenv-services` task 1.3: growing `ServiceDecl` for a second
//! provider must not change what the first one renders.
//!
//! Pinned as the whole document rather than as properties. Every other
//! services test asserts one field at a time, so a change in key order,
//! an added default, or a dropped clause would pass all of them — and
//! the claim being made here is byte-identity, which only the whole
//! artifact can carry.
//!
//! The golden was produced by the commit *before* `ServiceDecl` grew and
//! re-checked against the current tree.

use devcroft::provider::{RestartPolicy, ServiceDecl, Shutdown};
use std::collections::BTreeMap;
use std::path::Path;

/// Both shapes flox can declare: an ordinary foreground service, and a
/// daemon with the shutdown command that is the only way to stop one.
fn flox_declarations() -> Vec<ServiceDecl> {
    vec![
        ServiceDecl {
            name: "web".to_string(),
            command: "python3 -m http.server".to_string(),
            vars: BTreeMap::from([
                ("PORT".to_string(), "8080".to_string()),
                ("LOG_LEVEL".to_string(), "debug".to_string()),
            ]),
            is_daemon: false,
            working_dir: None,
            depends_on: Vec::new(),
            restart: RestartPolicy::Never,
            shutdown: Shutdown::Default,
            readiness: None,
        },
        ServiceDecl {
            name: "db".to_string(),
            command: "pg_ctl start".to_string(),
            vars: BTreeMap::new(),
            is_daemon: true,
            working_dir: None,
            depends_on: Vec::new(),
            restart: RestartPolicy::Never,
            shutdown: Shutdown::Command("pg_ctl stop".to_string()),
            readiness: None,
        },
    ]
}

#[test]
fn a_flox_declaration_renders_exactly_as_it_did_before_servicedecl_grew() {
    let rendered = devcroft::services::render_config(
        &flox_declarations(),
        Path::new("/nix/store/abc-bash/bin/bash"),
    );
    let golden = include_str!("golden/service_config_flox.json");

    assert_eq!(
        rendered.trim(),
        golden.trim(),
        "growing `ServiceDecl` for devenv must leave flox's rendered supervisor config \
         byte-identical; a provider gaining fields is additive or it is not additive"
    );
}
