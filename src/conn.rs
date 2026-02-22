use std::sync::Arc;

use rustls::{
    ClientConfig, RootCertStore,
    client::{
        danger::{ServerCertVerified, ServerCertVerifier},
        verify_server_cert_signed_by_trust_anchor,
    },
    crypto::{
        CryptoProvider, WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature,
    },
    pki_types::CertificateDer,
    server::ParsedCertificate,
};

/// Return a `ClientConfig` suitable for connecting to Sonos speakers.
pub fn tls_config() -> Result<ClientConfig, rustls::Error> {
    let verifier = CustomVerifier::new()?;
    Ok(ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth())
}

/// Custom server certificate verifier that only verifies against the Sonos certificate.
///
/// It is pretty much copied from [`WebPkiServerVerifier`](https://docs.rs/rustls/latest/rustls/client/struct.WebPkiServerVerifier.html) with the following changes:
/// * The root certificate store only contains the Sonos certificate
/// * doesn't do revocation
/// * Disable server name verification, so that we can connect using IP addresses.
#[derive(Debug)]
pub struct CustomVerifier {
    roots: Arc<RootCertStore>,
    supported: WebPkiSupportedAlgorithms,
}

impl CustomVerifier {
    const ROOT_CA_CERT: &[u8] = include_bytes!("../registered_ca_root.cer");

    pub fn new() -> Result<Self, rustls::Error> {
        let cert = CertificateDer::from_slice(Self::ROOT_CA_CERT);
        let mut root_store = RootCertStore::empty();
        root_store.add(cert)?;
        let provider = CryptoProvider::get_default().unwrap();
        Ok(Self {
            roots: Arc::new(root_store),
            supported: provider.signature_verification_algorithms,
        })
    }
}

impl ServerCertVerifier for CustomVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        ocsp_response: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let cert = ParsedCertificate::try_from(end_entity)?;

        // Note: we use the crate-internal `_impl` fn here in order to provide revocation
        // checking information, if applicable.
        verify_server_cert_signed_by_trust_anchor(
            &cert,
            &self.roots,
            intermediates,
            now,
            self.supported.all,
        )?;

        if !ocsp_response.is_empty() {
            eprintln!("Unvalidated OCSP response: {:?}", ocsp_response.to_vec());
        }

        // verify_server_name(&cert, server_name)?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.supported)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.supported)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.supported.supported_schemes()
    }
}
