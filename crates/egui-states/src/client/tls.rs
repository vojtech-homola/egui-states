//! How a native client secures its connection.

use std::fmt;
use std::str::FromStr;

/// How a native client checks the server when it connects with TLS, set with
/// [`ClientBuilder::tls`](crate::ClientBuilder::tls).
///
/// The connection is encrypted in every mode. The modes differ in which
/// servers the client accepts, which decides whether someone on the network
/// can pose as the server.
///
/// The server must have TLS turned on, and a client without TLS cannot
/// connect to it: neither falls back to an unencrypted connection.
///
/// A mode can be parsed from text, for example from a command-line argument:
/// `any`, `system`, or a fingerprint.
///
/// ```
/// use egui_states::Tls;
///
/// assert_eq!("any".parse(), Ok(Tls::AcceptAny));
/// assert_eq!("system".parse(), Ok(Tls::System));
/// let digits = "4f".repeat(32);
/// let pinned = format!("SHA256 Fingerprint={}", "4F:".repeat(31) + "4F");
/// assert_eq!(pinned.parse(), Ok(Tls::Fingerprint(digits)));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Tls {
    /// Accepts any certificate the server presents.
    ///
    /// Nobody can read the traffic by listening, but nothing stops someone on
    /// the network from posing as the server. This suits a server with a
    /// certificate it generated for the session, when its fingerprint cannot
    /// be passed to the client.
    AcceptAny,
    /// Accepts only the certificate with this SHA-256 fingerprint, as the
    /// server reports it (`tls_fingerprint`).
    ///
    /// This protects against listening and against someone posing as the
    /// server, under any name or address the client uses. The fingerprint is
    /// 64 hexadecimal digits; upper case, `:` or space separators, and a
    /// `sha256:` or `SHA256 Fingerprint=` prefix are accepted, so the output of
    /// `openssl x509 -fingerprint -sha256` or a browser's certificate viewer
    /// can be pasted. An invalid fingerprint fails the connection.
    Fingerprint(String),
    /// Accepts certificates the operating system trusts, for the host the
    /// client connects to.
    ///
    /// This needs a certificate issued by an authority installed on this
    /// computer, and naming the host the client connects to (see
    /// [`ClientBuilder::host`](crate::ClientBuilder::host)). A certificate the
    /// server generated is never trusted.
    System,
}

impl FromStr for Tls {
    type Err = String;

    /// Parses `any`, `system` (in any case), or a fingerprint.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("any") {
            return Ok(Self::AcceptAny);
        }
        if text.eq_ignore_ascii_case("system") {
            return Ok(Self::System);
        }
        normalize_fingerprint(text).map(Self::Fingerprint)
    }
}

impl fmt::Display for Tls {
    /// Writes the text [`Tls::from_str`] parses back.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AcceptAny => f.write_str("any"),
            Self::System => f.write_str("system"),
            Self::Fingerprint(fingerprint) => f.write_str(fingerprint),
        }
    }
}

/// Reduces a fingerprint to 64 lowercase hexadecimal digits.
fn normalize_fingerprint(text: &str) -> Result<String, String> {
    let invalid = || {
        format!(
            "invalid TLS mode or fingerprint {text:?}: expected `any`, `system`, or a SHA-256 \
             fingerprint of 64 hexadecimal digits"
        )
    };
    let mut digits = text.trim();
    // `SHA256 Fingerprint=AB:CD...`, as `openssl x509 -fingerprint` prints it.
    if let Some((_, value)) = digits.rsplit_once('=') {
        digits = value;
    }
    if let Some(prefix) = digits.get(..7)
        && prefix.eq_ignore_ascii_case("sha256:")
    {
        digits = &digits[7..];
    }
    let digits: String = digits
        .chars()
        .filter(|c| *c != ':' && !c.is_whitespace())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if digits.len() == 64 && digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(digits)
    } else {
        Err(invalid())
    }
}

/// Parses a fingerprint into the bytes of the SHA-256 hash.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn parse_fingerprint(text: &str) -> Result<[u8; 32], String> {
    let digits = normalize_fingerprint(text)?;
    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&digits[index * 2..index * 2 + 2], 16)
            .map_err(|error| error.to_string())?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGITS: &str = "4f2a0011223344556677889900aabbccddeeff00112233445566778899aabbcc";

    #[test]
    fn modes_parse_from_text() {
        assert_eq!("any".parse(), Ok(Tls::AcceptAny));
        assert_eq!(" System ".parse(), Ok(Tls::System));
        let upper_with_colons = DIGITS
            .as_bytes()
            .chunks(2)
            .map(|pair| std::str::from_utf8(pair).unwrap().to_ascii_uppercase())
            .collect::<Vec<_>>()
            .join(":");
        for text in [
            DIGITS.to_string(),
            upper_with_colons.clone(),
            upper_with_colons.replace(':', " "),
            format!("sha256:{DIGITS}"),
            format!("SHA256 Fingerprint={upper_with_colons}"),
            format!("sha256 Fingerprint={DIGITS}\n"),
        ] {
            assert_eq!(
                text.parse(),
                Ok(Tls::Fingerprint(DIGITS.to_string())),
                "{text}"
            );
        }
        for text in [
            "",
            "none",
            &DIGITS[2..],
            &format!("{DIGITS}00"),
            &DIGITS.replace('a', "g"),
        ] {
            let error = text.parse::<Tls>().unwrap_err();
            assert!(error.contains("64 hexadecimal digits"), "{error}");
        }
        for mode in [Tls::AcceptAny, Tls::System, Tls::Fingerprint(DIGITS.into())] {
            assert_eq!(mode.to_string().parse(), Ok(mode));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn fingerprints_parse_to_hash_bytes() {
        let bytes = parse_fingerprint(&DIGITS.to_ascii_uppercase()).unwrap();
        assert_eq!(bytes[..3], [0x4f, 0x2a, 0x00]);
        assert_eq!(bytes[31], 0xcc);
        assert!(parse_fingerprint("4f2a").is_err());
    }
}
