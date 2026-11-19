//! Keeping a tool that takes a model-chosen address from becoming a way into the private network (ADR-0021).
//!
//! The name is resolved here, every answer is checked, and the connection is made to the address that was checked, so a second
//! DNS answer cannot rebind it. Redirects are followed by hand, each hop checked again.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// Why an address is off limits, or `None` when it is an ordinary public one.
pub fn forbidden(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(v4) => forbidden_v4(v4),
        IpAddr::V6(v6) => forbidden_v6(v6),
    }
}

fn forbidden_v4(ip: Ipv4Addr) -> Option<&'static str> {
    let [a, b, ..] = ip.octets();
    if ip.is_loopback() {
        Some("a loopback address (this computer)")
    } else if ip.is_unspecified() || a == 0 {
        Some("an unspecified address")
    } else if ip.is_private() {
        Some("a private network address")
    } else if ip.is_link_local() {
        Some("a link-local address (cloud metadata services live here)")
    } else if a == 100 && (64..=127).contains(&b) {
        Some("a shared (carrier-grade NAT) address")
    } else if ip.is_broadcast() || ip.is_multicast() || a >= 240 {
        Some("a broadcast, multicast or reserved address")
    } else if (a == 192 && b == 0 && ip.octets()[2] == 0) || (a == 198 && (b == 18 || b == 19)) || ip.is_documentation()
    {
        Some("a reserved or documentation address")
    } else {
        None
    }
}

fn forbidden_v6(ip: Ipv6Addr) -> Option<&'static str> {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return forbidden_v4(v4);
    }
    let segments = ip.segments();
    // 64:ff9b::/96 carries an IPv4 address in its last 32 bits.
    if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        let [hi, lo] = [segments[6], segments[7]];
        return forbidden_v4(Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8));
    }
    if ip.is_loopback() {
        Some("a loopback address (this computer)")
    } else if ip.is_unspecified() {
        Some("an unspecified address")
    } else if segments[0] & 0xfe00 == 0xfc00 {
        Some("a private network address")
    } else if segments[0] & 0xffc0 == 0xfe80 || segments[0] & 0xffc0 == 0xfec0 {
        Some("a link-local address")
    } else if ip.is_multicast() {
        Some("a multicast address")
    } else if segments[0] == 0x2001 && segments[1] == 0x0db8 {
        Some("a documentation address")
    } else {
        None
    }
}

/// What went wrong finding where to connect.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NetError {
    /// The name does not exist or could not be looked up.
    #[error("{0} could not be found.")]
    NotFound(String),
    /// The name or address leads somewhere a tool may not go.
    #[error("{host} leads to {why} ({ip}), which tools may not reach unless that exact host is granted.")]
    Private {
        /// The host asked for.
        host: String,
        /// The address it resolved to.
        ip: IpAddr,
        /// What kind of address it is.
        why: &'static str,
    },
}

/// Find the addresses to connect to for `host`. A host whose any answer is off limits is refused (a name that resolves to both a
/// public and a private address is how rebinding works), unless `private_ok` says a person granted this exact host.
pub async fn resolve(host: &str, port: u16, private_ok: bool) -> Result<Vec<SocketAddr>, NetError> {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let addresses: Vec<SocketAddr> = match bare.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => {
            tokio::net::lookup_host((bare, port)).await.map_err(|_| NetError::NotFound(host.to_string()))?.collect()
        }
    };
    if addresses.is_empty() {
        return Err(NetError::NotFound(host.to_string()));
    }
    if !private_ok && let Some((address, why)) = addresses.iter().find_map(|a| forbidden(a.ip()).map(|why| (a, why))) {
        return Err(NetError::Private { host: host.to_string(), ip: address.ip(), why });
    }
    Ok(addresses)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bad(text: &str) -> &'static str {
        forbidden(text.parse().unwrap()).unwrap_or_else(|| panic!("{text} should be off limits"))
    }

    fn fine(text: &str) {
        assert!(forbidden(text.parse().unwrap()).is_none(), "{text} is an ordinary address");
    }

    #[test]
    fn every_kind_of_private_or_special_address_is_off_limits() {
        for text in [
            "127.0.0.1",
            "127.9.9.9",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "0.1.2.3",
            "100.64.0.1",
            "100.127.255.255",
            "255.255.255.255",
            "224.0.0.1",
            "240.0.0.1",
            "198.18.0.1",
            "192.0.2.1",
            "203.0.113.5",
            "::1",
            "::",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
            "::ffff:10.1.2.3",
            "::ffff:169.254.169.254",
            "64:ff9b::7f00:1",
            "64:ff9b::a9fe:a9fe",
        ] {
            assert!(!bad(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn ordinary_public_addresses_are_fine() {
        for text in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "172.15.0.1",
            "172.32.0.1",
            "100.63.0.1",
            "100.128.0.1",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
        ] {
            fine(text);
        }
    }

    #[tokio::test]
    async fn an_address_is_checked_before_any_connection_and_a_name_that_leads_inside_is_refused() {
        assert!(matches!(resolve("169.254.169.254", 80, false).await, Err(NetError::Private { .. })));
        assert!(matches!(resolve("[::1]", 80, false).await, Err(NetError::Private { .. })));
        assert!(
            matches!(resolve("localhost", 80, false).await, Err(NetError::Private { why, .. }) if why.contains("loopback"))
        );
        assert_eq!(
            resolve("127.0.0.1", 8080, true).await.unwrap(),
            vec!["127.0.0.1:8080".parse::<SocketAddr>().unwrap()],
            "an exact grant opens it"
        );
        assert!(resolve("8.8.8.8", 443, false).await.is_ok());
        assert!(matches!(resolve("no-such-host.invalid", 443, false).await, Err(NetError::NotFound(_))));
    }
}
