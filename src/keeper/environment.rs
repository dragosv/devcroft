//! Environment names shared by the host lifecycle and guest keeper.

/// devcroft's own variables in a keeper's environment, which sessions must
/// not inherit (`keeper::session`): everything `KeeperEnv::vars` adds
/// besides `SHELL` and `PATH`, which are for sessions, plus what the init
/// helper and `spawn_keeper` add. Named one by one rather than by prefix,
/// since a project may name its own variables `DEVCROFT_*` (this repository's
/// tests do) and a fleet task's process is meant to see `DEVCROFT_TASK`.
const KEEPER_INTERNAL: [&str; 14] = [
    "DEVCROFT_CAPABILITY_PLAN",
    "DEVCROFT_SSH_HOST_KEY",
    "DEVCROFT_SSH_AUTHORIZED_KEY",
    "DEVCROFT_SHELL",
    "DEVCROFT_START_SERVICES",
    "DEVCROFT_SANDBOX_NAME",
    "DEVCROFT_SERVICES_ROOT",
    "DEVCROFT_SANDBOX_HOME",
    "DEVCROFT_PROXY_RELAY_PORT",
    "DEVCROFT_PROXY_SOCKET",
    "DEVCROFT_PROXY_RELAY_FD",
    "DEVCROFT_ACTIVATION_PROVISIONED",
    ACTIVATION_FD_ENV,
    ACTIVATION_OVERLAY_ENV,
];

/// The provisioning process's end of the pipe its activation script's
/// exports come back on (`run_provisioning`).
pub const ACTIVATION_FD_ENV: &str = "DEVCROFT_ACTIVATION_FD";

/// What the activation script exported, as `up` read it, for the keeper to
/// adopt (`spawn_keeper`).
pub const ACTIVATION_OVERLAY_ENV: &str = "DEVCROFT_ACTIVATION_OVERLAY";

/// Whether `name` is one of devcroft's own keeper variables (see
/// `KEEPER_INTERNAL`); the hooks are `DEVCROFT_HOOK_<NAME>`.
pub(crate) fn is_keeper_internal(name: &str) -> bool {
    KEEPER_INTERNAL.contains(&name) || name.starts_with("DEVCROFT_HOOK_")
}
