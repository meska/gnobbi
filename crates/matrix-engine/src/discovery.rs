//! Unauthenticated homeserver preflight. All inputs and remote data are untrusted.
use url::Url;
// The fixture script needs python3, which the Windows runner does not have (and will not get).
#[cfg(all(test, unix))]
#[path = "discovery_tests.rs"]
mod tests;

/// Payload-free errors: never retain remote bodies, URLs or credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryError {
    InvalidUrl,
    InvalidServerName,
    Transport,
    /// The server's certificate was rejected (unknown issuer, wrong host, expired).
    Tls,
    HttpStatus(u16),
    InvalidResponse,
    InvalidRedirect,
    RedirectLimit,
}

/// Validated HTTPS base URL, intentionally not Debug/Display (may be private).
#[derive(Clone, PartialEq, Eq)]
pub struct HomeserverUrl(Url);

impl HomeserverUrl {
    pub fn parse(value: &str) -> Result<Self, DiscoveryError> {
        if value.len() > 2048
            || value
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
        {
            return Err(DiscoveryError::InvalidUrl);
        }
        // The scheme is case-insensitive (iOS capitalises the first letter of what is typed).
        let authority = value
            .get(..8)
            .filter(|scheme| scheme.eq_ignore_ascii_case("https://"))
            .and_then(|_| value[8..].split('/').next())
            .ok_or(DiscoveryError::InvalidUrl)?;
        if authority.is_empty() || authority.contains('@') {
            return Err(DiscoveryError::InvalidUrl);
        }
        let mut url = Url::parse(value).map_err(|_| DiscoveryError::InvalidUrl)?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(DiscoveryError::InvalidUrl);
        }
        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }
        Ok(Self(url))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// `host[:port]` when this is a bare origin (root path), i.e. could be a server name.
    fn server_name(&self) -> Option<String> {
        if self.0.path() != "/" {
            return None;
        }
        let host = self.0.host_str()?;
        Some(match self.0.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_owned(),
        })
    }
}

/// Describes available SDK primitives, NOT a working app login or SSO/OIDC UX.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginSupport {
    SdkPasswordOnly,
    Unsupported,
}

/// Server-provided type is private data: intentionally no Debug/Display.
pub struct LoginFlow {
    kind: String,
}
impl LoginFlow {
    pub fn kind(&self) -> &str {
        &self.kind
    }
    pub fn support(&self) -> LoginSupport {
        if self.kind == "m.login.password" {
            LoginSupport::SdkPasswordOnly
        } else {
            LoginSupport::Unsupported
        }
    }
}

/// Preflight metadata only; no authenticated client/session is created.
pub struct DiscoveryResult {
    pub homeserver: HomeserverUrl,
    pub server_versions: Vec<String>,
    /// `org.matrix.simplified_msc3575` advertised in `/versions`, as the SDK requires.
    pub sliding_sync: bool,
    pub login_flows: Vec<LoginFlow>,
}

/// Call on a background runtime, never block the GUI thread.
pub struct Discovery {
    http: reqwest::Client,
}
fn secure_http_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
}
impl Discovery {
    pub fn new() -> Result<Self, DiscoveryError> {
        Ok(Self {
            http: secure_http_builder()
                .build()
                .map_err(|_| DiscoveryError::Transport)?,
        })
    }

    /// Explicit additional trust for a private homeserver; hostname and expiry
    /// verification remain enabled. Never accepts a server-discovered certificate.
    pub fn with_trusted_root(pem: &[u8]) -> Result<Self, DiscoveryError> {
        if pem.len() > 1048576 {
            return Err(DiscoveryError::InvalidResponse);
        }
        let certs = reqwest::Certificate::from_pem_bundle(pem)
            .map_err(|_| DiscoveryError::InvalidResponse)?;
        if certs.is_empty() {
            return Err(DiscoveryError::InvalidResponse);
        }
        Ok(Self {
            http: crate::auth::with_trust_roots(secure_http_builder(), certs)
                .build().map_err(|_| DiscoveryError::Transport)?,
        })
    }

    pub(crate) fn with_http(http: reqwest::Client) -> Self {
        Self { http }
    }

    pub(crate) async fn probe_federation(&self, name: &str) -> Result<(), crate::search::SearchError> {
        use crate::search::SearchError;
        let base = HomeserverUrl::parse(&format!("https://{name}"))
            .map_err(|_| SearchError::InvalidServer)?;
        // An explicit port is not delegated (spec); a bare name may be.
        // shortcut: no SRV lookup, a delegation by SRV alone is probed on 8448; add when a real server needs it.
        let target = if base.0.port().is_some() { name.to_owned() } else { self.delegated(&base).await? };
        let base = HomeserverUrl::parse(&format!("https://{target}")).map_err(|_| SearchError::InvalidServer)?;
        let url = base.0.join("_matrix/federation/v1/version").map_err(|_| SearchError::InvalidServer)?;
        match self.json(url).await {
            Ok(value) if value.get("server").is_some_and(|v| v.is_object()) => Ok(()),
            Ok(_) | Err(DiscoveryError::InvalidResponse | DiscoveryError::HttpStatus(404)) => Err(SearchError::NotMatrix),
            Err(DiscoveryError::Tls) => Err(SearchError::Tls),
            Err(_) => Err(SearchError::Network),
        }
    }

    /// `host:port` that serves federation for `origin`: `m.server` of its .well-known, else port 8448.
    async fn delegated(&self, origin: &HomeserverUrl) -> Result<String, crate::search::SearchError> {
        let host = origin.0.host_str().ok_or(crate::search::SearchError::InvalidServer)?;
        let url = origin.0.join(".well-known/matrix/server").map_err(|_| crate::search::SearchError::InvalidServer)?;
        let target = match self.json(url).await {
            Ok(v) => v.get("m.server").and_then(|v| v.as_str()).map(str::to_owned),
            Err(DiscoveryError::Tls) => return Err(crate::search::SearchError::Tls),
            Err(_) => None,
        }
        .unwrap_or_else(|| host.to_owned());
        // Delegated or not, a name without a port is served on 8448.
        Ok(if HomeserverUrl::parse(&format!("https://{target}")).is_ok_and(|u| u.0.port().is_some()) { target } else { format!("{target}:8448") })
    }

    /// Sign-in entry point. A bare server name resolves `.well-known` first; a full URL
    /// is probed as typed, but an origin answering 404 falls back to its delegation.
    pub async fn resolve(
        &self,
        homeserver: HomeserverUrl,
        bare_name: bool,
    ) -> Result<DiscoveryResult, DiscoveryError> {
        let Some(name) = homeserver.server_name() else {
            return self.probe_url(homeserver).await;
        };
        if bare_name {
            return self.discover_server_name(&name).await;
        }
        match self.probe_url(homeserver).await {
            Err(DiscoveryError::HttpStatus(404)) => self.discover_server_name(&name).await,
            other => other,
        }
    }

    pub async fn discover_server_name(
        &self,
        name: &str,
    ) -> Result<DiscoveryResult, DiscoveryError> {
        let name = matrix_sdk::ruma::ServerName::parse(name)
            .map_err(|_| DiscoveryError::InvalidServerName)?;
        let origin = HomeserverUrl::parse(&format!("https://{name}"))?;
        let well_known = match self
            .json(
                origin
                    .0
                    .join(".well-known/matrix/client")
                    .map_err(|_| DiscoveryError::InvalidUrl)?,
            )
            .await
        {
            Ok(value) => value,
            Err(DiscoveryError::HttpStatus(404)) => return self.probe_url(origin).await,
            Err(error) => return Err(error),
        };
        let base = well_known
            .get("m.homeserver")
            .and_then(|v| v.get("base_url"))
            .and_then(|v| v.as_str())
            .ok_or(DiscoveryError::InvalidResponse)?;
        self.probe_url(HomeserverUrl::parse(base)?).await
    }

    /// Explicit URL bypasses .well-known; the caller must choose this mode.
    pub async fn probe_url(
        &self,
        homeserver: HomeserverUrl,
    ) -> Result<DiscoveryResult, DiscoveryError> {
        let versions = self
            .json(
                homeserver
                    .0
                    .join("_matrix/client/versions")
                    .map_err(|_| DiscoveryError::InvalidUrl)?,
            )
            .await?;
        let server_versions = string_array(versions.get("versions"))?;
        if server_versions.is_empty() {
            return Err(DiscoveryError::InvalidResponse);
        }
        let sliding_sync = versions
            .get("unstable_features")
            .and_then(|v| v.get("org.matrix.simplified_msc3575"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let login = self
            .json(
                homeserver
                    .0
                    .join("_matrix/client/v3/login")
                    .map_err(|_| DiscoveryError::InvalidUrl)?,
            )
            .await?;
        let flows = login
            .get("flows")
            .and_then(|v| v.as_array())
            .ok_or(DiscoveryError::InvalidResponse)?;
        let login_flows = flows
            .iter()
            .map(|v| {
                let kind = v
                    .get("type")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .ok_or(DiscoveryError::InvalidResponse)?;
                Ok(LoginFlow {
                    kind: kind.to_owned(),
                })
            })
            .collect::<Result<Vec<_>, DiscoveryError>>()?;
        Ok(DiscoveryResult {
            homeserver,
            server_versions,
            sliding_sync,
            login_flows,
        })
    }

    async fn json(&self, mut url: Url) -> Result<serde_json::Value, DiscoveryError> {
        for hops in 0..=3 {
            let response = self
                .http
                .get(url.clone())
                .send()
                .await
                .map_err(|e| if is_certificate_error(&e) { DiscoveryError::Tls } else { DiscoveryError::Transport })?;
            if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                if hops == 3 {
                    return Err(DiscoveryError::RedirectLimit);
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or(DiscoveryError::InvalidRedirect)?;
                if location.len() > 2048
                    || location
                        .chars()
                        .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
                {
                    return Err(DiscoveryError::InvalidRedirect);
                }
                // Validate raw absolute/network-path references before the URL parser
                // can repair malformed authority syntax or erase empty userinfo.
                if location.starts_with("//") {
                    HomeserverUrl::parse(&format!("https:{location}"))
                        .map_err(|_| DiscoveryError::InvalidRedirect)?;
                } else if Url::parse(location).is_ok() {
                    HomeserverUrl::parse(location).map_err(|_| DiscoveryError::InvalidRedirect)?;
                }
                let next = url
                    .join(location)
                    .map_err(|_| DiscoveryError::InvalidRedirect)?;
                // Validate without adding the base-URL trailing slash to an endpoint.
                HomeserverUrl::parse(next.as_str()).map_err(|_| DiscoveryError::InvalidRedirect)?;
                url = next;
                continue;
            }
            if !response.status().is_success() {
                return Err(DiscoveryError::HttpStatus(response.status().as_u16()));
            }
            let mut response = response;
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| DiscoveryError::Transport)?
            {
                if body.len().saturating_add(chunk.len()) > 65536 {
                    return Err(DiscoveryError::InvalidResponse);
                }
                body.extend_from_slice(&chunk);
            }
            return serde_json::from_slice(&body).map_err(|_| DiscoveryError::InvalidResponse);
        }
        Err(DiscoveryError::RedirectLimit)
    }
}
/// shortcut: matches rustls' "invalid peer certificate" text in the source chain; revisit if reqwest/rustls change it.
fn is_certificate_error(error: &reqwest::Error) -> bool {
    let mut source = std::error::Error::source(error);
    while let Some(e) = source {
        if e.to_string().contains("invalid peer certificate") {
            return true;
        }
        source = e.source();
    }
    false
}

fn string_array(value: Option<&serde_json::Value>) -> Result<Vec<String>, DiscoveryError> {
    value
        .and_then(|v| v.as_array())
        .ok_or(DiscoveryError::InvalidResponse)?
        .iter()
        .map(|v| {
            v.as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .ok_or(DiscoveryError::InvalidResponse)
        })
        .collect()
}
