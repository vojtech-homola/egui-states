//! Rejects WebSocket connections opened by web pages of other origins.
//!
//! A browser lets any page open a WebSocket to any address, including this
//! server on the user's own machine, but it always states the page's origin
//! (`scheme://host[:port]`) in the `Origin` header, which the page cannot
//! change or omit. Native clients send no `Origin` and are not affected; the
//! token is what guards against other programs.
//!
//! A page may connect when it was loaded from the server itself, so its origin
//! equals the address the browser connected to, and that address is an IP
//! address or a name that only exists on the local network, such as
//! `localhost` or `pc-a.local`. Requiring such an address stops DNS rebinding,
//! where an attacker's domain is made to resolve to this machine so that its
//! origin and address agree. Other pages need their origin allowed explicitly.

use std::net::IpAddr;

use tokio_tungstenite::tungstenite::http::{HeaderMap, header};

/// Set by proxies such as `trunk serve` to the address the browser used.
const X_FORWARDED_HOST: &str = "x-forwarded-host";
/// Set by proxies to the scheme the browser used.
const X_FORWARDED_PROTO: &str = "x-forwarded-proto";

/// Which browser pages may open a WebSocket to the server.
#[derive(Debug, Default)]
pub(crate) struct OriginPolicy {
    allowed: Vec<OriginPattern>,
    /// Set by a `*` entry: every page may connect.
    any: bool,
}

impl OriginPolicy {
    /// Parses the allowed origin patterns.
    ///
    /// A pattern is `scheme://host[:port]`. `*` stands for any scheme, any
    /// run of characters in the host (`*.example.com`), or any port (`:*`).
    /// Without a port, the scheme's default port is meant, as in browsers.
    /// A pattern of just `*` allows every page, including local files.
    pub(crate) fn new(patterns: &[String]) -> Result<Self, String> {
        let any = patterns.iter().any(|text| text == "*");
        let allowed = patterns
            .iter()
            .filter(|text| *text != "*")
            .map(|text| {
                OriginPattern::parse(text).ok_or_else(|| {
                    format!("invalid allowed origin {text:?}: expected scheme://host[:port]")
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { allowed, any })
    }

    /// Checks the headers of a WebSocket upgrade request.
    ///
    /// Returns the reason when a browser page may not connect.
    pub(crate) fn check(&self, headers: &HeaderMap) -> Result<(), String> {
        if self.any {
            return Ok(());
        }
        let mut values = headers.get_all(header::ORIGIN).iter();
        let Some(value) = values.next() else {
            // Not a browser.
            return Ok(());
        };
        if values.next().is_some() {
            return Err("the request has several Origin headers".to_string());
        }
        let text = value
            .to_str()
            .map_err(|_| "the Origin header is not plain text".to_string())?;
        let rejected = || format!("a page from {text} may not connect; list it in allowed_origins");
        // Includes "null", sent for local files and sandboxed frames.
        let Some(origin) = Origin::parse(text) else {
            return Err(rejected());
        };
        if self.allowed.iter().any(|pattern| pattern.matches(&origin)) {
            return Ok(());
        }

        let served = served_origin(headers).ok_or_else(rejected)?;
        if origin.scheme != served.scheme
            || origin.host != served.host
            || origin.effective_port() != served.effective_port()
            || !is_local_name_or_address(&served.host)
        {
            return Err(rejected());
        }
        Ok(())
    }
}

/// Suffixes reserved for names on the local network, which no one can
/// register on the internet: mDNS (`.local`), home networks (`.home.arpa`),
/// private networks (`.internal`), and the local machine (`.localhost`).
const LOCAL_SUFFIXES: &[&str] = &[".local", ".home.arpa", ".internal", ".localhost"];

/// Whether `host` is an IP address or a local network name, which an
/// attacker's page on the internet cannot have as its own origin.
///
/// Local names are names without a dot (such as `localhost` or `pc-a`),
/// which public DNS does not allow, and names under [`LOCAL_SUFFIXES`].
fn is_local_name_or_address(host: &str) -> bool {
    let address = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    if address.parse::<IpAddr>().is_ok() {
        return true;
    }
    // A trailing dot only marks the name as fully qualified.
    let name = host.strip_suffix('.').unwrap_or(host);
    if name.is_empty() || name.starts_with('.') || name.contains("..") {
        return false;
    }
    !name.contains('.') || LOCAL_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
}

/// The origin of the address the browser connected to.
///
/// A browser cannot set headers on a WebSocket request, so `X-Forwarded-*`
/// headers were added by a proxy and name the address and scheme the browser
/// used; the `Host` header then names the proxy's backend instead. Without
/// them the browser reached this server directly, which speaks plain HTTP.
fn served_origin(headers: &HeaderMap) -> Option<Origin> {
    // Chained proxies append; the first entry is the browser's.
    let first = |name| -> Option<&str> {
        Some(headers.get(name)?.to_str().ok()?.split(',').next()?.trim())
    };
    let address = first(X_FORWARDED_HOST).or_else(|| first(header::HOST.as_str()))?;
    let scheme = first(X_FORWARDED_PROTO)
        .unwrap_or("http")
        .to_ascii_lowercase();
    // Some proxies, such as Traefik, report a WebSocket request's scheme as
    // `ws` or `wss`; the page that opened it was loaded over HTTP or HTTPS.
    let scheme = match scheme.as_str() {
        "ws" => "http".to_string(),
        "wss" => "https".to_string(),
        _ => scheme,
    };
    let (host, port) = split_authority(address)?;
    Some(Origin { scheme, host, port })
}

/// The port a browser uses for `scheme` when the origin names none.
fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        _ => None,
    }
}

/// A parsed `scheme://host[:port]` origin.
struct Origin {
    scheme: String,
    host: String,
    port: Option<u16>,
}

impl Origin {
    fn parse(text: &str) -> Option<Self> {
        let (scheme, authority) = split_scheme(text)?;
        if scheme == "*" {
            return None;
        }
        let (host, port) = split_authority(authority)?;
        Some(Self { scheme, host, port })
    }

    /// The port, with the scheme's default filled in.
    fn effective_port(&self) -> Option<u16> {
        self.port.or_else(|| default_port(&self.scheme))
    }
}

/// An allowed origin, possibly with wildcards.
#[derive(Debug)]
struct OriginPattern {
    /// `None` for any scheme.
    scheme: Option<String>,
    /// Lowercase host, where `*` matches any run of characters.
    host: String,
    port: PortPattern,
}

#[derive(Debug)]
enum PortPattern {
    Any,
    /// The scheme's default port.
    Default,
    Exact(u16),
}

impl OriginPattern {
    fn parse(text: &str) -> Option<Self> {
        // Tolerate the trailing slash of a copied page address.
        let text = text.strip_suffix('/').unwrap_or(text);
        let (scheme, authority) = split_scheme(text)?;
        let (host, port) = match authority.strip_suffix(":*") {
            Some(host) => (split_authority(host)?.0, PortPattern::Any),
            None => match split_authority(authority)? {
                (host, Some(port)) => (host, PortPattern::Exact(port)),
                (host, None) => (host, PortPattern::Default),
            },
        };
        Some(Self {
            scheme: (scheme != "*").then_some(scheme),
            host,
            port,
        })
    }

    fn matches(&self, origin: &Origin) -> bool {
        let scheme = self
            .scheme
            .as_ref()
            .is_none_or(|scheme| *scheme == origin.scheme);
        let port = match self.port {
            PortPattern::Any => true,
            PortPattern::Default => origin.effective_port() == default_port(&origin.scheme),
            PortPattern::Exact(port) => origin.effective_port() == Some(port),
        };
        scheme && port && wildcard_match(&self.host, &origin.host)
    }
}

/// Matches `text` against `pattern`, where `*` matches any run of characters.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let mut parts = parts.peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            // The last part must end the text.
            return rest.ends_with(part);
        }
        match rest.find(part) {
            Some(index) => rest = &rest[index + part.len()..],
            None => return false,
        }
    }
    // No `*` at all: the whole text must have matched.
    rest.is_empty()
}

/// Splits `scheme://authority`, lowercasing the scheme. The scheme may be `*`.
fn split_scheme(text: &str) -> Option<(String, &str)> {
    let (scheme, authority) = text.split_once("://")?;
    let valid = scheme == "*"
        || (scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')));
    valid.then(|| (scheme.to_ascii_lowercase(), authority))
}

/// Splits `host[:port]` into a lowercase host and the port.
///
/// IPv6 addresses keep their brackets. Anything with a path, user info, or
/// an invalid port is rejected.
fn split_authority(authority: &str) -> Option<(String, Option<u16>)> {
    if authority.is_empty()
        || authority.contains(|c: char| matches!(c, '/' | '?' | '#' | '@') || c.is_whitespace())
    {
        return None;
    }
    let (host, port) = match authority.strip_prefix('[') {
        Some(rest) => {
            let end = rest.find(']')?;
            let host = &authority[..end + 2];
            match &rest[end + 1..] {
                "" => (host, None),
                port => (host, Some(port.strip_prefix(':')?)),
            }
        }
        None => match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        },
    };
    if host.is_empty() || (host.contains(':') && !host.starts_with('[')) {
        return None;
    }
    let port = match port {
        Some(port) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
            Some(port.parse().ok()?)
        }
        Some(_) => return None,
        None => None,
    };
    Some((host.to_ascii_lowercase(), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.append(
                header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        headers
    }

    fn policy(patterns: &[&str]) -> OriginPolicy {
        let patterns = patterns.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        OriginPolicy::new(&patterns).unwrap()
    }

    #[test]
    fn same_origin_pages_on_local_addresses_and_native_clients_connect() {
        let default = policy(&[]);
        for request in [
            // Native client.
            vec![("Host", "127.0.0.1:8091")],
            vec![
                ("Host", "127.0.0.1:8091"),
                ("Origin", "http://127.0.0.1:8091"),
            ],
            vec![
                ("Host", "LocalHost:8091"),
                ("Origin", "http://localhost:8091"),
            ],
            vec![
                ("Host", "192.168.1.5:8091"),
                ("Origin", "http://192.168.1.5:8091"),
            ],
            vec![("Host", "[::1]:8091"), ("Origin", "http://[::1]:8091")],
            // Names that exist only on the local network.
            vec![
                ("Host", "pc-a.local:8091"),
                ("Origin", "http://pc-a.local:8091"),
            ],
            vec![("Host", "PC-A:8091"), ("Origin", "http://pc-a:8091")],
            vec![
                ("Host", "nas.home.arpa:8091"),
                ("Origin", "http://nas.home.arpa:8091"),
            ],
            vec![
                ("Host", "box.internal:8091"),
                ("Origin", "http://box.internal:8091"),
            ],
            vec![
                ("Host", "app.localhost:8091"),
                ("Origin", "http://app.localhost:8091"),
            ],
            vec![
                ("Host", "pc-a.local.:8091"),
                ("Origin", "http://pc-a.local.:8091"),
            ],
            // Default port, written or not.
            vec![("Host", "localhost"), ("Origin", "http://localhost")],
            vec![("Host", "localhost:80"), ("Origin", "http://localhost")],
            // `trunk serve` forwarding a page it served on 8090.
            vec![
                ("Host", "127.0.0.1"),
                ("X-Forwarded-Host", "localhost:8090"),
                ("X-Forwarded-Proto", "http"),
                ("Origin", "http://localhost:8090"),
            ],
            // A TLS proxy on the default port.
            vec![
                ("Host", "127.0.0.1:8091"),
                ("X-Forwarded-Host", "localhost"),
                ("X-Forwarded-Proto", "https"),
                ("Origin", "https://localhost"),
            ],
            // Proxies that name the WebSocket scheme instead, like Traefik.
            vec![
                ("Host", "127.0.0.1:8091"),
                ("X-Forwarded-Host", "pc-a.local"),
                ("X-Forwarded-Proto", "wss"),
                ("Origin", "https://pc-a.local"),
            ],
            vec![
                ("Host", "127.0.0.1:8091"),
                ("X-Forwarded-Host", "pc-a.local:8090"),
                ("X-Forwarded-Proto", "WS"),
                ("Origin", "http://pc-a.local:8090"),
            ],
        ] {
            assert_eq!(default.check(&headers(&request)), Ok(()), "{request:?}");
        }
    }

    #[test]
    fn foreign_pages_are_rejected() {
        let default = policy(&[]);
        for request in [
            vec![
                ("Host", "127.0.0.1:8091"),
                ("Origin", "https://news.example"),
            ],
            vec![
                ("Host", "127.0.0.1:8091"),
                ("Origin", "http://127.0.0.1:8090"),
            ],
            vec![
                ("Host", "127.0.0.1:8091"),
                ("Origin", "http://localhost:8091"),
            ],
            vec![("Host", "127.0.0.1:8091"), ("Origin", "null")],
            vec![("Origin", "http://127.0.0.1:8091")],
            vec![
                ("Host", "127.0.0.1:8091"),
                ("Origin", "http://127.0.0.1:8091"),
                ("Origin", "http://127.0.0.1:8091"),
            ],
            // The server speaks plain HTTP, so an HTTPS page is another origin.
            vec![("Host", "localhost"), ("Origin", "https://localhost")],
            // An HTTP page on port 80 is not the HTTPS page a TLS proxy serves.
            vec![
                ("Host", "127.0.0.1:8091"),
                ("X-Forwarded-Host", "localhost"),
                ("X-Forwarded-Proto", "https"),
                ("Origin", "http://localhost"),
            ],
            vec![
                ("Host", "127.0.0.1:8091"),
                ("X-Forwarded-Host", "localhost"),
                ("X-Forwarded-Proto", "wss"),
                ("Origin", "http://localhost"),
            ],
            // DNS rebinding: an attacker's name resolved to this machine.
            vec![
                ("Host", "evil.example:8091"),
                ("Origin", "http://evil.example:8091"),
            ],
            vec![
                ("Host", "127.0.0.1"),
                ("X-Forwarded-Host", "evil.example:8090"),
                ("Origin", "http://evil.example:8090"),
            ],
            // A server reached by an internet name needs that name allowed.
            vec![
                ("Host", "pc-a.corp.example:8091"),
                ("Origin", "http://pc-a.corp.example:8091"),
            ],
            // Lookalikes of local suffixes are internet names.
            vec![
                ("Host", "evil-local.example:8091"),
                ("Origin", "http://evil-local.example:8091"),
            ],
            vec![
                ("Host", "local.example:8091"),
                ("Origin", "http://local.example:8091"),
            ],
        ] {
            let error = default.check(&headers(&request)).unwrap_err();
            assert!(
                error.contains("allowed_origins") || error.contains("several"),
                "{error}"
            );
        }
    }

    #[test]
    fn allowed_origins_match_patterns_and_default_ports() {
        let configured = policy(&[
            "HTTP://Dev.Example:8090/",
            "http://pc-a.local:*",
            "https://*.example.org",
            "*://tools.local:9000",
            "http://plain.local:80",
        ]);
        let from = |origin| headers(&[("Host", "127.0.0.1:8091"), ("Origin", origin)]);
        for origin in [
            "http://dev.example:8090",
            "http://pc-a.local:8091",
            "http://pc-a.local",
            "https://app.example.org",
            "https://app.example.org:443",
            "http://tools.local:9000",
            "https://tools.local:9000",
            // An explicit default port matches the origin browsers send.
            "http://plain.local",
        ] {
            assert_eq!(configured.check(&from(origin)), Ok(()), "{origin}");
        }
        for origin in [
            "http://dev.example:8091",
            "https://pc-a.local:8091",
            "https://example.org",
            "https://app.example.org:8443",
            "http://app.example.org",
            "http://tools.local",
            "http://plain.local:8080",
        ] {
            assert!(configured.check(&from(origin)).is_err(), "{origin}");
        }
    }

    #[test]
    fn a_star_allows_every_page() {
        let any = policy(&["http://other.example", "*"]);
        for origin in ["https://news.example", "null", "http://evil.example:8091"] {
            let request = headers(&[("Host", "127.0.0.1:8091"), ("Origin", origin)]);
            assert_eq!(any.check(&request), Ok(()), "{origin}");
        }
        // A star inside a pattern is still only a wildcard.
        let request = headers(&[("Host", "127.0.0.1:8091"), ("Origin", "null")]);
        assert!(policy(&["*://*:*"]).check(&request).is_err());
    }

    #[test]
    fn local_names_and_addresses_are_recognized() {
        for (host, expected) in [
            ("127.0.0.1", true),
            ("[::1]", true),
            ("localhost", true),
            ("pc-a", true),
            ("pc-a.local", true),
            ("pc-a.local.", true),
            ("nas.home.arpa", true),
            ("box.internal", true),
            ("app.localhost", true),
            ("example.com", false),
            ("local", true),
            ("evil.local.example", false),
            ("evillocal", true),
            ("evil-local.example", false),
            (".local", false),
            ("pc..local", false),
            ("[evil.example]", false),
        ] {
            assert_eq!(is_local_name_or_address(host), expected, "{host}");
        }
    }

    #[test]
    fn wildcards_match_any_run_of_characters() {
        for (pattern, text, expected) in [
            ("a.example", "a.example", true),
            ("a.example", "a.example.org", false),
            ("*", "anything", true),
            ("*.example", "a.b.example", true),
            ("*.example", "example", false),
            ("a*c", "abbc", true),
            ("a*c", "abcd", false),
            ("*b*", "abc", true),
        ] {
            assert_eq!(wildcard_match(pattern, text), expected, "{pattern} {text}");
        }
    }

    #[test]
    fn invalid_patterns_are_rejected() {
        for pattern in [
            "localhost:8090",
            "http://",
            "http://host/path",
            "null",
            "http://h:x",
            "",
        ] {
            assert!(
                OriginPolicy::new(&[pattern.to_string()]).is_err(),
                "{pattern}"
            );
        }
    }
}
