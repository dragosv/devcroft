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

#[test]
fn no_credential_routes_are_configured_yet() {
    // Brokering is task group 3 and is not built. Asserting it *empty* now is
    // what makes the later addition deliberate: a route appearing without a
    // manifest key to declare it would be exactly the silent widening the
    // `brokered-credentials` spec forbids.
    assert!(
        config().routes.is_empty(),
        "no brokered route exists until the manifest can declare one"
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
        proxy_config(Vec::new(), "t").strict_filter,
        "`HostFilter::new_strict`'s rule: an empty allowlist denies. A proxy \
         that failed open on one would be a silent footgun for any caller that \
         forgets to check `wants_egress_proxy()` first"
    );
}
