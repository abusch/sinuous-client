use std::sync::{Arc, OnceLock};

use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
    client::{
        WebPkiServerVerifier,
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    },
    crypto::CryptoProvider,
    pki_types::{CertificateDer, ServerName, UnixTime},
};

use crate::error::BoxError;

/// The root CA that signs the certificates presented by Sonos players.
const SONOS_ROOT_CA: &[u8] = include_bytes!("../assets/registered_ca_root.cer");

/// Return a `ClientConfig` suitable for connecting to Sonos speakers.
///
/// The config is built on first use and shared by all connections, so they share its TLS session
/// cache. It uses the process-wide default [`CryptoProvider`] if one is installed at that point,
/// and falls back to `aws-lc-rs` otherwise.
pub fn tls_config() -> Result<Arc<ClientConfig>, BoxError> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();

    if let Some(config) = CONFIG.get() {
        return Ok(config.clone());
    }
    let config = Arc::new(build_tls_config()?);
    Ok(CONFIG.get_or_init(|| config).clone())
}

fn build_tls_config() -> Result<ClientConfig, BoxError> {
    let provider = CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()));
    let verifier = SonosVerifier::new(provider.clone())?;
    Ok(ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth())
}

/// Verifies that server certificates are issued by the Sonos root CA.
///
/// Players present certificates for names like `sonos-<id>.local`, which don't match the IP
/// addresses we connect to, so the server name isn't checked. Everything else (chain, validity
/// period, handshake signatures) is verified as usual.
#[derive(Debug)]
struct SonosVerifier {
    inner: Arc<WebPkiServerVerifier>,
}

impl SonosVerifier {
    fn new(provider: Arc<CryptoProvider>) -> Result<Self, BoxError> {
        let mut roots = RootCertStore::empty();
        roots.add(CertificateDer::from_slice(SONOS_ROOT_CA))?;
        let inner =
            WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider).build()?;
        Ok(Self { inner })
    }
}

impl ServerCertVerifier for SonosVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // The name is only checked once the chain has been verified, so a name mismatch means the
        // certificate is otherwise valid.
        match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => Ok(ServerCertVerified::assertion()),
            result => result,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}
