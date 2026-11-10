//! Identidad del host y verificación por huella (certificate pinning).
//!
//! No hay autoridad certificadora: cada host genera un certificado autofirmado
//! y el cliente lo acepta solo si su huella SHA-256 es la que esperaba. La
//! huella se comunica por un canal aparte (en la Etapa 1, a mano).

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

use crate::TransportError;

/// Identificador ALPN: ambos extremos deben anunciar el mismo o el saludo TLS
/// falla. Evita hablar por error con otro servicio QUIC.
const ALPN: &[u8] = b"disacomp/1";

/// Nombre que el cliente pone en el saludo TLS. La verificación es por huella,
/// así que solo tiene que ser un nombre sintácticamente válido.
pub(crate) const SERVER_NAME: &str = "disacomp";

/// Huella SHA-256 de un certificado. Identifica al host de forma inequívoca.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    fn of(certificate: &CertificateDer<'_>) -> Self {
        Self(Sha256::digest(certificate.as_ref()).into())
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({self})")
    }
}

/// La cadena no es una huella válida.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("la huella debe tener 64 caracteres hexadecimales")]
pub struct InvalidFingerprint;

impl FromStr for Fingerprint {
    type Err = InvalidFingerprint;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(InvalidFingerprint);
        }
        let mut bytes = [0u8; 32];
        let (pairs, _) = text.as_bytes().as_chunks::<2>();
        for (byte, pair) in bytes.iter_mut().zip(pairs) {
            let pair = std::str::from_utf8(pair).map_err(|_| InvalidFingerprint)?;
            *byte = u8::from_str_radix(pair, 16).map_err(|_| InvalidFingerprint)?;
        }
        Ok(Self(bytes))
    }
}

/// Certificado autofirmado y clave privada de un host.
pub struct Identity {
    certificate: CertificateDer<'static>,
    key: PrivatePkcs8KeyDer<'static>,
    fingerprint: Fingerprint,
}

impl Identity {
    /// Genera un par de claves y un certificado nuevos.
    pub fn generate() -> Result<Self, TransportError> {
        let generated = rcgen::generate_simple_self_signed([SERVER_NAME.to_owned()])?;
        let certificate = generated.cert.der().clone();
        let key = PrivatePkcs8KeyDer::from(generated.signing_key.serialize_der());
        let fingerprint = Fingerprint::of(&certificate);
        Ok(Self {
            certificate,
            key,
            fingerprint,
        })
    }

    /// Huella que el cliente necesita para conectarse a este host.
    pub fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

pub(crate) fn server_config(identity: &Identity) -> Result<rustls::ServerConfig, TransportError> {
    let mut config = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(
            vec![identity.certificate.clone()],
            identity.key.clone_key().into(),
        )?;
    config.alpn_protocols = vec![ALPN.to_vec()];
    Ok(config)
}

/// Configuración TLS del cliente y un indicador que se activa si el host
/// presentó un certificado con una huella distinta de la esperada.
pub(crate) fn client_config(
    expected: Fingerprint,
) -> Result<(rustls::ClientConfig, Arc<AtomicBool>), TransportError> {
    let provider = provider();
    let mismatch = Arc::new(AtomicBool::new(false));
    let verifier = PinnedCertVerifier {
        expected,
        algorithms: provider.signature_verification_algorithms,
        mismatch: Arc::clone(&mismatch),
    };

    // `dangerous()` es el nombre que rustls da a cualquier verificador propio.
    // Aquí no se desactiva nada: se sustituye "firmado por una CA conocida"
    // por "es exactamente el certificado que esperaba", que es más estricto.
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    config.alpn_protocols = vec![ALPN.to_vec()];
    Ok((config, mismatch))
}

/// Acepta únicamente el certificado cuya huella coincide con la esperada.
#[derive(Debug)]
struct PinnedCertVerifier {
    expected: Fingerprint,
    algorithms: WebPkiSupportedAlgorithms,
    mismatch: Arc<AtomicBool>,
}

impl ServerCertVerifier for PinnedCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if Fingerprint::of(end_entity) == self.expected {
            Ok(ServerCertVerified::assertion())
        } else {
            self.mismatch.store(true, Ordering::Relaxed);
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    // Comprobar la huella no basta: el certificado es público y cualquiera
    // podría presentarlo. Estas dos funciones verifican que el host firma el
    // saludo con la clave privada de ese certificado, es decir, que la posee.

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_text_roundtrip() {
        let fingerprint = Identity::generate().unwrap().fingerprint();
        let text = fingerprint.to_string();
        assert_eq!(text.len(), 64);
        assert_eq!(text.parse::<Fingerprint>(), Ok(fingerprint));
        assert_eq!(text.to_uppercase().parse::<Fingerprint>(), Ok(fingerprint));
    }

    #[test]
    fn fingerprint_rejects_invalid_text() {
        assert!("".parse::<Fingerprint>().is_err());
        assert!("abc".parse::<Fingerprint>().is_err());
        assert!("zz".repeat(32).parse::<Fingerprint>().is_err());
        assert!("ñ".repeat(32).parse::<Fingerprint>().is_err());
        assert!("+a".repeat(32).parse::<Fingerprint>().is_err());
        assert!("ab".repeat(33).parse::<Fingerprint>().is_err());
    }

    #[test]
    fn each_identity_has_its_own_fingerprint() {
        let a = Identity::generate().unwrap();
        let b = Identity::generate().unwrap();
        assert_ne!(a.fingerprint(), b.fingerprint());
    }
}
