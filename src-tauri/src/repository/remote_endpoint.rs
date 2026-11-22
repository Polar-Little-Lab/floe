//! HTTPS destination validation shared by local configuration and future transport.
//! This does not resolve DNS, grant network access, or authorize credentials.
use super::Result;
pub(crate) fn https(value: &str) -> Result<tauri::Url> {
    if value.is_empty()
        || value.len() > 4096
        || !value.starts_with("https://")
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
        || value.contains(['\\', '?', '#'])
    {
        return Err("INVALID_HTTPS_REMOTE".into());
    }
    let remainder = &value[8..];
    let (authority, path) = remainder
        .split_once('/')
        .map(|(a, p)| (a, format!("/{p}")))
        .unwrap_or((remainder, "/".into()));
    if authority.ends_with(':') {
        return Err("INVALID_REMOTE_PORT".into());
    }
    if authority.is_empty() || !authority.is_ascii() || authority.contains(['@', '%']) {
        return Err("REMOTE_AUTHORITY_NOT_ALLOWED".into());
    }
    // URL parsers preserve malformed escapes; reject them before a client/server
    // can interpret the same spelling differently. Encoded separators/control
    // characters are not valid repository path components for this transport.
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err("INVALID_REMOTE_ESCAPE".into());
            }
            let digit = |b: u8| (b as char).to_digit(16);
            let decoded = match (digit(bytes[i + 1]), digit(bytes[i + 2])) {
                (Some(a), Some(b)) => (a * 16 + b) as u8,
                _ => return Err("INVALID_REMOTE_ESCAPE".into()),
            };
            if decoded < 0x20 || decoded == 0x7f || decoded == b'/' || decoded == b'\\' {
                return Err("REMOTE_ESCAPE_NOT_ALLOWED".into());
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    let url = tauri::Url::parse(value).map_err(|_| "INVALID_HTTPS_REMOTE")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port() == Some(0)
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("INVALID_HTTPS_REMOTE".into());
    }
    let host = url.host_str().ok_or("INVALID_REMOTE_HOST")?;
    if host.ends_with('.') {
        return Err("NONCANONICAL_REMOTE_HOST".into());
    }
    let raw_host = if authority.starts_with('[') {
        authority.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    if host.parse::<std::net::Ipv4Addr>().is_ok() && raw_host != host {
        return Err("NONCANONICAL_REMOTE_HOST".into());
    }
    // Reject dot segments, including percent-encoded ones, instead of silently
    // granting a destination different from the path that the user supplied.
    for segment in path.split('/') {
        let dots = segment.replace("%2e", ".").replace("%2E", ".");
        if matches!(dots.as_str(), "." | "..") {
            return Err("REMOTE_DOT_SEGMENT".into());
        }
    }
    Ok(url)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn https_endpoints_preserve_destination_and_support_explicit_ports_ipv6_and_encoded_paths() {
        for value in [
            "https://example.invalid/repo.git",
            "https://EXAMPLE.invalid:443/repo.git",
            "https://example.invalid:8443/team/repo.git",
            "https://[2001:db8::1]:8443/r.git",
            "https://example.invalid/team/a%20b.git",
            "https://example.invalid/%ED%95%9C.git",
        ] {
            let url = https(value).unwrap();
            assert_eq!(url.scheme(), "https");
            assert!(url.username().is_empty());
            assert!(url.password().is_none());
            assert!(url.query().is_none());
        }
        assert_eq!(
            https("https://EXAMPLE.invalid:443/repo.git")
                .unwrap()
                .origin()
                .ascii_serialization(),
            "https://example.invalid"
        );
        assert_eq!(
            https("https://example.invalid:8443/r").unwrap().port(),
            Some(8443)
        );
    }
    #[test]
    fn ambiguous_or_secret_bearing_endpoints_refuse_with_redacted_errors() {
        for value in [
            "https://user:secret@example.invalid/r",
            "https://@example.invalid/r",
            "https://example.invalid/r?token=secret",
            "https://example.invalid/r#secret",
            "https://example.invalid\\@other.invalid/r",
            "https:///example.invalid/r",
            "https://example.invalid:/r",
            "https://example.invalid:bad/r",
            "https://example.invalid:70000/r",
            "https://example.invalid:0/r",
            "https://%65xample.invalid/r",
            "https://example.invalid./r",
            "https://2130706433/r",
            "https://127.1/r",
            "https://0x7f000001/r",
            "https://example.invalid/a/../r",
            "https://example.invalid/%2E%2e/r",
            "https://example.invalid/a%2fr",
            "https://example.invalid/a%5Cr",
            "https://example.invalid/a%00r",
            "https://example.invalid/a%0ar",
            "https://example.invalid/a%7fr",
            "https://example.invalid/r%",
            "https://example.invalid/r%GG",
            "https://example.invalid/r\n",
            "http://example.invalid/r",
            "file:///tmp/repo",
            "ssh://host/r",
            "ext::helper",
        ] {
            let error = https(value).unwrap_err();
            assert!(!error.contains("secret"));
            assert!(!error.contains(value));
        }
    }
}
