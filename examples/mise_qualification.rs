//! Group 0 measurement helper: apply the real policy compiler, then exec a
//! qualified artifact under it. Not part of the published crate's include list.
use std::collections::BTreeMap;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: mise_qualification PROJECT REQUEST.json".into());
    }
    let root = Path::new(&args[1]).canonicalize()?;
    let request: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let grants: Vec<String> = serde_json::from_value(request["grants"].clone())?;
    let env: BTreeMap<String, String> = serde_json::from_value(request["env"].clone())?;
    let argv: Vec<String> = serde_json::from_value(request["argv"].clone())?;
    if argv.is_empty() {
        return Err("argv must not be empty".into());
    }
    let (manifest, _) = devcroft::config::parse("[sandbox]\nname = 'miseprobe'\n")?;
    let compiled = devcroft::policy::compile(&manifest).with_provider_grants("mise", &grants);
    let plan = compiled.to_capability_plan();
    let caps = plan.to_capability_set(&root)?;
    // Do not grant the executable directory or a compiler just because it is
    // available to this measurement process. The child must use only the plan.
    nono::Sandbox::apply_auto(&caps)?;
    let error = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(&root)
        .env_clear()
        .envs(env)
        .exec();
    Err(error.into())
}
