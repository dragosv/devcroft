//! **The three capabilities devcroft refuses stay off** (`adopt-nono-proxy`
//! tasks 2.1/2.2, design D3).
//!
//! `nono-proxy` compiles in TLS interception, SPIFFE and AWS routing, and
//! `system-keyring` is its only Cargo feature — so none of the three can be
//! compiled out. 31 of the 66 crates the dependency costs exist to serve them.
//! Unreachable is therefore a property of devcroft's *configuration*, and a
//! comment cannot enforce it.
//!
//! **The failure mode this guards is not sabotage.** Nobody is going to enable
//! TLS interception on purpose. What happens is an upstream `ProxyConfig`
//! default moving in a minor release, and devcroft silently inheriting a
//! capability its own threat model says it does not have. A test fails on that
//! upgrade; a comment does not.
//!
//! That is not hypothetical here. It already happened once, on the first
//! config field examined: `strict_connect_auth` defaults to `false`, which
//! would have tunnelled CONNECT without authentication and reopened the
//! open-relay hole `add-egress-proxy` task group 4a closed.

use devcroft::proxy::backend::proxy_config;

/// Deliberately not a minimal manifest's worth of input: an allowlist with
/// several entries and a realistic token, because a guard that only holds for
/// the empty case is not a guard.
fn config() -> nono_proxy::ProxyConfig {
    proxy_config(
        vec![
            "api.anthropic.com".to_string(),
            "*.githubusercontent.com".to_string(),
            "crates.io".to_string(),
        ],
        "0123456789abcdef0123456789abcdef",
        Vec::new(),
    )
}

#[test]
fn tls_interception_is_off() {
    let c = config();
    assert!(
        c.intercept_ca_dir.is_none(),
        "TLS interception is an explicit non-goal (docs/threat-model.md); a CA \
         directory means the proxy would generate certificates for the sandbox"
    );
    assert!(
        c.intercept_parent_ca_pems.is_none(),
        "a parent CA is interception configuration by another name"
    );
    assert!(
        c.preloaded_ca.is_none(),
        "a preloaded CA is interception configuration by another name"
    );
}

/// A route exists only when the manifest declared one. The empty case is the
/// one that matters: a route appearing without a `[[broker]]` entry behind it
/// would be exactly the silent widening the `brokered-credentials` spec
/// forbids.
#[test]
fn no_route_appears_without_a_manifest_declaring_it() {
    assert!(
        config().routes.is_empty(),
        "no brokered route exists unless the manifest declares one"
    );
}

/// And when one *is* declared, the secret is referenced rather than carried.
#[test]
fn a_declared_route_names_its_secret_instead_of_holding_it() {
    let c = proxy_config(
        vec!["api.anthropic.com".to_string()],
        "0123456789abcdef0123456789abcdef",
        vec![devcroft::proxy::backend::BrokerRoute {
            prefix: "anthropic".to_string(),
            upstream: "https://api.anthropic.com".to_string(),
            header: "x-api-key".to_string(),
            secret_var: "DEVCROFT_BROKER_SECRET_ANTHROPIC".to_string(),
        }],
    );
    assert_eq!(c.routes.len(), 1);
    let r = &c.routes[0];
    assert_eq!(
        r.credential_key.as_deref(),
        Some("env://DEVCROFT_BROKER_SECRET_ANTHROPIC"),
        "the route must reference the secret by variable, never embed its value"
    );
    assert_eq!(
        r.inject_header, "x-api-key",
        "Anthropic reads x-api-key; injecting into Authorization would send \
         `Bearer <key>` to an API that ignores it, and the failure would look \
         like a bad key rather than a bad header"
    );
    assert!(
        r.credential_format.is_none(),
        "left to the crate deliberately: it builds `Bearer {{}}` for an \
         Authorization header and the bare secret for any other, which is \
         exactly what x-api-key wants"
    );
}

#[test]
fn authentication_is_required_including_on_connect() {
    let c = config();
    assert!(c.require_auth, "an unauthenticated proxy is an open relay");
    assert!(
        c.session_token.is_some(),
        "devcroft mints the token in `proxy::spawn`; the crate must check that \
         one rather than minting a second"
    );
    // The one that matters, and the reason this file exists: the crate's
    // default here is `false`, on the stated grounds that undici does not echo
    // URL userinfo as `Proxy-Authorization` on CONNECT. Measured otherwise —
    // undici 8.10.2 and curl 8.7.1 both send it — so devcroft keeps refusing.
    assert!(
        c.strict_connect_auth,
        "CONNECT must fail closed on bad auth; the crate defaults this to false \
         and taking that default reopens the open-relay hole task group 4a closed"
    );
}

#[test]
fn an_empty_allowlist_denies_rather_than_allowing_everything() {
    assert!(
        proxy_config(Vec::new(), "t", Vec::new()).strict_filter,
        "`HostFilter::new_strict`'s rule: an empty allowlist denies. A proxy \
         that failed open on one would be a silent footgun for any caller that \
         forgets to check `wants_egress_proxy()` first"
    );
}

/// No certificate authority reaches the sandbox (task 3.5).
///
/// **This assertion failed when first written**, and that is the point of it.
/// The crate defaults `intercept_ca_env_vars` to five names —
/// `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`, `NODE_EXTRA_CA_CERTS`,
/// `CURL_CA_BUNDLE`, `GIT_SSL_CAINFO`. They are inert while
/// `intercept_ca_dir` is `None`, so nothing leaked; but leaving them defaulted
/// would make devcroft's refusal depend on a *second* field staying unset, and
/// the second switch is the one an upstream release moves quietly. Pinned
/// empty, so devcroft states what it wants instead of inheriting what it gets.
#[test]
fn no_certificate_authority_is_exported_to_the_sandbox() {
    assert!(
        config().intercept_ca_env_vars.is_empty(),
        "devcroft installs no CA into the sandbox; exporting CA variables would \
         advertise an interception it does not perform"
    );
}

/// **The credential-absence guarantee, checked where it is actually made.**
///
/// `broker_env` builds every variable a brokered sandbox receives, and it is
/// never given the resolved secret — the guarantee is in the signature. This
/// test pins the *shape* so the two derived names, and the phantom token's
/// value, cannot drift into something that would carry one.
#[test]
fn the_sandbox_environment_carries_a_phantom_token_and_no_credential() {
    let manifest = r#"
[sandbox]
name = "p"
[network]
allow = ["api.anthropic.com"]
[[broker]]
provider = "anthropic"
upstream = "https://api.anthropic.com"
secret = "env:SOME_REAL_KEY"
"#;
    let (m, _) = devcroft::config::parse(manifest).unwrap();
    let vars = devcroft::proxy::backend::broker_env(&m.brokers, 4711, "session-token-abc");

    assert_eq!(
        vars,
        vec![
            (
                "ANTHROPIC_BASE_URL".to_string(),
                "http://127.0.0.1:4711/anthropic".to_string()
            ),
            (
                "ANTHROPIC_API_KEY".to_string(),
                "session-token-abc".to_string()
            ),
        ],
        "both names derive from the provider prefix, and the key is the session \
         token — an SDK that requires a key finds one, and it is worthless \
         outside this proxy"
    );

    // The manifest named where the real secret lives; nothing here may echo it.
    assert!(
        !vars
            .iter()
            .any(|(k, v)| k.contains("SOME_REAL_KEY") || v.contains("SOME_REAL_KEY")),
        "the sandbox is told nothing about where the credential came from"
    );
}
