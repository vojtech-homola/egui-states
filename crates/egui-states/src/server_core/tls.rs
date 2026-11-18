//! The server's TLS identity: its certificate, key, and fingerprint.
//!
//! Without a supplied certificate, the server generates a self-signed one in
//! memory. It is never written to disk, so every process presents a new one.

use std::io;
use std::path::Path;
use std::sync::Arc;

use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, KeyPair, KeyUsagePurpose,
};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::pem::{self, PemObject};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use crate::server_core::http::web::LoadError;
use crate::server_core::server::TlsSettings;
use crate::tls::{ALPN, VERSIONS, fingerprint, hex, provider};

/// Names a generated certificate is issued for. Browsers warn about a
/// self-signed certificate whatever its names, and native clients that pin a
/// fingerprint or accept any certificate do not check them.
const GENERATED_NAMES: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// How long a generated certificate is valid, in days: the longest validity
/// browsers accept. It starts a day early, in case a clock is a little behind.
const GENERATED_DAYS: u64 = 397;

/// The name a generated certificate carries, which browsers show in their
/// warning and certificate viewer.
const GENERATED_SUBJECT: &str = "egui-states";

/// What a server presents to TLS clients.
pub(crate) struct TlsIdentity {
    pub(crate) acceptor: TlsAcceptor,
    /// SHA-256 of the server certificate, in lowercase hexadecimal.
    pub(crate) fingerprint: String,
}

impl TlsIdentity {
    /// Builds the identity `settings` ask for; `None` when TLS is off.
    pub(crate) fn from_settings(settings: &TlsSettings<'_>) -> Result<Option<Self>, LoadError> {
        let invalid = |message: &str| Err(LoadError::Invalid(message.to_string()));
        match (settings.enabled, settings.cert, settings.key) {
            (false, None, None) => Ok(None),
            (false, _, _) => invalid("tls_cert and tls_key require tls to be enabled"),
            (true, None, None) => Self::generate().map(Some),
            (true, Some(cert), Some(key)) => Self::load(cert, key).map(Some),
            (true, _, _) => invalid("tls_cert and tls_key must be given together"),
        }
    }

    /// Generates a self-signed certificate.
    fn generate() -> Result<Self, LoadError> {
        let (certificate, key) = generate_certificate().map_err(|error| {
            LoadError::Invalid(format!("generating the TLS certificate failed: {error}"))
        })?;
        Self::new(vec![certificate], key)
    }

    /// Loads a certificate chain and its private key from PEM files.
    fn load(cert: &Path, key: &Path) -> Result<Self, LoadError> {
        let chain = CertificateDer::pem_slice_iter(&read(cert)?)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| pem_error(cert, "certificate", error))?;
        if chain.is_empty() {
            return Err(LoadError::Invalid(format!(
                "{} contains no PEM certificate",
                cert.display()
            )));
        }
        let key_der = PrivateKeyDer::from_pem_slice(&read(key)?)
            .map_err(|error| pem_error(key, "private key", error))?;
        Self::new(chain, key_der)
    }

    fn new(
        chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
    ) -> Result<Self, LoadError> {
        let fingerprint = hex(&fingerprint(&chain[0]));
        // `with_single_cert` also checks that the key belongs to the certificate.
        let mut config = ServerConfig::builder_with_provider(provider())
            .with_protocol_versions(VERSIONS)
            .and_then(|builder| builder.with_no_client_auth().with_single_cert(chain, key))
            .map_err(|error| {
                LoadError::Invalid(format!("invalid TLS certificate or key: {error}"))
            })?;
        config.alpn_protocols = vec![ALPN.to_vec()];
        Ok(Self {
            acceptor: TlsAcceptor::from(Arc::new(config)),
            fingerprint,
        })
    }
}

/// Creates a key and a self-signed certificate for it.
fn generate_certificate() -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), rcgen::Error>
{
    let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;
    let names = GENERATED_NAMES.map(str::to_string).to_vec();
    let mut params = CertificateParams::new(names)?;
    let mut subject = DistinguishedName::new();
    subject.push(DnType::CommonName, GENERATED_SUBJECT);
    params.distinguished_name = subject;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let day = std::time::Duration::from_secs(24 * 60 * 60);
    let epoch = rcgen::date_time_ymd(1970, 1, 1);
    params.not_before = epoch + now.saturating_sub(day);
    params.not_after = epoch + now + day * GENERATED_DAYS as u32;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let certificate = params.self_signed(&key)?;
    let key = PrivatePkcs8KeyDer::from(key.serialize_der());
    Ok((certificate.der().clone(), key.into()))
}

fn read(path: &Path) -> Result<Vec<u8>, LoadError> {
    std::fs::read(path).map_err(|error| {
        LoadError::Io(io::Error::new(
            error.kind(),
            format!("reading {} failed: {error}", path.display()),
        ))
    })
}

fn pem_error(path: &Path, what: &str, error: pem::Error) -> LoadError {
    let reason = match error {
        // An encrypted key is in a section of its own, which is skipped.
        pem::Error::NoItemsFound => {
            format!("no PEM {what} found (an encrypted key cannot be used)")
        }
        error => format!("invalid PEM {what}: {error}"),
    };
    LoadError::Invalid(format!("{}: {reason}", path.display()))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A self-signed certificate and its key, as PEM text.
    pub(crate) fn pem_pair(names: &[&str]) -> (String, String) {
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let names = names
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>();
        let certificate = CertificateParams::new(names)
            .unwrap()
            .self_signed(&key)
            .unwrap();
        (
            to_pem("CERTIFICATE", certificate.der()),
            to_pem("PRIVATE KEY", &key.serialize_der()),
        )
    }

    /// Wraps DER bytes in a PEM section.
    pub(crate) fn to_pem(label: &str, der: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut base64 = String::new();
        for chunk in der.chunks(3) {
            let bytes = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let value = u32::from_be_bytes([0, bytes[0], bytes[1], bytes[2]]);
            for index in 0..4 {
                if index <= chunk.len() {
                    base64.push(ALPHABET[(value >> (18 - 6 * index) & 63) as usize] as char);
                } else {
                    base64.push('=');
                }
            }
        }
        let mut text = format!("-----BEGIN {label}-----\n");
        for line in base64.as_bytes().chunks(64) {
            text.push_str(std::str::from_utf8(line).unwrap());
            text.push('\n');
        }
        text.push_str(&format!("-----END {label}-----\n"));
        text
    }

    /// The DER bytes of the first PEM certificate in `pem`.
    pub(crate) fn certificate_der(pem: &str) -> Vec<u8> {
        CertificateDer::from_pem_slice(pem.as_bytes())
            .unwrap()
            .to_vec()
    }

    struct Files(PathBuf);

    impl Files {
        fn new(files: &[(&str, &str)]) -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "egui-states-tls-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            for (name, content) in files {
                std::fs::write(path.join(name), content).unwrap();
            }
            Self(path)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn settings<'a>(
        enabled: bool,
        cert: Option<&'a Path>,
        key: Option<&'a Path>,
    ) -> TlsSettings<'a> {
        TlsSettings { enabled, cert, key }
    }

    fn invalid(result: Result<Option<TlsIdentity>, LoadError>) -> String {
        match result {
            Err(LoadError::Invalid(message)) => message,
            Err(LoadError::Io(error)) => panic!("expected invalid settings, got {error}"),
            Ok(_) => panic!("expected invalid settings"),
        }
    }

    #[test]
    fn generated_identities_are_new_and_report_a_sha256_fingerprint() {
        assert!(
            TlsIdentity::from_settings(&settings(false, None, None))
                .unwrap()
                .is_none()
        );
        let first = TlsIdentity::from_settings(&settings(true, None, None))
            .unwrap()
            .unwrap();
        let second = TlsIdentity::from_settings(&settings(true, None, None))
            .unwrap()
            .unwrap();
        assert_eq!(first.fingerprint.len(), 64);
        assert!(
            first
                .fingerprint
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert_ne!(first.fingerprint, second.fingerprint);
    }

    #[test]
    fn generated_certificates_are_named_after_the_library() {
        let (certificate, _key) = generate_certificate().unwrap();
        let contains = |text: &str| {
            certificate
                .windows(text.len())
                .any(|window| window == text.as_bytes())
        };
        assert!(contains(GENERATED_SUBJECT));
        assert!(!contains("rcgen"));
    }

    #[test]
    fn supplied_certificates_are_loaded_and_checked() {
        let (cert, key) = pem_pair(&["pc-a.local"]);
        let (_, other_key) = pem_pair(&["pc-a.local"]);
        let files = Files::new(&[
            ("cert.pem", &cert),
            ("key.pem", &key),
            ("other-key.pem", &other_key),
            ("garbage.pem", "not a certificate"),
            (
                "encrypted.pem",
                "-----BEGIN ENCRYPTED PRIVATE KEY-----\nAAAA\n-----END ENCRYPTED PRIVATE KEY-----\n",
            ),
        ]);
        let (cert_path, key_path) = (files.path("cert.pem"), files.path("key.pem"));

        let identity =
            TlsIdentity::from_settings(&settings(true, Some(&cert_path), Some(&key_path)))
                .unwrap()
                .unwrap();
        assert_eq!(
            identity.fingerprint,
            hex(&fingerprint(&certificate_der(&cert)))
        );

        let message = invalid(TlsIdentity::from_settings(&settings(
            true,
            Some(&cert_path),
            None,
        )));
        assert!(message.contains("together"), "{message}");
        let message = invalid(TlsIdentity::from_settings(&settings(
            false,
            Some(&cert_path),
            Some(&key_path),
        )));
        assert!(message.contains("require tls"), "{message}");

        let other_key = files.path("other-key.pem");
        let message = invalid(TlsIdentity::from_settings(&settings(
            true,
            Some(&cert_path),
            Some(&other_key),
        )));
        assert!(
            message.contains("invalid TLS certificate or key"),
            "{message}"
        );

        let garbage = files.path("garbage.pem");
        let message = invalid(TlsIdentity::from_settings(&settings(
            true,
            Some(&garbage),
            Some(&key_path),
        )));
        assert!(message.contains("no PEM certificate"), "{message}");
        let encrypted = files.path("encrypted.pem");
        let message = invalid(TlsIdentity::from_settings(&settings(
            true,
            Some(&cert_path),
            Some(&encrypted),
        )));
        assert!(message.contains("encrypted"), "{message}");

        let missing = files.path("missing.pem");
        match TlsIdentity::from_settings(&settings(true, Some(&missing), Some(&key_path))) {
            Err(LoadError::Io(error)) => {
                assert_eq!(error.kind(), io::ErrorKind::NotFound);
                assert!(error.to_string().contains("missing.pem"), "{error}");
            }
            _ => panic!("expected an I/O error"),
        }
    }

    #[test]
    fn pem_encoding_matches_the_parser() {
        for length in [1, 2, 3, 47, 48, 49, 100] {
            let der: Vec<u8> = (0..length as u8).collect();
            let pem = to_pem("CERTIFICATE", &der);
            let parsed = CertificateDer::from_pem_slice(pem.as_bytes()).unwrap();
            assert_eq!(parsed.to_vec(), der, "{length}");
        }
    }
}
