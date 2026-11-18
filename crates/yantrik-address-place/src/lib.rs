//! Where an address leads: the internet, the local network, or never a destination.
//!
//! One answer for every program that fetches for someone else and must not be turned on the
//! machine's own services: yantrik-egress decides each connection by it, and so does the download
//! manager for what an agent asks it to fetch. Moved out of yantrik-egress (an L3 service) so an
//! app may use it; the proxy re-exports it from `policy`.

use std::net::IpAddr;

pub mod ranges;

/// What kind of address a destination resolved to.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Place {
    Internet,
    Lan,
    /// Never a destination.
    Forbidden,
}

/// Where `ip` leads. Never a destination: see the module's notes. Not the internet (`Lan`): the
/// local network, and every other private or special-use range (`crate::ranges`: CGNAT and
/// Tailscale's 100.64/10, the documentation and benchmark ranges, NAT64, 6to4, Teredo, …).
pub fn place_of(ip: IpAddr) -> Place {
    match base_place(ip) {
        Place::Internet if ranges::special(ip) => Place::Lan,
        p => p,
    }
}

fn base_place(ip: IpAddr) -> Place {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            if v4.is_loopback() || v4.is_unspecified() || v4.is_link_local() || v4.is_multicast() || v4.is_broadcast() || o[0] == 0 {
                Place::Forbidden
            } else if v4.is_private() || (o[0] == 100 && (64..128).contains(&o[1])) {
                Place::Lan
            } else {
                Place::Internet
            }
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return place_of(IpAddr::V4(v4));
            }
            let seg = v6.segments();
            let b = v6.octets();
            // An IPv4 address carried inside an IPv6 one is where it leads: NAT64 (64:ff9b::/96)
            // and 6to4 (2002::/16) are classed by the address inside. The old IPv4-compatible
            // form (::a.b.c.d) is never a destination.
            if seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                return place_of(IpAddr::V4(std::net::Ipv4Addr::new(b[12], b[13], b[14], b[15])));
            }
            if seg[0] == 0x2002 {
                return place_of(IpAddr::V4(std::net::Ipv4Addr::new(b[2], b[3], b[4], b[5])));
            }
            if seg[..6] == [0, 0, 0, 0, 0, 0] {
                return Place::Forbidden;
            }
            if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || (seg[0] & 0xffc0) == 0xfe80 {
                Place::Forbidden
            } else if (seg[0] & 0xfe00) == 0xfc00 {
                Place::Lan
            } else {
                Place::Internet
            }
        }
    }
}
