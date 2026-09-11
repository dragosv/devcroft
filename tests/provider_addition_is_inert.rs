//! config spec (add-devenv-provider): "Adding a provider does not change
//! any other manifest".
//!
//! A new `env.provider` value is only additive if a manifest that does not
//! name it compiles to exactly what it compiled to before. That is easy to
//! claim and easy to break — a baseline rule reordered while adding a
//! provider changes every sandbox on the host, and nothing else in the
//! suite would notice, because every other policy test asserts properties
//! rather than the whole compiled artifact.
//!
//! So this pins the artifact itself. The golden below was produced by the
//! commit *before* `devenv` existed as a provider and re-checked against
//! the current tree; a diff here means a change intended for one provider
//! reached the policy every other manifest gets.

use devcroft::policy::compile;

/// Deliberately exercises more than the defaults: a manifest with only
/// `[sandbox]` would compile to the baseline alone and would not notice a
/// change in how manifest-sourced rules are annotated or ordered.
const MANIFEST: &str = r#"
[sandbox]
name = "inert"

[env]
provider = "flox"

[filesystem]
allow = ["."]
read = ["/opt/shared"]

[network]
default = "deny"
allow = ["api.example.com", "crates.io"]
"#;

#[test]
fn a_manifest_that_does_not_name_devenv_compiles_exactly_as_before() {
    let (manifest, warnings) = devcroft::config::parse(MANIFEST).expect("manifest parses");
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    let rendered = format!("{:#?}", compile(&manifest));
    let golden = include_str!("golden/policy_without_devenv.txt");

    assert_eq!(
        rendered.trim(),
        golden.trim(),
        "compiling a manifest that names another provider must produce byte-identical \
         output; adding a provider is additive or it is not additive"
    );
}
