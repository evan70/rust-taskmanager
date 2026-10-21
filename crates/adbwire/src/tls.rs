//! TLS settings for talking to adbd: TLS 1.3, our own certificate presented as the client, and the
//! phone's certificate accepted without a trust chain (it is self-signed; what protects the
//! pairing is the code, mixed into the key exchange).

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, ring as ring_provider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error, ServerConfig, SignatureScheme};
use std::sync::Arc;

pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring_provider::default_provider())
}

#[derive(Debug)]
struct AcceptAny(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(&self, _end: &CertificateDer<'_>, _inter: &[CertificateDer<'_>], _name: &ServerName<'_>, _ocsp: &[u8], _now: UnixTime) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// A client configuration that presents `cert_der` / `pkcs8` and accepts any server certificate.
pub fn client_config(cert_der: &[u8], pkcs8: &[u8]) -> Result<Arc<ClientConfig>, Error> {
    let p = provider();
    let cfg = ClientConfig::builder_with_provider(p.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAny(p)))
        .with_client_auth_cert(vec![CertificateDer::from(cert_der.to_vec())], PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pkcs8.to_vec())))?;
    Ok(Arc::new(cfg))
}

/// A server configuration with a throwaway certificate; used by the tests, which play the phone.
pub fn test_server_config() -> Arc<ServerConfig> {
    let cert = rcgen::generate_simple_self_signed(vec!["adbwire-test".to_string()]).expect("certificate");
    let key = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());
    let cfg = ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("tls13")
        .with_no_client_auth()
        .with_single_cert(vec![cert.cert.der().clone()], PrivateKeyDer::Pkcs8(key))
        .expect("server config");
    Arc::new(cfg)
}
