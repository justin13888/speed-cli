use axum_server::tls_rustls::RustlsConfig;
use eyre::{Result, eyre};
use rcgen::{CertifiedKey, generate_simple_self_signed};

/// Generates a self-signed certificate for testing HTTPS server.
pub async fn get_self_signed_cert() -> Result<RustlsConfig> {
    let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];

    let CertifiedKey { cert, signing_key } =
        generate_simple_self_signed(subject_alt_names).unwrap();

    let cert_pem = cert.pem();
    let signing_key_pem = signing_key.serialize_pem();

    RustlsConfig::from_pem(cert_pem.into_bytes(), signing_key_pem.into_bytes())
        .await
        .map_err(|e| eyre!("Failed to create RustlsConfig: {}", e))
}
