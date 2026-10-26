use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointKind {
    Unknown,
    Ipv4,
    Ipv6,
    Fqdn,
}

pub fn categorize(endpoint: &str) -> EndpointKind {
    if endpoint.is_empty() || endpoint.contains("://") {
        return EndpointKind::Unknown;
    }

    let host = host_of(endpoint);

    // IPV6 with brackets
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return match inner.parse::<IpAddr>() {
            Ok(ip) if ip.is_ipv6() => EndpointKind::Ipv6,
            _ => EndpointKind::Unknown,
        };
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        return if ip.is_ipv4() {
            EndpointKind::Ipv4
        } else {
            EndpointKind::Ipv6
        };
    }

    // FQDNs have no web url chars like ?, #, @...
    if host.is_empty() || host.contains([' ', '/', '?', '#', '@', ':', '[', ']']) {
        return EndpointKind::Unknown;
    }

    // FQDNs must be printable and contain no spaces
    for c in host.chars() {
        if c.is_control() || c.is_whitespace() {
            return EndpointKind::Unknown;
        }
    }

    EndpointKind::Fqdn
}

fn host_of(endpoint: &str) -> &str {
    if endpoint.starts_with('[') {
        // IPv6 with brackets, possibly followed by a port
        if let Some(end) = endpoint.find(']') {
            return &endpoint[..=end];
        }

        return endpoint;
    }

    match endpoint.rsplit_once(':') {
        Some((host, port))
            if !host.contains(':')
                && !port.is_empty()
                && port.chars().all(|c| c.is_ascii_digit()) =>
        {
            host
        }
        _ => endpoint,
    }
}

pub fn address(endpoint: &str) -> &str {
    let host = host_of(endpoint);
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorizes_endpoints() {
        assert_eq!(categorize(""), EndpointKind::Unknown);
        assert_eq!(categorize("1.2.3.4:51820"), EndpointKind::Ipv4);
        assert_eq!(categorize("1.2.3.4"), EndpointKind::Ipv4);
        assert_eq!(categorize("[2001:db8::1]:51820"), EndpointKind::Ipv6);
        assert_eq!(categorize("2001:db8::1"), EndpointKind::Ipv6);
        assert_eq!(categorize("[not-an-ip]:51820"), EndpointKind::Unknown);
        assert_eq!(categorize("vpn.example.com:51820"), EndpointKind::Fqdn);
        assert_eq!(categorize("vpn.example.com"), EndpointKind::Fqdn);
        assert_eq!(categorize("https://example.com"), EndpointKind::Unknown);
        assert_eq!(categorize("bad host:51820"), EndpointKind::Unknown);
        assert_eq!(categorize("user@host:51820"), EndpointKind::Unknown);
    }

    #[test]
    fn extracts_address_without_port() {
        assert_eq!(address("1.2.3.4:51820"), "1.2.3.4");
        assert_eq!(address("vpn.example.com:51820"), "vpn.example.com");
        assert_eq!(address("[2001:db8::1]:51820"), "2001:db8::1");
        assert_eq!(address("2001:db8::1"), "2001:db8::1");
    }
}
