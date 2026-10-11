//! The kernel's IPv4 route table, `/proc/net/route`, read as text.
//!
//! Two readers need it. The Network Manager window opens on the interface the default route
//! leaves by, and network-service reports the gateway of each interface — the one its
//! "Gateway" field showed as "—" on a working DHCP link because nothing read it. One parser,
//! so the window and the service cannot disagree about which route is the default. It lives
//! with the contracts for the reason `machine_status` does: it is the one crate both share.
//!
//! Pure: the table arrives as text, so the tests need no kernel.

use std::net::Ipv4Addr;

/// Where the kernel lists its IPv4 routes.
pub const ROUTE_TABLE: &str = "/proc/net/route";

/// `RTF_UP`: a route the kernel will actually use.
const RTF_UP: u32 = 0x1;
/// `RTF_GATEWAY`: the route goes through a router rather than straight onto the link.
const RTF_GATEWAY: u32 = 0x2;

/// One default route: destination 0.0.0.0 with mask 0.0.0.0, up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultRoute {
    pub iface: String,
    /// `None` for a default route straight onto the link (a point-to-point tunnel has one).
    pub gateway: Option<Ipv4Addr>,
    pub metric: u32,
}

/// Every default route in the table, the one the kernel prefers first: the lowest metric.
pub fn default_routes(table: &str) -> Vec<DefaultRoute> {
    let mut routes: Vec<DefaultRoute> = table
        .lines()
        // The header row: "Iface Destination Gateway Flags RefCnt Use Metric Mask MTU …".
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 8 || f[1] != "00000000" || f[7] != "00000000" {
                return None;
            }
            let flags = u32::from_str_radix(f[3], 16).ok()?;
            if flags & RTF_UP == 0 {
                return None;
            }
            let gateway = Some(address(f[2])?)
                .filter(|g| flags & RTF_GATEWAY != 0 && !g.is_unspecified());
            Some(DefaultRoute {
                iface: f[0].to_string(),
                gateway,
                metric: f[6].parse::<u32>().unwrap_or(u32::MAX),
            })
        })
        .collect();
    // Stable, so two routes at the same metric keep the table's order, as the kernel does.
    routes.sort_by_key(|r| r.metric);
    routes
}

/// The interface the default route leaves by. With more than one (a cable and a radio both
/// up), the kernel prefers the lowest metric, and so does this.
pub fn default_route_interface(table: &str) -> Option<String> {
    default_routes(table).into_iter().next().map(|r| r.iface)
}

/// The router an interface's default route goes through, as `192.168.4.1`. `None` when the
/// interface carries no default route, which is most interfaces on a machine with one link.
pub fn gateway_for(table: &str, iface: &str) -> Option<String> {
    default_routes(table)
        .into_iter()
        .find(|r| r.iface == iface && r.gateway.is_some())
        .and_then(|r| r.gateway)
        .map(|g| g.to_string())
}

/// An address column. The kernel prints the network-order address as a host-order `%08X`, so
/// the bytes are the number's own in memory order: `0104A8C0` is 192.168.4.1 on the
/// little-endian machines this OS runs on.
fn address(hex: &str) -> Option<Ipv4Addr> {
    u32::from_str_radix(hex, 16).ok().map(|n| Ipv4Addr::from(n.to_ne_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str =
        "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT";

    fn table(rows: &[&str]) -> String {
        std::iter::once(HEADER).chain(rows.iter().copied()).collect::<Vec<_>>().join("\n")
    }

    /// The reported machine: ens18 on DHCP, 192.168.4.0/24 through 192.168.4.1.
    fn dhcp() -> String {
        table(&[
            "ens18\t00000000\t0104A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0",
            "ens18\t0004A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0",
        ])
    }

    #[test]
    fn the_default_route_is_the_all_zero_destination_and_mask() {
        assert_eq!(default_route_interface(&dhcp()).as_deref(), Some("ens18"));
    }

    #[test]
    fn with_two_default_routes_the_lower_metric_carries_the_traffic() {
        let t = table(&[
            "wlan0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0",
            "enp3s0\t00000000\t0104A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0",
        ]);
        assert_eq!(default_route_interface(&t).as_deref(), Some("enp3s0"));
        // Each interface still has its own router.
        assert_eq!(gateway_for(&t, "wlan0").as_deref(), Some("192.168.1.1"));
        assert_eq!(gateway_for(&t, "enp3s0").as_deref(), Some("192.168.4.1"));
    }

    #[test]
    fn a_route_that_is_not_up_and_a_table_with_no_default_name_nothing() {
        let down = table(&["eth0\t00000000\t0104A8C0\t0002\t0\t0\t100\t00000000\t0\t0\t0"]);
        assert_eq!(default_route_interface(&down), None);
        assert_eq!(gateway_for(&down, "eth0"), None);
        let local = table(&["eth0\t0000A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0"]);
        assert_eq!(default_route_interface(&local), None);
        assert_eq!(default_route_interface(""), None);
    }

    #[test]
    fn the_gateway_is_read_from_the_interfaces_default_route() {
        assert_eq!(gateway_for(&dhcp(), "ens18").as_deref(), Some("192.168.4.1"));
        // The link route is not a gateway, and another interface has none.
        assert_eq!(gateway_for(&dhcp(), "ens19"), None);
    }

    #[test]
    fn a_default_route_onto_the_link_has_no_gateway() {
        let tunnel = table(&["wg0\t00000000\t00000000\t0001\t0\t0\t50\t00000000\t0\t0\t0"]);
        assert_eq!(default_route_interface(&tunnel).as_deref(), Some("wg0"));
        assert_eq!(gateway_for(&tunnel, "wg0"), None);
    }
}
