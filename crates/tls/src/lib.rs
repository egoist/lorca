//! The certificate trust every Device connection uses: providers and their sign-ins, the
//! relay, lorca.app, and MCP servers over HTTPS.
//!
//! macOS and Windows check a server's certificate with the system, so a root an
//! administrator installed for a TLS-inspecting proxy counts, as it does in the browser.
//! Linux trusts its CA store and the web's roots bundled here, so a container without a store
//! still connects. Phones keep the bundled roots.

use std::sync::{Arc, OnceLock};

/// A reqwest client over [`client_config`]. `reqwest::Client::new()` would trust the bundled
/// roots alone.
pub fn client() -> reqwest::Client {
    client_builder().build().expect("a client over a built TLS config")
}

pub fn client_builder() -> reqwest::ClientBuilder {
    // HTTP/1.1 alone: the CLI builds reqwest 0.12 without HTTP/2.
    reqwest::Client::builder().use_preconfigured_tls(client_config(&["http/1.1"]))
}

/// A failed request in words: reqwest's own, which name the request ("error sending request for
/// url (…)"), then the last cause in the chain, which tells a certificate the system does not
/// trust ("invalid peer certificate: UnknownIssuer") from a refused connection.
pub fn describe(error: &reqwest::Error) -> String {
    let mut cause = None;
    let mut next = std::error::Error::source(error);
    while let Some(source) = next {
        cause = Some(source);
        next = source.source();
    }
    match cause {
        Some(cause) => format!("{error}: {cause}"),
        None => error.to_string(),
    }
}

/// The rustls config, offering `alpn` in the handshake. A reqwest client with this config
/// sends exactly these protocols.
pub fn client_config(alpn: &[&str]) -> rustls::ClientConfig {
    let mut config = base().clone();
    config.alpn_protocols = alpn.iter().map(|protocol| protocol.as_bytes().to_vec()).collect();
    config
}

/// Built once: Linux reads its CA store from disk. The provider is named because the build
/// links both ring and aws-lc-rs, and rustls picks neither by itself.
fn base() -> &'static rustls::ClientConfig {
    static CONFIG: OnceLock<rustls::ClientConfig> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .expect("ring supports the default TLS versions");
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let builder = builder.dangerous().with_custom_certificate_verifier(Arc::new(system_verifier(provider)));
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let builder = builder.with_root_certificates(roots());
        builder.with_no_client_auth()
    })
}

/// The system's verifier, also trusting the certificates in the PEM file `SSL_CERT_FILE`
/// names.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn system_verifier(provider: Arc<rustls::crypto::CryptoProvider>) -> rustls_platform_verifier::Verifier {
    use rustls_platform_verifier::Verifier;

    let extra = extra_roots();
    if !extra.is_empty() {
        match Verifier::new_with_extra_roots(extra, provider.clone()) {
            Ok(verifier) => return verifier,
            Err(error) => tracing::warn!("SSL_CERT_FILE: {error}"),
        }
    }
    Verifier::new(provider).expect("the system verifier needs no setup on macOS and Windows")
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn extra_roots() -> Vec<rustls::pki_types::CertificateDer<'static>> {
    use rustls::pki_types::{pem::PemObject, CertificateDer};

    let Some(path) = std::env::var_os("SSL_CERT_FILE") else { return Vec::new() };
    let certs = match CertificateDer::pem_file_iter(&path) {
        Ok(certs) => certs,
        Err(error) => {
            tracing::warn!("SSL_CERT_FILE {}: {error}", path.to_string_lossy());
            return Vec::new();
        }
    };
    certs
        .filter_map(|cert| cert.inspect_err(|error| tracing::warn!("SSL_CERT_FILE {}: {error}", path.to_string_lossy())).ok())
        .collect()
}

/// The bundled roots, and on Linux the system's store: rustls-native-certs reads
/// `SSL_CERT_FILE` and `SSL_CERT_DIR` in its place when either is set.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn roots() -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    #[cfg(target_os = "linux")]
    {
        let native = rustls_native_certs::load_native_certs();
        for error in &native.errors {
            tracing::debug!("system CA store: {error}");
        }
        roots.add_parsable_certificates(native.certs);
    }
    roots
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn describe_names_the_cause_after_the_request() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("https://127.0.0.1:{}/", listener.local_addr().unwrap().port());
        drop(listener);
        let error = super::client().get(&url).send().await.unwrap_err();
        let text = super::describe(&error);
        let cause = text.strip_prefix(&format!("error sending request for url ({url}): ")).unwrap_or_else(|| panic!("{text}"));
        assert!(!cause.is_empty() && !cause.contains("error sending request"), "{text}");
    }
}
