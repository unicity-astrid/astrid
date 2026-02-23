#[cfg(feature = "reqwest")]
/// A DNS resolver that prevents SSRF by blocking resolution to local, private, or multicast IP addresses.
#[derive(Clone)]
pub struct SafeDnsResolver;

#[cfg(feature = "reqwest")]
impl reqwest::dns::Resolve for SafeDnsResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let name_str = name.as_str().to_string();
        Box::pin(async move {
            let addrs = tokio::net::lookup_host((name_str.as_str(), 0))
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

            let mut safe_addrs = Vec::new();
            for addr in addrs {
                if is_safe_ip(addr.ip()) {
                    safe_addrs.push(addr);
                }
            }

            if safe_addrs.is_empty() {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "DNS resolved to an unauthorized private or local IP address",
                ))
                    as Box<dyn std::error::Error + Send + Sync>);
            }

            let iter: reqwest::dns::Addrs = Box::new(safe_addrs.into_iter());
            Ok(iter)
        })
    }
}

/// Checks if an IP address is safe to connect to (not local, private, or multicast).
#[must_use]
pub fn is_safe_ip(mut ip: std::net::IpAddr) -> bool {
    // Escape hatch for integration tests that need to spin up local servers
    if std::env::var("ASTRID_TEST_ALLOW_LOCAL_IP").is_ok() {
        return true;
    }

    // Global escape hatch for deployments that require plugins to access internal network services
    if std::env::var("ASTRID_ALLOW_LOCAL_IPS").is_ok() {
        return true;
    }

    if let std::net::IpAddr::V6(ipv6) = ip {
        if let Some(ipv4) = ipv6.to_ipv4_mapped() {
            ip = std::net::IpAddr::V4(ipv4);
        } else if let Some(ipv4) = ipv6.to_ipv4() {
            // Also handle IPv4-compatible IPv6 addresses
            ip = std::net::IpAddr::V4(ipv4);
        }
    }

    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }

    match ip {
        std::net::IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            let is_private = octets[0] == 10 ||
                octets[0] == 0 || // 0.0.0.0/8
                octets[0] == 255 || // Broadcast
                (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31) ||
                (octets[0] == 192 && octets[1] == 168) ||
                (octets[0] == 169 && octets[1] == 254) ||
                (octets[0] == 100 && octets[1] >= 64 && octets[1] <= 127) ||
                octets[0] == 127;
            !is_private
        },
        std::net::IpAddr::V6(ipv6) => {
            let segments = ipv6.segments();
            let is_private = (segments[0] & 0xfe00) == 0xfc00 || (segments[0] & 0xffc0) == 0xfe80;
            !is_private
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;
    use std::str::FromStr;

    #[test]
    fn test_is_safe_ip() {
        // Safe IPs
        assert!(is_safe_ip(IpAddr::from_str("8.8.8.8").unwrap()));
        assert!(is_safe_ip(IpAddr::from_str("1.1.1.1").unwrap()));
        assert!(is_safe_ip(IpAddr::from_str("198.51.100.1").unwrap()));
        assert!(is_safe_ip(
            IpAddr::from_str("2001:4860:4860::8888").unwrap()
        ));

        // Loopback / Unspecified
        assert!(!is_safe_ip(IpAddr::from_str("127.0.0.1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("::1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("0.0.0.0").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("::").unwrap()));

        // 0.0.0.0/8 block
        assert!(!is_safe_ip(IpAddr::from_str("0.0.0.1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("0.255.255.255").unwrap()));

        // Private IPv4 (RFC 1918)
        assert!(!is_safe_ip(IpAddr::from_str("10.0.0.1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("10.255.255.255").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("172.16.0.1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("172.31.255.255").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("192.168.0.1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("192.168.255.255").unwrap()));

        // Link-local / CGNAT
        assert!(!is_safe_ip(IpAddr::from_str("169.254.169.254").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("100.64.0.1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("100.127.255.255").unwrap()));

        // Private IPv6 (Unique Local, Link Local)
        assert!(!is_safe_ip(IpAddr::from_str("fc00::1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("fd00::1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("fe80::1").unwrap()));

        // IPv4-mapped IPv6 bypassing traditional checks
        assert!(!is_safe_ip(IpAddr::from_str("::ffff:127.0.0.1").unwrap()));
        assert!(!is_safe_ip(IpAddr::from_str("::ffff:10.0.0.1").unwrap()));
        assert!(!is_safe_ip(
            IpAddr::from_str("::ffff:169.254.169.254").unwrap()
        ));
    }
}
