//! TLS material shared by every TLS listener (HTTPS / HTTP-2, HTTP/3,
//! raw QUIC).
//!
//! One certificate is generated (or loaded) once at server startup and
//! reused for all three, so the three listeners present a consistent
//! identity. Per-listener ALPN differs and is supplied at config-build
//! time. Clients in this tool always skip certificate validation, so a
//! self-signed cert with `localhost` / `127.0.0.1` SANs works across
//! machines too.

use std::path::Path;
use std::sync::Arc;

use axum_server::tls_rustls::RustlsConfig;
use eyre::{Result, eyre};
use rcgen::{CertifiedKey, generate_simple_self_signed};
use rustls::ServerConfig;
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// A resolved certificate chain + private key, shared by all TLS
/// listeners. Cloneable so each listener can build its own
/// ALPN-specific [`ServerConfig`].
pub struct TlsMaterial {
    certs: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
}

impl Clone for TlsMaterial {
    fn clone(&self) -> Self {
        // `PrivateKeyDer` has no `Clone` impl; `clone_key` is its
        // explicit deep copy.
        Self {
            certs: self.certs.clone(),
            key: self.key.clone_key(),
        }
    }
}

impl TlsMaterial {
    /// Generate a fresh self-signed certificate (SANs `localhost` and
    /// `127.0.0.1`).
    pub fn self_signed() -> Result<Self> {
        let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
        let CertifiedKey { cert, signing_key } = generate_simple_self_signed(subject_alt_names)
            .map_err(|e| eyre!("self-signed certificate generation failed: {e}"))?;

        let certs = vec![cert.der().clone()];
        let key = PrivateKeyDer::try_from(signing_key.serialize_der())
            .map_err(|e| eyre!("failed to encode generated private key: {e}"))?;
        Ok(Self { certs, key })
    }

    /// Load a certificate chain + key from PEM files.
    pub fn from_pem_files(cert_path: &Path, key_path: &Path) -> Result<Self> {
        let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert_path)
            .map_err(|e| eyre!("reading certificate {}: {e}", cert_path.display()))?
            .collect::<std::result::Result<_, _>>()
            .map_err(|e| eyre!("parsing certificate {}: {e}", cert_path.display()))?;
        if certs.is_empty() {
            return Err(eyre!("no certificates found in {}", cert_path.display()));
        }
        let key = PrivateKeyDer::from_pem_file(key_path)
            .map_err(|e| eyre!("reading private key {}: {e}", key_path.display()))?;
        Ok(Self { certs, key })
    }

    /// Build a rustls [`ServerConfig`] with the given ALPN protocol
    /// list. Pinned to TLS 1.3 with the aws-lc-rs provider — required
    /// by QUIC and harmless for the HTTPS listener.
    pub fn server_config(&self, alpn: &[&[u8]]) -> Result<ServerConfig> {
        let mut config =
            ServerConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
                .with_protocol_versions(&[&rustls::version::TLS13])
                .map_err(|e| eyre!("rustls TLS 1.3 setup failed: {e}"))?
                .with_no_client_auth()
                .with_single_cert(self.certs.clone(), self.key.clone_key())
                .map_err(|e| eyre!("rustls server config (cert/key) failed: {e}"))?;
        config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
        // QUIC permits early data to be 0 or u32::MAX; harmless for TCP TLS.
        config.max_early_data_size = u32::MAX;
        Ok(config)
    }

    /// Build the axum-server [`RustlsConfig`] for the HTTPS (HTTP/2 over
    /// TLS) listener. ALPN is restricted to `h2` so an HTTP/1.1-over-TLS
    /// client fails the ALPN negotiation instead of being served h2.
    pub fn axum_rustls_config(&self) -> Result<RustlsConfig> {
        let server_config = self.server_config(&[b"h2"])?;
        Ok(RustlsConfig::from_config(Arc::new(server_config)))
    }
}
