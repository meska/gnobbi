//! TLS fixture tests. No account or actual Matrix server is involved.
use super::*;
use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    child: Child,
    root: PathBuf,
    base: String,
}
impl Fixture {
    fn start(mode: &str) -> Self {
        let root = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!(
                "discovery-fixture-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir(&root).unwrap();
        let mut child = Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/discovery_https.py"
            ))
            .arg(&root)
            .arg(mode)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut base = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut base)
            .unwrap();
        let fixture = Self {
            child,
            root,
            base: base.trim().to_owned(),
        };
        assert!(
            fixture.base.starts_with("https://127.0.0.1:"),
            "fixture readiness failed"
        );
        fixture
    }
    fn discovery(&self) -> Discovery {
        let pem = std::fs::read(self.root.join("ca.pem")).unwrap();
        let http = secure_http_builder()
            .add_root_certificate(reqwest::Certificate::from_pem(&pem).unwrap())
            .no_proxy()
            .build()
            .unwrap();
        Discovery { http }
    }
    fn requests(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.root.join("requests.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().unwrap();
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
fn run<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

#[cfg(unix)]
#[test]
fn signin_explicit_ca_keeps_hostname_and_expiry_verification() {
    use crate::signin::{Secret, SignInError, SignInProfile};
    for mode in ["wrong-host", "expired"] {
        let f = Fixture::start(mode);
        let root = f.root.join("signin");
        let mut profile = SignInProfile::open(root.clone()).unwrap();
        let result = run(profile.password_login(
            f.base.clone(),
            "@alice:fixture.invalid".into(),
            Secret::new("fixture-not-a-real-password".into()).unwrap(),
            Secret::new("protocol fixture unlock phrase".into()).unwrap(),
            Some(std::fs::read(f.root.join("ca.pem")).unwrap()),
        ));
        assert!(matches!(result, Err(SignInError::Discovery)), "mode {mode}");
        assert!(f.requests().is_empty());
        assert!(!root.join("account-1").exists());
    }
}

#[cfg(unix)]
#[test]
fn signin_unsupported_flow_never_posts_credentials_or_creates_an_account_store() {
    use crate::signin::{Secret, SignInError, SignInProfile};
    for mode in ["sso-only", "empty-flows"] {
        let f = Fixture::start(mode);
        let root = f.root.join("signin");
        let mut profile = SignInProfile::open(root.clone()).unwrap();
        let result = run(profile.password_login(
            f.base.clone(),
            "@alice:fixture.invalid".into(),
            Secret::new("fixture-not-a-real-password".into()).unwrap(),
            Secret::new("protocol fixture unlock phrase".into()).unwrap(),
            Some(std::fs::read(f.root.join("ca.pem")).unwrap()),
        ));
        assert!(
            matches!(result, Err(SignInError::UnsupportedLogin)),
            "mode {mode}"
        );
        assert_eq!(f.requests().len(), 2);
        assert!(!root.join("account-1").exists());
        assert_eq!(profile.active_account(), None);
    }
}

#[cfg(unix)]
#[test]
fn signin_without_sliding_sync_stops_before_credentials_and_account_store() {
    use crate::signin::{Secret, SignInError, SignInProfile};
    let f = Fixture::start("default");
    let root = f.root.join("signin");
    let mut profile = SignInProfile::open(root.clone()).unwrap();
    let result = run(profile.password_login(
        f.base.clone(),
        "@alice:fixture.invalid".into(),
        Secret::new("fixture-not-a-real-password".into()).unwrap(),
        Secret::new("protocol fixture unlock phrase".into()).unwrap(),
        Some(std::fs::read(f.root.join("ca.pem")).unwrap()),
    ));
    assert!(matches!(result, Err(SignInError::NoSlidingSync)));
    assert_eq!(f.requests().len(), 2, "only versions and login flows");
    assert!(!root.join("account-1").exists());
    assert_eq!(profile.active_account(), None);
}

#[cfg(unix)]
#[test]
fn signin_with_sliding_sync_advertised_goes_on_to_credentials() {
    use crate::signin::{Secret, SignInError, SignInProfile};
    let f = Fixture::start("sliding");
    let mut profile = SignInProfile::open(f.root.join("signin")).unwrap();
    let result = run(profile.password_login(
        f.base.clone(),
        "@alice:fixture.invalid".into(),
        Secret::new("fixture-not-a-real-password".into()).unwrap(),
        Secret::new("protocol fixture unlock phrase".into()).unwrap(),
        Some(std::fs::read(f.root.join("ca.pem")).unwrap()),
    ));
    assert!(!matches!(result, Err(SignInError::NoSlidingSync)));
}

#[test]
fn ambiguous_redirect_authorities_are_rejected_before_url_normalization() {
    for mode in ["ambiguous", "empty-userinfo"] {
        let f = Fixture::start(&format!("unsafe-redirect-{mode}"));
        let result = run(f
            .discovery()
            .discover_server_name(f.base.strip_prefix("https://").unwrap()));
        assert!(
            matches!(result, Err(DiscoveryError::InvalidRedirect)),
            "mode {mode}"
        );
        assert_eq!(f.requests().len(), 1);
    }
}

#[test]
fn redirect_loop_stops_after_three_hops_without_fallback() {
    let f = Fixture::start("loop");
    let result = run(f
        .discovery()
        .discover_server_name(f.base.strip_prefix("https://").unwrap()));
    assert!(matches!(result, Err(DiscoveryError::RedirectLimit)));
    assert_eq!(f.requests().len(), 4);
}

#[test]
fn unsafe_redirect_targets_are_not_requested() {
    for mode in ["http", "userinfo", "query", "fragment", "missing"] {
        let f = Fixture::start(&format!("unsafe-redirect-{mode}"));
        let result = run(f
            .discovery()
            .discover_server_name(f.base.strip_prefix("https://").unwrap()));
        assert!(
            matches!(result, Err(DiscoveryError::InvalidRedirect)),
            "mode {mode}"
        );
        assert_eq!(f.requests().len(), 1);
    }
}

#[test]
fn cross_origin_https_redirect_rechecks_tls() {
    let f = Fixture::start("cross-origin");
    let result = run(f
        .discovery()
        .discover_server_name(f.base.strip_prefix("https://").unwrap()))
    .unwrap();
    assert_eq!(result.login_flows.len(), 4);
    assert_eq!(f.requests().len(), 4);
}

#[test]
fn explicit_url_preserves_base_path_and_validates_redirected_api_endpoints() {
    let f = Fixture::start("endpoint-redirect");
    let result = run(f
        .discovery()
        .probe_url(HomeserverUrl::parse(&format!("{}/prefix", f.base)).unwrap()))
    .unwrap();
    assert_eq!(result.server_versions, ["v1.15"]);
    assert_eq!(
        result.login_flows[0].support(),
        LoginSupport::SdkPasswordOnly
    );
    assert_eq!(
        f.requests()
            .iter()
            .map(|r| r["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "/prefix/_matrix/client/versions",
            "/final-versions",
            "/prefix/_matrix/client/v3/login",
            "/final-login"
        ]
    );
}

#[test]
fn default_trust_rejects_the_fixture_ca_before_any_http_request() {
    let f = Fixture::start("good");
    let result = run(Discovery::new()
        .unwrap()
        .probe_url(HomeserverUrl::parse(&f.base).unwrap()));
    assert!(matches!(result, Err(DiscoveryError::Transport)));
    assert!(f.requests().is_empty());
}

#[test]
fn trusted_ca_does_not_bypass_hostname_verification() {
    let f = Fixture::start("wrong-host");
    let result = run(f
        .discovery()
        .probe_url(HomeserverUrl::parse(&f.base).unwrap()));
    assert!(matches!(result, Err(DiscoveryError::Transport)));
    assert!(f.requests().is_empty());
}

#[test]
fn trusted_ca_does_not_bypass_expiry_verification() {
    let f = Fixture::start("expired");
    let result = run(f
        .discovery()
        .discover_server_name(f.base.strip_prefix("https://").unwrap()));
    assert!(matches!(result, Err(DiscoveryError::Transport)));
    assert!(f.requests().is_empty());
}

#[test]
fn invalid_remote_data_stops_discovery_without_leaking_payloads() {
    for (mode, error, requests) in [
        ("bad-base", DiscoveryError::InvalidUrl, 1),
        ("bad-schema", DiscoveryError::InvalidResponse, 1),
        ("bad-json", DiscoveryError::InvalidResponse, 1),
        ("bad-versions", DiscoveryError::InvalidResponse, 2),
        ("bad-flows", DiscoveryError::InvalidResponse, 3),
        ("server-error", DiscoveryError::HttpStatus(503), 1),
        ("oversized-stream", DiscoveryError::InvalidResponse, 1),
    ] {
        let f = Fixture::start(mode);
        let result = run(f
            .discovery()
            .discover_server_name(f.base.strip_prefix("https://").unwrap()));
        assert!(matches!(result, Err(e) if e == error), "mode {mode}");
        assert_eq!(f.requests().len(), requests);
        assert!(!format!("{error:?}").contains("fixture-sensitive-marker"));
    }
}

#[test]
fn sso_only_and_empty_flows_do_not_invent_password_support() {
    for mode in ["sso-only", "empty-flows"] {
        let f = Fixture::start(mode);
        let result = run(f
            .discovery()
            .probe_url(HomeserverUrl::parse(&f.base).unwrap()))
        .unwrap();
        assert!(
            result
                .login_flows
                .iter()
                .all(|flow| flow.support() == LoginSupport::Unsupported)
        );
        assert_eq!(result.login_flows.len(), usize::from(mode == "sso-only"));
    }
}

#[test]
fn invalid_server_names_do_not_make_network_requests() {
    let f = Fixture::start("good");
    let d = f.discovery();
    for name in [
        "https://example.invalid",
        " example.invalid",
        "example.invalid/path",
        "@user:example.invalid",
        "",
        "example.invalid?secret",
        "user:secret@example.invalid",
    ] {
        assert!(
            matches!(
                run(d.discover_server_name(name)),
                Err(DiscoveryError::InvalidServerName)
            ),
            "invalid server-name fixture"
        );
    }
    assert!(f.requests().is_empty());
}

#[test]
fn slow_response_body_is_cancelled_by_the_http_deadline() {
    let f = Fixture::start("slow");
    let pem = std::fs::read(f.root.join("ca.pem")).unwrap();
    let d = Discovery {
        http: secure_http_builder()
            .add_root_certificate(reqwest::Certificate::from_pem(&pem).unwrap())
            .no_proxy()
            .timeout(std::time::Duration::from_millis(100))
            .build()
            .unwrap(),
    };
    let start = std::time::Instant::now();
    let result = run(d.probe_url(HomeserverUrl::parse(&f.base).unwrap()));
    assert!(matches!(result, Err(DiscoveryError::Transport)));
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(f.requests().len(), 1);
}

#[test]
fn oversized_discovery_data_is_rejected_before_parsing() {
    let f = Fixture::start("oversized");
    let result = run(f
        .discovery()
        .discover_server_name(f.base.strip_prefix("https://").unwrap()));
    assert!(matches!(result, Err(DiscoveryError::InvalidResponse)));
    assert_eq!(f.requests().len(), 1);
}

#[test]
fn follows_three_validated_relative_https_redirects() {
    let f = Fixture::start("redirect");
    let result = run(f
        .discovery()
        .discover_server_name(f.base.strip_prefix("https://").unwrap()))
    .unwrap();
    assert_eq!(result.homeserver.as_str(), format!("{}/delegated/", f.base));
    assert_eq!(f.requests().len(), 6);
}

#[test]
fn absent_well_known_falls_back_to_the_original_https_origin() {
    let f = Fixture::start("missing");
    let result = run(f
        .discovery()
        .discover_server_name(f.base.strip_prefix("https://").unwrap()))
    .unwrap();
    assert_eq!(result.homeserver.as_str(), format!("{}/", f.base));
    assert_eq!(f.requests()[1]["path"], "/_matrix/client/versions");
}

#[test]
fn discovers_delegation_and_reports_server_advertised_login_flows() {
    let f = Fixture::start("good");
    let d = f.discovery();
    let result = run(d.discover_server_name(f.base.strip_prefix("https://").unwrap())).unwrap();
    assert_eq!(result.homeserver.as_str(), format!("{}/delegated/", f.base));
    assert_eq!(result.server_versions, ["v1.1", "v1.15"]);
    assert_eq!(result.login_flows.len(), 4);
    assert_eq!(result.login_flows[0].kind(), "m.login.password");
    assert_eq!(
        result.login_flows[0].support(),
        LoginSupport::SdkPasswordOnly
    );
    for flow in &result.login_flows[1..] {
        assert_eq!(flow.support(), LoginSupport::Unsupported);
    }
    let requests = f.requests();
    assert_eq!(
        requests
            .iter()
            .map(|r| r["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "/.well-known/matrix/client",
            "/delegated/_matrix/client/versions",
            "/delegated/_matrix/client/v3/login"
        ]
    );
    assert!(requests.iter().all(|r| r["authorization"] == false));
}

#[test]
fn apex_with_delegation_resolves_for_name_and_for_url_but_a_prefixed_url_is_not_rerouted() {
    let f = Fixture::start("apex");
    let d = f.discovery();
    let delegated = format!("{}/delegated/", f.base);
    let name = f.base.strip_prefix("https://").unwrap();
    for bare in [true, false] {
        let r = run(d.resolve(HomeserverUrl::parse(&f.base).unwrap(), bare)).unwrap();
        assert_eq!(r.homeserver.as_str(), delegated, "bare {bare}");
    }
    assert!(!name.is_empty());
    let prefixed = HomeserverUrl::parse(&format!("{}/other", f.base)).unwrap();
    assert!(matches!(
        run(d.resolve(prefixed, false)),
        Err(DiscoveryError::HttpStatus(404))
    ));
}
