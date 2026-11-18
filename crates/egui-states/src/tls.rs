//! TLS settings shared by the server and the native client.
//!
//! The crypto provider is always passed to rustls explicitly. The library runs
//! inside other applications and inside a Python extension, so it neither
//! installs nor relies on rustls' process-wide default provider.

use std::sync::{Arc, OnceLock};

use sha2::{Digest, Sha256};
use tokio_rustls::rustls::SupportedProtocolVersion;
use tokio_rustls::rustls::crypto::{CryptoProvider, ring};
use tokio_rustls::rustls::version::TLS13;

/// The protocol versions both peers accept: TLS 1.3 only.
pub(crate) const VERSIONS: &[&SupportedProtocolVersion] = &[&TLS13];

/// The application protocol, announced through ALPN: HTTP/1.1, which carries
/// the WebSocket upgrade, the web page, and browser downloads.
pub(crate) const ALPN: &[u8] = b"http/1.1";

/// The `ring` crypto provider.
pub(crate) fn provider() -> Arc<CryptoProvider> {
    static PROVIDER: OnceLock<Arc<CryptoProvider>> = OnceLock::new();
    PROVIDER
        .get_or_init(|| Arc::new(ring::default_provider()))
        .clone()
}

/// The SHA-256 fingerprint of a DER-encoded certificate.
pub(crate) fn fingerprint(certificate: &[u8]) -> [u8; 32] {
    Sha256::digest(certificate).into()
}

/// Formats `bytes` as lowercase hexadecimal digits.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_are_sha256_in_lowercase_hex() {
        assert_eq!(hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
        // SHA-256 of the empty input.
        assert_eq!(
            hex(&fingerprint(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
