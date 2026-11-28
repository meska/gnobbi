//! The one place HTTP clients start from.

/// Android: rustls-platform-verifier needs JNI init and Java classes the NativeActivity app does not
/// have (it panics), so TLS verifies against the embedded Mozilla roots, like matrix-engine.
/// shortcut: certificates the user installed on the phone are not trusted; upgrade by shipping the verifier's AAR.
pub fn client_builder() -> reqwest::ClientBuilder {
    let b = reqwest::Client::builder();
    #[cfg(target_os = "android")]
    let b = b.tls_certs_only(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().filter_map(|der| reqwest::Certificate::from_der(der).ok()));
    b
}
