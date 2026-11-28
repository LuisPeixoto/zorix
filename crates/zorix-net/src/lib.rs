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

const LOOPBACK: &str = "a loopback address (this computer)";
const PRIVATE: &str = "a private network address";
const SHARED: &str = "a shared (carrier-grade NAT) address";

/// Why an address can never be the address of a service somebody set up, or `None` when it could be. Unlike [`forbidden`] this lets
/// this computer, private networks and shared (carrier-grade NAT, as Tailscale uses) addresses through: a company's own GitHub
/// Enterprise or Jira is a legitimate place for a connection to point at, and a person chose it. What it refuses is what no service
/// lives at: link-local addresses (cloud metadata services), the unspecified address, multicast, broadcast, reserved and
/// documentation ranges, and the tunnel forms that carry one of those.
pub fn never_a_service(ip: IpAddr) -> Option<&'static str> {
    match forbidden(ip) {
        Some(why) if why == LOOPBACK || why == PRIVATE || why == SHARED => None,
        other => other,
    }
}

fn forbidden_v4(ip: Ipv4Addr) -> Option<&'static str> {
    let [a, b, ..] = ip.octets();
    if ip.is_loopback() {
        Some(LOOPBACK)
    } else if ip.is_unspecified() || a == 0 {
        Some("an unspecified address")
    } else if ip.is_private() {
        Some(PRIVATE)
    } else if ip.is_link_local() {
        Some("a link-local address (cloud metadata services live here)")
    } else if a == 100 && (64..=127).contains(&b) {
        Some(SHARED)
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
    let embedded = |hi: u16, lo: u16| Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8);
    // 64:ff9b::/96 carries an IPv4 address in its last 32 bits.
    if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return forbidden_v4(embedded(segments[6], segments[7]));
    }
    // 64:ff9b:1::/48 is the local-use NAT64 prefix: whoever runs the network decides what it reaches, so it is never public.
    if segments[..3] == [0x64, 0xff9b, 1] {
        return Some("a local NAT64 address");
    }
    if ip.is_loopback() {
        return Some(LOOPBACK);
    }
    if ip.is_unspecified() {
        return Some("an unspecified address");
    }
    // ::a.b.c.d (the old IPv4-compatible form) and ::5efe:a.b.c.d (ISATAP) carry an IPv4 address in the last 32 bits.
    if segments[..6] == [0; 6] || (segments[..4] == [0; 4] && segments[4] & 0xfdff == 0 && segments[5] == 0x5efe) {
        return forbidden_v4(embedded(segments[6], segments[7])).or(Some("an IPv4-compatible address"));
    }
    // 6to4 (2002::/16) carries the IPv4 address of the relay's client in bits 16 to 47.
    if segments[0] == 0x2002 {
        return forbidden_v4(embedded(segments[1], segments[2])).or(Some("a 6to4 tunnel address"));
    }
    if segments[0] & 0xfe00 == 0xfc00 {
        Some(PRIVATE)
    } else if segments[0] & 0xffc0 == 0xfe80 || segments[0] & 0xffc0 == 0xfec0 {
        Some("a link-local address")
    } else if ip.is_multicast() {
        Some("a multicast address")
    } else if segments[0] == 0x2001 && segments[1] == 0x0db8 {
        Some("a documentation address")
    } else if segments[0] == 0x2001 && segments[1] < 0x0200 {
        // 2001::/23 is for protocol assignments (Teredo, benchmarking, anycast services), not for ordinary hosts.
        Some("a special-purpose address (Teredo and other protocol assignments)")
    } else if segments[..4] == [0x100, 0, 0, 0] {
        Some("a discard-only address")
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
    check(host, addresses, private_ok)
}

/// Judge the answers of a lookup. Every one must be fine: a single private address among public ones refuses the host.
pub fn check(host: &str, addresses: Vec<SocketAddr>, private_ok: bool) -> Result<Vec<SocketAddr>, NetError> {
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
    fn a_service_may_live_on_a_private_network_but_never_at_a_special_address() {
        for text in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.0.9",
            "172.20.0.1",
            "100.64.0.7",
            "::1",
            "fd12::1",
            "::ffff:10.0.0.1",
            "8.8.8.8",
        ] {
            assert!(never_a_service(text.parse().unwrap()).is_none(), "{text} can be a service");
        }
        for text in [
            "169.254.169.254",
            "169.254.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "240.0.0.1",
            "192.0.2.1",
            "fe80::1",
            "ff02::1",
            "::",
            "::ffff:169.254.169.254",
            "64:ff9b::a9fe:a9fe",
            "2002:a9fe:a9fe::1",
        ] {
            assert!(never_a_service(text.parse().unwrap()).is_some(), "{text} is never a service");
        }
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
            // The tunnels and encodings that carry an IPv4 address (or a network of their own) inside an IPv6 one.
            "64:ff9b:1::1",
            "64:ff9b:1:ffff::a9fe:a9fe",
            "::7f00:1",
            "::a9fe:a9fe",
            "::808:808",
            "2002:7f00:1::1",
            "2002:a9fe:a9fe::1",
            "2002:0a00:0001::5",
            "2002:0808:0808::1",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
            "2001:1::1",
            "2001:2::1",
            "2001:10::1",
            "::5efe:7f00:1",
            "::200:5efe:a00:1",
            "100::1",
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
            "2001:4860:4860::8888",
            "2001:200::1",
        ] {
            fine(text);
        }
    }

    #[test]
    fn a_name_with_one_private_answer_among_public_ones_is_refused_whatever_the_order() {
        let answer = |text: &str| SocketAddr::new(text.parse().unwrap(), 443);
        for order in [["93.184.216.34", "10.0.0.5"], ["10.0.0.5", "93.184.216.34"]] {
            let result = check("rebind.example", order.iter().map(|a| answer(a)).collect(), false);
            assert!(matches!(&result, Err(NetError::Private { ip, .. }) if ip.to_string() == "10.0.0.5"), "{result:?}");
        }
        // The same goes for a private answer hidden in an IPv6 tunnel form next to a public one.
        let mixed = vec![answer("93.184.216.34"), answer("2002:7f00:1::1")];
        assert!(matches!(check("rebind.example", mixed, false), Err(NetError::Private { .. })));
        // All public is fine, an exact grant opens a private one, and nothing at all is "not found".
        assert_eq!(
            check("ok.example", vec![answer("93.184.216.34"), answer("2606:4700:4700::1111")], false).unwrap().len(),
            2
        );
        assert!(check("lan.example", vec![answer("10.0.0.5")], true).is_ok());
        assert!(matches!(check("empty.example", vec![], false), Err(NetError::NotFound(_))));
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
