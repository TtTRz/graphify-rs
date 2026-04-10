//! URL validation and SSRF prevention.

use url::Url;

use crate::SecurityError;

/// Maximum fetch size: 50 MB.
pub const MAX_FETCH_SIZE: usize = 50 * 1024 * 1024;

/// Maximum "safe" size for in-memory processing: 10 MB.
pub const MAX_SAFE_SIZE: usize = 10 * 1024 * 1024;

/// Validate a URL: must be http/https, must not resolve to private/localhost IPs.
///
/// Returns the parsed [`Url`] on success.
pub fn validate_url(url_str: &str) -> Result<Url, SecurityError> {
    let url = Url::parse(url_str)?;

    // Only allow http/https
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(SecurityError::InvalidScheme(url.scheme().to_string()));
    }

    // Block private/reserved IPs
    if let Some(host) = url.host_str() {
        if is_private_host(host) {
            return Err(SecurityError::PrivateIp(host.to_string()));
        }
    } else {
        return Err(SecurityError::PrivateIp("(no host)".to_string()));
    }

    Ok(url)
}

/// Check whether a host string refers to a private or reserved address.
fn is_private_host(host: &str) -> bool {
    // Exact matches
    if host == "localhost" || host == "::1" || host == "[::1]" {
        return true;
    }

    // Prefix-based checks for IPv4 private/reserved ranges
    if host.starts_with("127.")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("169.254.")
        || host.starts_with("0.")
    {
        return true;
    }

    // 172.16.0.0 – 172.31.255.255
    if is_172_private(host) {
        return true;
    }

    false
}

/// Check whether a host falls in the 172.16.0.0/12 private range.
fn is_172_private(host: &str) -> bool {
    if let Some(rest) = host.strip_prefix("172.")
        && let Some(second_octet_str) = rest.split('.').next()
            && let Ok(second_octet) = second_octet_str.parse::<u8>() {
                return (16..=31).contains(&second_octet);
            }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_https_url() {
        let result = validate_url("https://example.com/page");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().host_str(), Some("example.com"));
    }

    #[test]
    fn test_valid_http_url() {
        let result = validate_url("http://example.com");
        assert!(result.is_ok());
    }

    #[test]
    fn test_reject_ftp_scheme() {
        let result = validate_url("ftp://example.com/file");
        assert!(matches!(result, Err(SecurityError::InvalidScheme(_))));
    }

    #[test]
    fn test_reject_file_scheme() {
        let result = validate_url("file:///etc/passwd");
        assert!(matches!(result, Err(SecurityError::InvalidScheme(_))));
    }

    #[test]
    fn test_reject_javascript_scheme() {
        let result = validate_url("javascript:alert(1)");
        assert!(result.is_err());
    }

    #[test]
    fn test_reject_localhost() {
        let result = validate_url("http://localhost:8080/api");
        assert!(matches!(result, Err(SecurityError::PrivateIp(_))));
    }

    #[test]
    fn test_reject_127() {
        let result = validate_url("http://127.0.0.1/admin");
        assert!(matches!(result, Err(SecurityError::PrivateIp(_))));
    }

    #[test]
    fn test_reject_10_network() {
        let result = validate_url("http://10.0.0.1/internal");
        assert!(matches!(result, Err(SecurityError::PrivateIp(_))));
    }

    #[test]
    fn test_reject_192_168() {
        let result = validate_url("http://192.168.1.1/router");
        assert!(matches!(result, Err(SecurityError::PrivateIp(_))));
    }

    #[test]
    fn test_reject_172_16() {
        let result = validate_url("http://172.16.0.1/secret");
        assert!(matches!(result, Err(SecurityError::PrivateIp(_))));
    }

    #[test]
    fn test_reject_172_31() {
        let result = validate_url("http://172.31.255.255/secret");
        assert!(matches!(result, Err(SecurityError::PrivateIp(_))));
    }

    #[test]
    fn test_allow_172_32() {
        let result = validate_url("http://172.32.0.1/public");
        assert!(result.is_ok());
    }

    #[test]
    fn test_reject_link_local() {
        let result = validate_url("http://169.254.169.254/latest/meta-data/");
        assert!(matches!(result, Err(SecurityError::PrivateIp(_))));
    }

    #[test]
    fn test_reject_ipv6_loopback() {
        let result = validate_url("http://[::1]/");
        assert!(matches!(result, Err(SecurityError::PrivateIp(_))));
    }

    #[test]
    fn test_reject_zero_ip() {
        let result = validate_url("http://0.0.0.0/");
        assert!(matches!(result, Err(SecurityError::PrivateIp(_))));
    }

    #[test]
    fn test_invalid_url() {
        let result = validate_url("not a url at all");
        assert!(result.is_err());
    }

    #[test]
    fn test_constants() {
        assert_eq!(MAX_FETCH_SIZE, 50 * 1024 * 1024);
        assert_eq!(MAX_SAFE_SIZE, 10 * 1024 * 1024);
    }
}
