//! The native client's TLS configuration for each [`Tls`] mode.

use std::fmt;
use std::io;
use std::sync::Arc;

use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::crypto::{
    WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature,
};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, Error, OtherError, SignatureScheme,
};

use crate::client::tls::{Tls, parse_fingerprint};
use crate::tls::{ALPN, VERSIONS, fingerprint, hex, provider};

/// Builds the connector for `tls` and the name the client presents for `host`.
pub(crate) fn connector(
    tls: &Tls,
    host: &str,
) -> Result<(TlsConnector, ServerName<'static>), String> {
    let (config, name) = client_config(tls, host)?;
    Ok((TlsConnector::from(config), name))
}

/// Builds the client configuration for `tls` and the name the client
/// presents for `host`.
pub(crate) fn client_config(
    tls: &Tls,
    host: &str,
) -> Result<(Arc<ClientConfig>, ServerName<'static>), String> {
    let builder = ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(VERSIONS)
        .map_err(|error| error.to_string())?;
    let pinned = |expected| {
        let verifier = PinVerifier {
            expected,
            algorithms: provider().signature_verification_algorithms,
        };
        builder
            .clone()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
    };
    let (builder, name) = match tls {
        Tls::AcceptAny => (pinned(None), pinned_name(host)),
        Tls::Fingerprint(text) => (pinned(Some(parse_fingerprint(text)?)), pinned_name(host)),
        Tls::System => {
            use rustls_platform_verifier::BuilderVerifierExt;
            let builder = builder.clone().with_platform_verifier().map_err(|error| {
                format!("the operating system's certificate store cannot be used: {error}")
            })?;
            let name = ServerName::try_from(host.to_string())
                .map_err(|_| format!("{host:?} is not a valid host name for TLS"))?;
            (builder, name)
        }
    };
    let mut config = builder.with_no_client_auth();
    config.alpn_protocols = vec![ALPN.to_vec()];
    Ok((Arc::new(config), name))
}

/// The name sent to the server in the pinned modes, which do not check it.
fn pinned_name(host: &str) -> ServerName<'static> {
    ServerName::try_from(host.to_string())
        .unwrap_or_else(|_| ServerName::try_from("localhost").expect("a valid name"))
}

/// Explains why the TLS handshake failed.
pub(crate) fn describe(error: &io::Error) -> String {
    let tls_error = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<Error>());
    match tls_error {
        Some(Error::InvalidCertificate(CertificateError::Other(other))) => {
            match other.0.downcast_ref::<Mismatch>() {
                Some(mismatch) => mismatch.to_string(),
                None => format!("the server's certificate is not trusted: {other}"),
            }
        }
        Some(Error::InvalidCertificate(error)) => {
            format!("the server's certificate is not trusted: {error}")
        }
        // Plain HTTP, such as an error page, where a TLS record should be.
        Some(Error::InvalidMessage(_)) => "the server does not use TLS".to_string(),
        Some(error) => format!("TLS handshake failed: {error}"),
        None => match error.kind() {
            io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted => {
                "the server closed the connection during the TLS handshake; it may not use TLS"
                    .to_string()
            }
            _ => format!("TLS handshake failed: {error}"),
        },
    }
}

/// A certificate other than the expected one.
#[derive(Debug)]
struct Mismatch {
    presented: String,
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the server's certificate does not match the fingerprint (it presented {})",
            self.presented
        )
    }
}

impl std::error::Error for Mismatch {}

/// Accepts the certificate with the `expected` fingerprint, or any with none.
///
/// The server must still prove that it holds the certificate's private key:
/// the handshake signature is verified as usual. Without that check, anyone
/// could replay the certificate, which is public.
#[derive(Debug)]
struct PinVerifier {
    expected: Option<[u8; 32]>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        if let Some(expected) = self.expected {
            // The certificate is public, so the comparison need not be constant-time.
            let presented = fingerprint(end_entity);
            if presented != expected {
                return Err(Error::InvalidCertificate(CertificateError::Other(
                    OtherError(Arc::new(Mismatch {
                        presented: hex(&presented),
                    })),
                )));
            }
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}
