//! This machine's own addresses — never a destination — and the network it is on.
//!
//! Loopback is refused by its range, but a service bound to 0.0.0.0 also answers on every other
//! address the machine has (`192.168.4.44`, a docker bridge), and traffic to those stays on this
//! machine. Asked of the kernel (`getifaddrs`) at most a second before each connection, so an
//! address that came with a new network is refused within a second of existing.
//!
//! The home network is not only the private ranges. A dual-stack home gives every device a global
//! IPv6 address from the ISP's prefix (2a02:8070:abcd:1::20 is the NAS), and some homes and most
//! servers sit on a public IPv4 subnet. So these are the local network too ([`place`]), whatever
//! range they are in:
//!
//! - every prefix an interface of this machine is on (its netmask);
//! - every prefix the kernel routes straight out of an interface, with no router (a DHCPv6-only
//!   network gives a /128 address and the /64 only as such a route) — no wider than /48 (IPv6) or
//!   /16 (IPv4), never the /0 default, and never out of a point-to-point or tun device, so a VPN's
//!   `0.0.0.0/1 dev tun0` never makes the internet the home network;
//! - the /56 around each global IPv6 address of this machine on a shared network (not a
//!   point-to-point or tun device: a VPN's address says nothing of who is near), which covers the
//!   other /64s of a prefix the ISP delegated (the camera on `…:2::/64` while the desktop is on
//!   `…:1::/64`) and at worst refuses a neighbour the rules then name with `lan: true`. It is a
//!   guess, so it is said: in the control socket's `status` (`assumed_links`) and in the refusal
//!   of an address that is the home network only by it ([`Net::assumed`]);
//! - every router a route sends through.
//!
//! The routes are every table's, IPv4 and IPv6, asked over netlink (`crate::netlink`); when
//! netlink will not answer, `/proc/self/net/route` (IPv4's main table only) and `ipv6_route`, and
//! that is logged.
//!
//! Read with the addresses ([`Watch`]): at most a second old, so a network change is seen within
//! a second; a read that fails — the addresses, or the routes (any error but a missing IPv6
//! table) — keeps the last one that did not. A connection that leaves from an address the read
//! did not know has the network read again then (`crate::proxy`), so a new network is not the
//! internet for that second.
//!
//! Known limit: the router's public WAN address (a request to it hairpins to its admin page)
//! cannot be known here without asking something outside, and nothing is asked; it is the
//! internet to this proxy.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::policy::{place_of, Place};

/// What the kernel says about this machine's network, now.
#[derive(Clone, Debug, Default)]
pub struct Net {
    /// Every address on every interface.
    pub own: Vec<IpAddr>,
    /// Every prefix that is the local network, as an address on it and its length.
    pub links: Vec<(IpAddr, u8)>,
    /// Every router a route sends through, in any table.
    pub gateways: Vec<IpAddr>,
    /// The /56 around each global IPv6 address of this machine on a shared network, as that
    /// address and 56: the local network by a guess, not by anything the kernel said.
    pub assumed: Vec<(IpAddr, u8)>,
}

/// An address on an interface, and the length of the prefix it is on (none for a /0).
pub struct Addr {
    pub dev: String,
    pub ip: IpAddr,
    pub len: Option<u8>,
}

/// What kind of interface a route goes out of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dev {
    /// A network shared with other machines: ethernet, wifi, a bridge.
    Shared,
    /// Point-to-point (IFF_POINTOPOINT), or with no link layer at all (ARPHRD_NONE: tun,
    /// WireGuard): what is behind it is someone else's network, or the internet.
    PointToPoint,
    Loopback,
}

/// A route in the kernel's tables: `dest/len`, through a router or straight out of `dev`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub dest: IpAddr,
    pub len: u8,
    pub via: Option<IpAddr>,
    pub dev: String,
}

/// The widest prefix a route without a router may make the local network.
const WIDEST_ROUTE_V4: u8 = 16;
const WIDEST_ROUTE_V6: u8 = 48;
/// Wider than this, a prefix an interface is on is still the local network, but said so, once
/// in the log and in the control socket's `status` ([`Net::wide`]): a rule for a host in it now
/// needs `lan: true`.
const WIDE_V4: u8 = 16;
const WIDE_V6: u8 = 32;
/// Around each global IPv6 address of this machine: the usual delegated prefix.
const DELEGATED_V6: u8 = 56;

/// Say `what` once for the life of the process, not on every connection.
fn log_once(key: String, what: &str) {
    static SAID: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let mut said = SAID.lock().unwrap_or_else(|e| e.into_inner());
    if said.get_or_insert_with(HashSet::new).insert(key.clone()) {
        tracing::warn!(about = key, "{what}");
    }
}

/// `ip/len` with the host bits cleared.
fn prefix(ip: IpAddr, len: u8) -> String {
    match ip {
        IpAddr::V4(a) => format!("{}/{len}", Ipv4Addr::from(u32::from(a) & u32::MAX.checked_shl(32 - u32::from(len.min(32))).unwrap_or(0))),
        IpAddr::V6(a) => format!("{}/{len}", Ipv6Addr::from(u128::from(a) & u128::MAX.checked_shl(128 - u32::from(len.min(128))).unwrap_or(0))),
    }
}

impl Net {
    /// The network from what the kernel listed: the addresses on each interface, what kind each
    /// interface is (an interface not listed is not trusted with a route), and its routes.
    pub fn build(addrs: &[Addr], devs: &HashMap<String, Dev>, routes: &[Route]) -> Net {
        let mut net = Net::default();
        for a in addrs {
            net.own.push(a.ip);
            if let Some(len) = a.len {
                net.links.push((a.ip, len));
            }
            if let IpAddr::V6(v6) = a.ip {
                if place_of(a.ip) == Place::Internet && devs.get(&a.dev) == Some(&Dev::Shared) && v6.to_ipv4_mapped().is_none() {
                    net.assumed.push((a.ip, DELEGATED_V6));
                }
            }
        }
        for r in routes {
            if let Some(via) = r.via {
                net.gateways.push(via);
                continue;
            }
            let multicast = match r.dest {
                IpAddr::V4(a) => a.is_multicast(),
                IpAddr::V6(a) => a.is_multicast(),
            };
            if r.len == 0 || multicast || devs.get(&r.dev) != Some(&Dev::Shared) {
                continue;
            }
            let widest = if r.dest.is_ipv4() { WIDEST_ROUTE_V4 } else { WIDEST_ROUTE_V6 };
            if r.len < widest {
                // Said only of the internet, and not of the route an interface's own prefix brings.
                let covered = net.links.iter().any(|&(n, l)| l <= r.len && within(r.dest, n, l));
                if covered || place_of(r.dest) != Place::Internet {
                    continue;
                }
                log_once(
                    prefix(r.dest, r.len),
                    "a route this wide is not taken as the home network; only the prefixes of this machine's interfaces are",
                );
                continue;
            }
            net.links.push((r.dest, r.len));
        }
        for (ip, len) in &net.links {
            if is_wide(*ip, *len) {
                log_once(prefix(*ip, *len), "an interface is on a prefix this wide; all of it is the home network, and a rule for a host in it needs `lan: true`");
            }
        }
        net.links.sort();
        net.links.dedup();
        net.gateways.sort();
        net.gateways.dedup();
        net.assumed.sort();
        net.assumed.dedup();
        net
    }

    /// The /56s taken as the local network by a guess ([`Net::assumed`]), written as prefixes.
    pub fn assumed_links(&self) -> Vec<String> {
        let mut out: Vec<String> = self.assumed.iter().map(|(ip, len)| prefix(*ip, *len)).collect();
        out.sort();
        out.dedup();
        out
    }

    /// The prefixes of the internet wider than /16 (IPv4) or /32 (IPv6) taken as the local
    /// network, written as prefixes. (A private range is the local network whatever its width.)
    pub fn wide(&self) -> Vec<String> {
        let mut out: Vec<String> = self.links.iter().filter(|(ip, len)| is_wide(*ip, *len)).map(|(ip, len)| prefix(*ip, *len)).collect();
        out.sort();
        out.dedup();
        out
    }
}

fn is_wide(ip: IpAddr, len: u8) -> bool {
    place_of(ip) == Place::Internet && len < if ip.is_ipv4() { WIDE_V4 } else { WIDE_V6 }
}

/// This machine's addresses, the prefixes that are its network and its routers, now — or `None`
/// when the kernel would not say (found on VM 520: the unit's `RestrictAddressFamilies` had left
/// out the netlink socket `getifaddrs` asks through, the list came back empty, and this machine's
/// own LAN address was let through).
#[cfg(unix)]
pub fn addresses() -> Option<Net> {
    let mut addrs = Vec::new();
    let mut devs: HashMap<String, Dev> = HashMap::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills a list we free below with freeifaddrs; each node is read only
    // while the list is alive, each name as the C string it is, and each address (and netmask)
    // only for the family its header says it is.
    unsafe {
        if libc::getifaddrs(&mut head) != 0 {
            return None;
        }
        let read = |sa: *const libc::sockaddr| -> Option<IpAddr> {
            if sa.is_null() {
                return None;
            }
            match (*sa).sa_family as libc::c_int {
                libc::AF_INET => Some(IpAddr::from(u32::from_be((*(sa as *const libc::sockaddr_in)).sin_addr.s_addr).to_be_bytes())),
                libc::AF_INET6 => Some(IpAddr::from((*(sa as *const libc::sockaddr_in6)).sin6_addr.s6_addr)),
                _ => None,
            }
        };
        let mut cur = head;
        while !cur.is_null() {
            let dev = if (*cur).ifa_name.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr((*cur).ifa_name).to_string_lossy().into_owned()
            };
            let flags = (*cur).ifa_flags;
            let kind = if flags & libc::IFF_LOOPBACK as u32 != 0 {
                Dev::Loopback
            } else if flags & libc::IFF_POINTOPOINT as u32 != 0 {
                Dev::PointToPoint
            } else {
                Dev::Shared
            };
            // Of one interface's entries, the strictest kind said.
            let e = devs.entry(dev.clone()).or_insert(kind);
            if kind != Dev::Shared {
                *e = kind;
            }
            if let Some(ip) = read((*cur).ifa_addr) {
                let len = read((*cur).ifa_netmask).and_then(|m| prefix_len(ip, m));
                addrs.push(Addr { dev, ip, len });
            }
            cur = (*cur).ifa_next;
        }
        libc::freeifaddrs(head);
    }
    // A machine always has loopback; a list without it is a list the kernel did not give.
    if !addrs.iter().any(|a| a.ip.is_loopback()) {
        return None;
    }
    // No link layer (tun, WireGuard) is point-to-point, whatever its flags say.
    for (name, kind) in devs.iter_mut() {
        if *kind == Dev::Shared && arphrd(name) == Some(ARPHRD_NONE) {
            *kind = Dev::PointToPoint;
        }
    }
    let name = |index: u32| {
        let mut buf = [0 as libc::c_char; libc::IF_NAMESIZE];
        // SAFETY: a buffer of IF_NAMESIZE, as if_indextoname asks; read only when it wrote a name.
        let p = unsafe { libc::if_indextoname(index, buf.as_mut_ptr()) };
        (!p.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned())
    };
    let routes = route_tables(|| crate::netlink::dump(&name), |p| std::fs::read_to_string(p))?;
    Some(Net::build(&addrs, &devs, &routes))
}

/// The routes in every table, from `dump` (netlink) — or, when it fails, from the `/proc` files
/// `read` reads, which hold IPv4's main table only. `None` when neither said: a failed read, so
/// the last good network is kept rather than one with no routes and no routers. A missing IPv6
/// table is no IPv6, not a failure; a missing IPv4 one is no fallback.
pub fn route_tables(dump: impl FnOnce() -> std::io::Result<Vec<Route>>, read: impl Fn(&str) -> std::io::Result<String>) -> Option<Vec<Route>> {
    let e = match dump() {
        Ok(r) => return Some(r),
        Err(e) => e,
    };
    log_once(
        "route-dump".into(),
        &format!("the kernel would not dump its routes over netlink ({e}); reading /proc, which lists IPv4's main table only"),
    );
    // Through `self`: the unit's `ProcSubset=pid` hides `/proc/net`, which is only a link to it.
    let v4 = match read("/proc/self/net/route") {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "this machine's routes could not be read, over netlink or from /proc");
            return None;
        }
    };
    let v6 = match read("/proc/self/net/ipv6_route") {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            tracing::warn!(error = %e, "this machine's IPv6 routes could not be read");
            return None;
        }
    };
    Some(routes(&v4, &v6))
}

#[cfg(not(unix))]
pub fn addresses() -> Option<Net> {
    None
}

const ARPHRD_NONE: u16 = 0xfffe;

/// The link-layer type of interface `name`, from `/sys/class/net/<name>/type`.
fn arphrd(name: &str) -> Option<u16> {
    if name.is_empty() || name.contains('/') || name.starts_with('.') {
        return None;
    }
    std::fs::read_to_string(format!("/sys/class/net/{name}/type")).ok()?.trim().parse().ok()
}

/// The length of `mask` when it is a netmask of `addr`'s family (ones, then zeros); none for /0,
/// which is no link.
fn prefix_len(addr: IpAddr, mask: IpAddr) -> Option<u8> {
    let (bits, width) = match (addr, mask) {
        (IpAddr::V4(_), IpAddr::V4(m)) => (u128::from(u32::from(m)) << 96, 32),
        (IpAddr::V6(_), IpAddr::V6(m)) => (u128::from(m), 128),
        _ => return None,
    };
    let len = bits.leading_ones();
    (len > 0 && len <= width && bits.checked_shl(len).unwrap_or(0) == 0).then_some(len as u8)
}

/// The routes in the kernel's tables, `/proc/net/route`- and `ipv6_route`-shaped (when netlink
/// would not answer): those that are
/// up and not a refusal (unreachable, prohibit, blackhole).
pub fn routes(v4: &str, v6: &str) -> Vec<Route> {
    const RTF_UP: u32 = 0x1;
    const RTF_GATEWAY: u32 = 0x2;
    const RTF_REJECT: u32 = 0x200;
    let usable = |flags: u32| flags & RTF_UP != 0 && flags & RTF_REJECT == 0;
    let mut out = Vec::new();
    // Iface Destination Gateway Flags RefCnt Use Metric Mask …, each address one 32-bit word in
    // host byte order.
    for line in v4.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        let (Some(dev), Some(dest), Some(gw), Some(flags), Some(mask)) = (cols.first(), cols.get(1), cols.get(2), cols.get(3), cols.get(7)) else {
            continue;
        };
        let word = |c: &str| u32::from_str_radix(c, 16).ok().map(|w| Ipv4Addr::from(w.to_ne_bytes()));
        let (Some(dest), Some(gw), Ok(flags), Some(mask)) = (word(dest), word(gw), u32::from_str_radix(flags, 16), word(mask)) else { continue };
        if !usable(flags) {
            continue;
        }
        let len = u32::from(mask).leading_ones();
        if u32::from(mask).checked_shl(len).unwrap_or(0) != 0 {
            continue;
        }
        let via = (flags & RTF_GATEWAY != 0 && !gw.is_unspecified()).then_some(IpAddr::V4(gw));
        out.push(Route { dest: IpAddr::V4(dest), len: len as u8, via, dev: (*dev).to_string() });
    }
    // dest plen src plen nexthop metric refcnt use flags iface, addresses as 32 hex digits.
    for line in v6.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 10 || cols[0].len() != 32 || cols[4].len() != 32 {
            continue;
        }
        let (Ok(dest), Ok(len), Ok(hop), Ok(flags)) =
            (u128::from_str_radix(cols[0], 16), u8::from_str_radix(cols[1], 16), u128::from_str_radix(cols[4], 16), u32::from_str_radix(cols[8], 16))
        else {
            continue;
        };
        if !usable(flags) || len > 128 {
            continue;
        }
        let via = (flags & RTF_GATEWAY != 0 && hop != 0).then_some(IpAddr::V6(Ipv6Addr::from(hop)));
        out.push(Route { dest: IpAddr::V6(Ipv6Addr::from(dest)), len, via, dev: cols[9].to_string() });
    }
    out
}

/// The network, as [`addresses`] (or a test's stand-in) reads it, shared by both doors: read
/// again when the last read is older than `fresh`, and when a read fails, the last one that did
/// not. `None` only when no read ever has.
pub struct Watch {
    read: fn() -> Option<Net>,
    fresh: Duration,
    last: Mutex<Option<(Instant, Net)>>,
}

impl Watch {
    /// At most a second old: one `getifaddrs` and two small reads a second at most, however many
    /// connections, and a network change seen within a second.
    pub const FRESH: Duration = Duration::from_secs(1);

    pub fn new(read: fn() -> Option<Net>, fresh: Duration) -> Watch {
        Watch { read, fresh, last: Mutex::new(None) }
    }

    pub fn now(&self) -> Option<Net> {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, net)) = last.as_ref() {
            if at.elapsed() < self.fresh {
                return Some(net.clone());
            }
        }
        match (self.read)() {
            Some(net) => {
                *last = Some((Instant::now(), net.clone()));
                Some(net)
            }
            None => {
                tracing::warn!(
                    known = last.is_some(),
                    "this machine's network could not be read; the last one read is used, and with none every connection is refused"
                );
                last.as_ref().map(|(_, net)| net.clone())
            }
        }
    }

    /// The network read now, however fresh the last read is; `None` when this read failed (the
    /// last good one is still kept for [`Watch::now`]).
    pub fn refresh(&self) -> Option<Net> {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        let Some(net) = (self.read)() else {
            tracing::warn!("this machine's network could not be read again");
            return None;
        };
        *last = Some((Instant::now(), net.clone()));
        Some(net)
    }
}

impl Net {
    /// Whether `ip` — or the IPv4 address it carries (mapped, NAT64, 6to4) — is on a prefix this
    /// machine is on, or is one of its routers.
    pub fn on_link(&self, ip: IpAddr) -> bool {
        let mut forms = vec![ip];
        if let IpAddr::V6(v6) = ip {
            let (seg, b) = (v6.segments(), v6.octets());
            if let Some(v4) = v6.to_ipv4_mapped() {
                forms.push(IpAddr::V4(v4));
            } else if seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] || seg[..3] == [0x64, 0xff9b, 1] {
                forms.push(IpAddr::V4(Ipv4Addr::new(b[12], b[13], b[14], b[15])));
            } else if seg[0] == 0x2002 {
                forms.push(IpAddr::V4(Ipv4Addr::new(b[2], b[3], b[4], b[5])));
            }
        }
        forms.iter().any(|&a| self.gateways.contains(&a) || self.links.iter().chain(&self.assumed).any(|&(net, len)| within(a, net, len)))
    }

    /// When `ip` is the local network only by a guessed /56 ([`Net::assumed`]): which, said so a
    /// refusal of it can say why.
    pub fn assumed(&self, ip: IpAddr) -> Option<String> {
        let by = self.assumed.iter().find(|&&(net, len)| within(ip, net, len))?;
        let known = Net { assumed: Vec::new(), ..self.clone() };
        (!known.on_link(ip)).then(|| {
            format!(
                "{ip} is the home network only by a guess: it is in {}, the /56 around this machine's {}.",
                prefix(by.0, by.1),
                by.0
            )
        })
    }
}

fn within(ip: IpAddr, net: IpAddr, len: u8) -> bool {
    match (ip, net) {
        (IpAddr::V4(a), IpAddr::V4(n)) if len <= 32 => {
            let mask = u32::MAX.checked_shl(32 - u32::from(len)).unwrap_or(0);
            u32::from(a) & mask == u32::from(n) & mask
        }
        (IpAddr::V6(a), IpAddr::V6(n)) if len <= 128 => {
            let mask = u128::MAX.checked_shl(128 - u32::from(len)).unwrap_or(0);
            u128::from(a) & mask == u128::from(n) & mask
        }
        _ => false,
    }
}

/// Whether `ip` is one of this machine's, or an IPv4 one written as IPv6. When the machine's own
/// addresses are not known, every address on the local network might be one of them, and is
/// taken to be: refused, not let through.
pub fn is_own(ip: IpAddr, own: Option<&[IpAddr]>) -> bool {
    let plain = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        v4 => v4,
    };
    match own {
        Some(own) => own.contains(&plain),
        None => place_of(plain) == Place::Lan,
    }
}

/// Where `ip` leads from this machine, on `net`: never a destination if it is this machine's
/// own; the local network if it is on one of this machine's prefixes or is one of its routers,
/// in whatever range; otherwise its range says ([`place_of`]).
pub fn place(ip: IpAddr, net: Option<&Net>) -> Place {
    if is_own(ip, net.map(|n| n.own.as_slice())) {
        return Place::Forbidden;
    }
    match place_of(ip) {
        Place::Internet if net.is_some_and(|n| n.on_link(ip)) => Place::Lan,
        p => p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn this_machine_has_its_loopback_and_it_is_its_own() {
        let own = addresses().expect("the kernel lists this machine's addresses").own;
        assert!(own.contains(&"127.0.0.1".parse().unwrap()), "{own:?}");
        assert!(is_own("::ffff:127.0.0.1".parse().unwrap(), Some(&own)));
        assert!(!is_own("192.0.2.1".parse().unwrap(), Some(&own)), "a documentation address is nobody's");
    }

    /// Not knowing this machine's addresses refuses the local network rather than letting it all
    /// through: any address on it might be this machine's.
    #[test]
    fn unknown_own_addresses_fail_closed_for_the_local_network() {
        assert!(is_own("192.168.4.44".parse().unwrap(), None));
        assert!(is_own("10.0.0.5".parse().unwrap(), None));
        assert!(!is_own("93.184.215.14".parse().unwrap(), None), "the internet is not this machine");
    }

    #[test]
    fn a_netmask_is_its_prefix_length() {
        let len = |a: &str, m: &str| prefix_len(a.parse().unwrap(), m.parse().unwrap());
        assert_eq!(len("192.168.4.44", "255.255.255.0"), Some(24));
        assert_eq!(len("81.2.69.165", "255.255.255.240"), Some(28));
        assert_eq!(len("2a02:8070:abcd:1::5", "ffff:ffff:ffff:ffff::"), Some(64));
        assert_eq!(len("10.0.0.1", "0.0.0.0"), None, "/0 is no link");
        assert_eq!(len("10.0.0.1", "255.0.255.0"), None, "not a netmask");
        assert_eq!(len("10.0.0.1", "ffff::"), None, "another family");
    }

    fn ip(a: &str) -> IpAddr {
        a.parse().unwrap()
    }

    fn addr(dev: &str, a: &str, len: u8) -> Addr {
        Addr { dev: dev.into(), ip: ip(a), len: Some(len) }
    }

    fn devs(list: &[(&str, Dev)]) -> HashMap<String, Dev> {
        list.iter().map(|(n, k)| (n.to_string(), *k)).collect()
    }

    fn on(dest: &str, len: u8, dev: &str) -> Route {
        Route { dest: ip(dest), len, via: None, dev: dev.into() }
    }

    fn via(gw: &str, dev: &str) -> Route {
        let any = if gw.contains(':') { "::" } else { "0.0.0.0" };
        Route { dest: ip(any), len: 0, via: Some(ip(gw)), dev: dev.into() }
    }

    /// Written the way the kernel writes them on a little-endian machine.
    #[cfg(target_endian = "little")]
    #[test]
    fn the_routes_are_read_from_the_route_tables() {
        let v4 = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                  eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
                  eth0\t0004A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n\
                  ppp0\t00000000\t01450251\t0003\t0\t0\t200\t00000000\t0\t0\t0\n\
                  eth0\t0005A8C0\t00000000\t0201\t0\t0\t100\t00FFFFFF\t0\t0\t0\n\
                  eth0\t0006A8C0\t00000000\t0000\t0\t0\t100\t00FFFFFF\t0\t0\t0\n";
        let v6 = "00000000000000000000000000000000 00 00000000000000000000000000000000 00 fe80000000000000021122fffe334455 00000400 00000001 00000000 00000003 eth0\n\
                  00000000000000000000000000000000 00 00000000000000000000000000000000 00 2a028070abcd00000000000000000001 00000400 00000001 00000000 00000003 eth0\n\
                  2a028070abcd00010000000000000000 40 00000000000000000000000000000000 00 00000000000000000000000000000000 00000100 00000001 00000000 00000001 eth0\n\
                  00000000000000000000000000000000 00 00000000000000000000000000000000 00 00000000000000000000000000000000 ffffffff 00000001 00000000 00200200 lo\n";
        let got = routes(v4, v6);
        let want = vec![
            via("192.168.1.1", "eth0"),
            on("192.168.4.0", 24, "eth0"),
            via("81.2.69.1", "ppp0"),
            via("fe80::211:22ff:fe33:4455", "eth0"),
            via("2a02:8070:abcd::1", "eth0"),
            on("2a02:8070:abcd:1::", 64, "eth0"),
        ];
        assert_eq!(got, want, "a refusal (192.168.5.0/24, ::/0 on lo) and a route that is down (192.168.6.0/24) are not routes");
        let net = Net::build(&[], &devs(&[("eth0", Dev::Shared), ("ppp0", Dev::PointToPoint)]), &got);
        assert_eq!(net.gateways, ["81.2.69.1", "192.168.1.1", "2a02:8070:abcd::1", "fe80::211:22ff:fe33:4455"].map(ip).to_vec());
    }

    /// The ISP delegates a /56; the desktop is on its first /64, the camera on another.
    #[test]
    fn a_device_on_another_64_of_a_delegated_prefix_is_the_home_network() {
        let net = Net::build(
            &[addr("lo", "127.0.0.1", 8), addr("lo", "::1", 128), addr("eth0", "2a02:8070:abcd:1::5", 64), addr("eth0", "192.168.1.10", 24)],
            &devs(&[("lo", Dev::Loopback), ("eth0", Dev::Shared)]),
            &[via("fe80::1", "eth0"), on("2a02:8070:abcd:1::", 64, "eth0")],
        );
        assert_eq!(place(ip("2a02:8070:abcd:2::30"), Some(&net)), Place::Lan, "the camera");
        assert_eq!(place(ip("2a02:8070:abcd:ff::1"), Some(&net)), Place::Lan, "the last /64 of the /56");
        for other in ["2a02:8070:abcd:100::1", "2a02:8070:abce:2::30", "2606:4700::1111"] {
            assert_eq!(place(ip(other), Some(&net)), Place::Internet, "{other}");
        }
    }

    /// Stateful DHCPv6 only (M=1, A=0): the address is a /128, and its /64 exists only as a route.
    #[test]
    fn a_dhcpv6_only_network_is_read_from_its_on_link_route() {
        let net = Net::build(
            &[addr("lo", "127.0.0.1", 8), addr("eth0", "2a02:8070:abcd:1::5", 128)],
            &devs(&[("lo", Dev::Loopback), ("eth0", Dev::Shared)]),
            &[on("2a02:8070:abcd:1::", 64, "eth0"), via("fe80::1", "eth0")],
        );
        assert!(net.links.contains(&(ip("2a02:8070:abcd:1::"), 64)), "{:?}", net.links);
        assert_eq!(place(ip("2a02:8070:abcd:1::20"), Some(&net)), Place::Lan);
        assert_eq!(place(ip("2a02:8070:abcd:1::5"), Some(&net)), Place::Forbidden, "this machine");
        // The route alone: a /64 outside the /56 around the address.
        let net = Net::build(&[addr("eth0", "2a02:8070:abcd:1::5", 128)], &devs(&[("eth0", Dev::Shared)]), &[on("2a02:8070:ffff:1::", 64, "eth0")]);
        assert_eq!(place(ip("2a02:8070:ffff:1::20"), Some(&net)), Place::Lan);
    }

    /// A full-tunnel VPN: 0.0.0.0/1 and 128.0.0.0/1 (and ::/1, 8000::/1) out of tun0. Out of a
    /// point-to-point device, and wider than any home, they are never the home network — not even
    /// if tun0 were taken for a shared network.
    #[cfg(target_endian = "little")]
    #[test]
    fn a_full_tunnel_vpn_leaves_the_internet_the_internet() {
        let v4 = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                  tun0\t00000000\t00000000\t0001\t0\t0\t0\t00000080\t0\t0\t0\n\
                  tun0\t00000080\t00000000\t0001\t0\t0\t0\t00000080\t0\t0\t0\n\
                  eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
                  eth0\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n";
        let v6 = "00000000000000000000000000000000 01 00000000000000000000000000000000 00 00000000000000000000000000000000 00000400 00000001 00000000 00000001 tun0\n\
                  80000000000000000000000000000000 01 00000000000000000000000000000000 00 00000000000000000000000000000000 00000400 00000001 00000000 00000001 tun0\n";
        let r = routes(v4, v6);
        assert!(r.contains(&on("0.0.0.0", 1, "tun0")) && r.contains(&on("128.0.0.0", 1, "tun0")), "{r:?}");
        let addrs = [addr("lo", "127.0.0.1", 8), addr("eth0", "192.168.1.10", 24), addr("tun0", "10.66.0.2", 32)];
        for tun in [Dev::PointToPoint, Dev::Shared] {
            let net = Net::build(&addrs, &devs(&[("lo", Dev::Loopback), ("eth0", Dev::Shared), ("tun0", tun)]), &r);
            for out in ["1.1.1.1", "70.1.1.1", "150.1.1.1", "2606:4700::1111"] {
                assert_eq!(place(ip(out), Some(&net)), Place::Internet, "{out} with tun0 {tun:?}");
            }
            assert_eq!(place(ip("192.168.1.1"), Some(&net)), Place::Lan);
        }
        // A narrow route out of a tun device is someone else's network too.
        let net = Net::build(&addrs, &devs(&[("tun0", Dev::PointToPoint)]), &[on("81.2.69.0", 24, "tun0")]);
        assert_eq!(place(ip("81.2.69.9"), Some(&net)), Place::Internet);
        // And a route out of an interface nobody listed is not trusted.
        let net = Net::build(&addrs, &devs(&[]), &[on("81.2.69.0", 24, "eth9")]);
        assert_eq!(place(ip("81.2.69.9"), Some(&net)), Place::Internet);
    }

    /// A route wider than a home (/8; a /40 for IPv6) is skipped; an interface's own prefix that
    /// wide still counts, and is said in `wide`.
    #[test]
    fn a_wide_route_is_skipped_and_a_wide_interface_prefix_is_said() {
        let d = devs(&[("eth0", Dev::Shared)]);
        let net = Net::build(&[addr("eth0", "192.168.1.10", 24)], &d, &[on("44.0.0.0", 8, "eth0"), on("2a02:8000::", 40, "eth0")]);
        assert_eq!(place(ip("44.9.9.9"), Some(&net)), Place::Internet);
        assert_eq!(place(ip("2a02:8000:1::1"), Some(&net)), Place::Internet);
        assert!(net.wide().is_empty());
        let net = Net::build(&[addr("eth0", "81.2.69.5", 16), addr("eth0", "2a02:8070:abcd:1::5", 48)], &d, &[on("81.3.0.0", 16, "eth0"), on("2a02:9000:1::", 48, "eth0")]);
        assert_eq!(place(ip("81.3.200.1"), Some(&net)), Place::Lan, "a /16 route is as wide as a home may be");
        assert_eq!(place(ip("2a02:9000:1:2::1"), Some(&net)), Place::Lan, "so is a /48");
        assert!(net.wide().is_empty(), "a /16 and a /48 are not wide: {:?}", net.wide());
        let net = Net::build(
            &[addr("lo", "127.0.0.1", 8), addr("eth0", "10.1.2.3", 8), addr("eth0", "44.1.2.3", 8), addr("eth0", "2a02:8070:abcd:1::5", 24)],
            &d,
            &[on("44.0.0.0", 8, "eth0")],
        );
        assert_eq!(place(ip("44.9.9.9"), Some(&net)), Place::Lan);
        assert_eq!(net.wide(), vec!["2a02:8000::/24".to_string(), "44.0.0.0/8".to_string()], "not loopback's /8, nor 10/8");
    }

    /// A read that fails keeps the last one that did not; with none, there is no network to judge by.
    #[test]
    fn a_failed_read_keeps_the_last_good_network() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static UP: AtomicBool = AtomicBool::new(false);
        fn read() -> Option<Net> {
            UP.load(Ordering::SeqCst).then(|| Net { own: vec!["127.0.0.1".parse().unwrap()], ..Net::default() })
        }
        let w = Watch::new(read, Duration::ZERO);
        assert!(w.now().is_none(), "never read");
        UP.store(true, Ordering::SeqCst);
        assert_eq!(w.now().unwrap().own.len(), 1);
        UP.store(false, Ordering::SeqCst);
        assert_eq!(w.now().map(|n| n.own), Some(vec![ip("127.0.0.1")]), "the last good one");
    }

    /// A home device on the ISP's global prefix, on a public IPv4 subnet, or the router, is the
    /// local network, in any form it is written in; the rest of the internet is not.
    #[test]
    fn the_prefixes_this_machine_is_on_and_its_routers_are_the_local_network() {
        let net = Net {
            own: vec!["127.0.0.1".parse().unwrap(), "2a02:8070:abcd:1::5".parse().unwrap()],
            links: vec![("2a02:8070:abcd:1::5".parse().unwrap(), 64), ("81.2.69.165".parse().unwrap(), 28)],
            gateways: vec!["81.2.69.1".parse().unwrap(), "2a02:8070:abcd::1".parse().unwrap()],
            ..Net::default()
        };
        for ip in [
            "2a02:8070:abcd:1::20", "2a02:8070:abcd:1:ffff::1", "81.2.69.170", "::ffff:81.2.69.170", "64:ff9b::5102:45aa",
            "2002:5102:45aa::1", "81.2.69.1", "2a02:8070:abcd::1",
        ] {
            assert_eq!(place(ip.parse().unwrap(), Some(&net)), Place::Lan, "{ip}");
        }
        for ip in ["2a02:8070:abcd:2::20", "81.2.69.176", "81.2.69.2", "1.1.1.1", "2606:4700::1111"] {
            assert_eq!(place(ip.parse().unwrap(), Some(&net)), Place::Internet, "{ip}");
        }
        assert_eq!(place("2a02:8070:abcd:1::5".parse().unwrap(), Some(&net)), Place::Forbidden, "this machine");
        assert_eq!(place("192.168.4.20".parse().unwrap(), Some(&net)), Place::Lan, "the private ranges as before");
    }

    /// A policy-routed host: an on-link /24 only in table 100, a router only in table 200 (both
    /// from a netlink dump, `crate::netlink::tests::fixture`). Both are the home network; the
    /// /1 out of tun0 in wg-quick's table is not.
    #[test]
    fn routes_and_routers_in_every_table_are_the_home_network() {
        let mut r = Vec::new();
        assert!(crate::netlink::parse(&crate::netlink::tests::fixture(), 7, &crate::netlink::tests::names, &mut r).unwrap());
        let net = Net::build(
            &[addr("lo", "127.0.0.1", 8), addr("eth0", "192.168.1.10", 24), addr("tun0", "10.66.0.2", 32)],
            &devs(&[("lo", Dev::Loopback), ("eth0", Dev::Shared), ("eth1", Dev::Shared), ("tun0", Dev::PointToPoint)]),
            &r,
        );
        for lan in ["81.2.69.9", "87.1.1.1", "2a02:8070:abcd:1::20", "10.0.0.1"] {
            assert_eq!(place(ip(lan), Some(&net)), Place::Lan, "{lan}");
        }
        for out in ["81.2.70.9", "87.1.1.2", "1.1.1.1", "70.1.1.1", "93.184.215.7", "2606:4700::1111"] {
            assert_eq!(place(ip(out), Some(&net)), Place::Internet, "{out}");
        }
        assert!(net.gateways.contains(&ip("fe80::2")), "an IPv4 route's IPv6 router (RTA_VIA)");
        assert!(!net.gateways.contains(&ip("10.0.0.2")), "a dead next hop is no router");
    }

    /// A route table that cannot be read is a failed read, not a table with no routes: the
    /// network is not taken without its routers. Only a missing IPv6 table (no IPv6) is empty.
    #[test]
    fn a_route_table_that_cannot_be_read_is_a_failed_read() {
        use std::io::{Error, ErrorKind};
        let nl_down = || Err(Error::from_raw_os_error(libc::EAFNOSUPPORT));
        let v4 = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\neth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n";
        let files = |v4_err: Option<i32>, v6_err: Option<i32>| {
            move |p: &str| match (p.ends_with("/route"), v4_err, v6_err) {
                (true, Some(e), _) | (false, _, Some(e)) => Err(Error::from_raw_os_error(e)),
                (true, None, _) => Ok(v4.to_string()),
                (false, _, None) => Ok(String::new()),
            }
        };
        let from_netlink = vec![on("81.2.69.0", 24, "eth0")];
        assert_eq!(route_tables(|| Ok(from_netlink.clone()), files(Some(libc::EMFILE), None)), Some(from_netlink), "netlink, not /proc");
        assert_eq!(route_tables(nl_down, files(None, Some(libc::ENOENT))).map(|r| r.len()), Some(1), "/proc; no IPv6 is no IPv6 routes");
        assert_eq!(route_tables(nl_down, files(Some(libc::EMFILE), None)), None, "IPv4's table unreadable");
        assert_eq!(route_tables(nl_down, files(None, Some(libc::EACCES))), None, "IPv6's table unreadable");
        assert_eq!(route_tables(nl_down, files(Some(libc::ENOENT), None)), None, "netlink failed and no /proc to fall back on");
        assert_eq!(Error::from_raw_os_error(libc::ENOENT).kind(), ErrorKind::NotFound);

        // And Watch then keeps the last good network, routers and all.
        use std::sync::atomic::{AtomicBool, Ordering};
        static ROUTES: AtomicBool = AtomicBool::new(true);
        fn read() -> Option<Net> {
            let ok = ROUTES.load(Ordering::SeqCst);
            let routes = route_tables(|| if ok { Ok(vec![via("81.2.69.1", "eth0")]) } else { Err(Error::from_raw_os_error(libc::EMFILE)) }, |_| {
                Err(Error::from_raw_os_error(libc::EMFILE))
            })?;
            Some(Net::build(&[addr("lo", "127.0.0.1", 8)], &devs(&[("eth0", Dev::Shared)]), &routes))
        }
        let w = Watch::new(read, Duration::ZERO);
        assert_eq!(w.now().unwrap().gateways, vec![ip("81.2.69.1")]);
        ROUTES.store(false, Ordering::SeqCst);
        assert_eq!(w.now().unwrap().gateways, vec![ip("81.2.69.1")], "the last good one, with its router");
        assert!(w.refresh().is_none(), "a read now says it failed");
    }

    /// The /56 is a guess, so it is said, and only made on a shared network: not around a VPN's
    /// global address on a point-to-point or tun device.
    #[test]
    fn the_guessed_56_is_said_and_not_made_for_a_tunnel() {
        let net = Net::build(
            &[addr("lo", "::1", 128), addr("eth0", "2a02:8070:abcd:1::5", 64), addr("wg0", "2a0c:1111:2222:3::9", 128), addr("tun0", "2a0c:3333:4444:5::9", 64)],
            &devs(&[("lo", Dev::Loopback), ("eth0", Dev::Shared), ("wg0", Dev::PointToPoint), ("tun0", Dev::PointToPoint)]),
            &[],
        );
        assert_eq!(net.assumed_links(), vec!["2a02:8070:abcd::/56".to_string()]);
        assert_eq!(place(ip("2a0c:1111:2222:4::1"), Some(&net)), Place::Internet, "not the /56 around wg0's address");
        assert_eq!(place(ip("2a0c:3333:4444:6::1"), Some(&net)), Place::Internet, "nor tun0's");
        assert_eq!(place(ip("2a02:8070:abcd:2::30"), Some(&net)), Place::Lan);
        let why = net.assumed(ip("2a02:8070:abcd:2::30")).expect("Lan only by the /56");
        assert!(why.contains("2a02:8070:abcd::/56") && why.contains("2a02:8070:abcd:1::5"), "{why}");
        assert!(!why.contains("lan: true"), "which rule opens it is the door's word, not this one's: {why}");
        assert_eq!(net.assumed(ip("2a02:8070:abcd:1::20")), None, "on eth0's own /64: not a guess");
        assert_eq!(net.assumed(ip("2606:4700::1111")), None);
    }
}
