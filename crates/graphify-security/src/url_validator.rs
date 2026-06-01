//! URL validation and SSRF prevention.

use url::Url;

use crate::SecurityError;

/// Maximum fetch size: 50 MB.
pub const MAX_FETCH_SIZE: usize = 50 * 1024 * 1024;

/// Maximum "safe" size for in-memory processing: 10 MB.
pub const MAX_SAFE_SIZE: usize = 10 * 1024 * 1024;

/// Validate a URL: must be http/https, must not resolve to private/localhost IPs.
///
/// Note: this is a static check only. It does not protect against DNS rebinding
/// attacks where a public hostname resolves to a private IP at request time.
/// For full SSRF protection, also check the resolved IP after DNS lookup.
pub fn validate_url(url_str: &str) -> Result<Url, SecurityError> {
    let url = Url::parse(url_str)?;

    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(SecurityError::InvalidScheme(url.scheme().to_string()));
    }

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
    if host == "localhost" {
        return true;
    }

    if let Ok(ip) = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<std::net::IpAddr>()
    {
        return ip_is_private(&ip);
    }

    if let Some(ipv4) = parse_nonstandard_ipv4(host) {
        return ip_is_private(&std::net::IpAddr::V4(ipv4));
    }

    false
}

/// Check if an IP address is private, loopback, link-local, or reserved.
fn ip_is_private(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || is_in_range(
                    v4,
                    &std::net::Ipv4Addr::new(100, 64, 0, 0),
                    &std::net::Ipv4Addr::new(100, 127, 255, 255),
                )
                || is_in_range(
                    v4,
                    &std::net::Ipv4Addr::new(198, 18, 0, 0),
                    &std::net::Ipv4Addr::new(198, 19, 255, 255),
                )
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || matches!(v6.octets()[0] & 0xfe, 0xfc)
                || matches!(v6.octets()[0], 0xfe) && matches!(v6.octets()[1] & 0xc0, 0x80)
        }
    }
}

fn is_in_range(
    ip: &std::net::Ipv4Addr,
    start: &std::net::Ipv4Addr,
    end: &std::net::Ipv4Addr,
) -> bool {
    let ip_u32 = u32::from(*ip);
    ip_u32 >= u32::from(*start) && ip_u32 <= u32::from(*end)
}

/// Try parsing non-standard IPv4 representations (decimal, hex, octal).
fn parse_nonstandard_ipv4(host: &str) -> Option<std::net::Ipv4Addr> {
    if let Ok(num) = host.parse::<u32>() {
        return Some(std::net::Ipv4Addr::from(num));
    }
    if let Some(hex) = host.strip_prefix("0x").or_else(|| host.strip_prefix("0X"))
        && let Ok(num) = u32::from_str_radix(hex, 16)
    {
        return Some(std::net::Ipv4Addr::from(num));
    }
    None
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
