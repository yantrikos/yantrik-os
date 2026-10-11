//! The kernel's routes, asked over netlink: one `RTM_GETROUTE` dump, every table, IPv4 and IPv6.
//!
//! `/proc/self/net/route` lists only IPv4's main table, so an on-link prefix or a router a
//! policy-routed host keeps in another table (`81.2.69.0/24 dev eth0 table 100`, systemd-networkd's
//! `RouteTable=`, a VRF) was not seen, and its devices counted as the internet. A dump with no
//! table asked for gives every table. Only unicast routes are routes here: the `local` table's
//! own addresses and broadcasts, multicast, and the refusals (unreachable, prohibit, blackhole)
//! are not. The answer is taken only from the kernel (port 0), and only for this request.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::local::Route;

const NLMSG_HDRLEN: usize = 16;
const RTMSG_LEN: usize = 12;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const RTM_NEWROUTE: u16 = 24;
const RTM_GETROUTE: u16 = 26;
const NLM_F_REQUEST: u16 = 0x1;
const NLM_F_DUMP: u16 = 0x300;
const NLM_F_DUMP_INTR: u16 = 0x10;
const FAMILY_V4: u8 = 2;
const FAMILY_V6: u8 = 10;
const RTN_UNICAST: u8 = 1;
const RTA_DST: u16 = 1;
const RTA_OIF: u16 = 4;
const RTA_GATEWAY: u16 = 5;
const RTA_MULTIPATH: u16 = 9;
const RTA_VIA: u16 = 18;
const RTNH_F_DEAD: u32 = 0x1;
/// The most a dump may be, however many routes: past this it is not read.
const MOST: usize = 16 << 20;

fn align(n: usize) -> usize {
    (n + 3) & !3
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_ne_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("the kernel's route dump {what}"))
}

/// An address of `family` from an attribute's payload.
fn ip(family: u8, b: &[u8]) -> Option<IpAddr> {
    match family {
        FAMILY_V4 => Some(IpAddr::V4(Ipv4Addr::from(<[u8; 4]>::try_from(b.get(..4)?).ok()?))),
        FAMILY_V6 => Some(IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(b.get(..16)?).ok()?))),
        _ => None,
    }
}

/// The attributes in `b`, as (type, payload).
fn attrs(b: &[u8]) -> io::Result<Vec<(u16, &[u8])>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 4 <= b.len() {
        let len = usize::from(u16_at(b, at).unwrap_or(0));
        let kind = u16_at(b, at + 2).unwrap_or(0) & 0x3fff;
        if len < 4 || at + len > b.len() {
            return Err(bad("has an attribute that does not fit"));
        }
        out.push((kind, &b[at + 4..at + len]));
        at += align(len);
    }
    Ok(out)
}

/// The router in a route's (or a next hop's) attributes: `RTA_GATEWAY`, or `RTA_VIA` (an IPv4
/// route through an IPv6 router).
fn router(family: u8, a: &[(u16, &[u8])]) -> Option<IpAddr> {
    a.iter().find_map(|&(k, v)| match k {
        RTA_GATEWAY => ip(family, v),
        RTA_VIA => ip(u16_at(v, 0)? as u8, v.get(2..)?),
        _ => None,
    })
}

/// Read one batch of a dump's messages into `out`, keeping those that answer request `seq`.
/// `Ok(true)` when the dump has ended. `name` gives an interface's name from its index.
pub fn parse(b: &[u8], seq: u32, name: &dyn Fn(u32) -> Option<String>, out: &mut Vec<Route>) -> io::Result<bool> {
    let mut at = 0;
    while at + NLMSG_HDRLEN <= b.len() {
        let len = u32_at(b, at).unwrap_or(0) as usize;
        if len < NLMSG_HDRLEN || at + len > b.len() {
            return Err(bad("has a message that does not fit"));
        }
        let (kind, flags, mseq) = (u16_at(b, at + 4).unwrap_or(0), u16_at(b, at + 6).unwrap_or(0), u32_at(b, at + 8).unwrap_or(0));
        let body = &b[at + NLMSG_HDRLEN..at + len];
        at += align(len);
        if mseq != seq {
            continue;
        }
        if flags & NLM_F_DUMP_INTR != 0 {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "the route tables changed during the dump"));
        }
        match kind {
            NLMSG_DONE => return Ok(true),
            NLMSG_ERROR => {
                let errno = u32_at(body, 0).ok_or_else(|| bad("has an error that does not fit"))? as i32;
                if errno != 0 {
                    return Err(io::Error::from_raw_os_error(-errno));
                }
            }
            RTM_NEWROUTE => {
                if body.len() < RTMSG_LEN {
                    return Err(bad("has a route that does not fit"));
                }
                let (family, len, kind, rflags) = (body[0], body[1], body[7], u32_at(body, 8).unwrap_or(0));
                let width = match family {
                    FAMILY_V4 => 32,
                    FAMILY_V6 => 128,
                    _ => continue,
                };
                if kind != RTN_UNICAST || len > width {
                    continue;
                }
                let a = attrs(&body[RTMSG_LEN..])?;
                let dest = match a.iter().find(|&&(k, _)| k == RTA_DST) {
                    Some(&(_, v)) => ip(family, v).ok_or_else(|| bad("has a destination that does not fit"))?,
                    None if family == FAMILY_V4 => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                    None => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
                };
                let dev = |index: u32| name(index).unwrap_or_default();
                if let Some(&(_, mut hops)) = a.iter().find(|&&(k, _)| k == RTA_MULTIPATH) {
                    // rtnexthop: length, flags, hops, interface index; then its own attributes.
                    while hops.len() >= 8 {
                        let hlen = usize::from(u16_at(hops, 0).unwrap_or(0));
                        if hlen < 8 || hlen > hops.len() {
                            return Err(bad("has a next hop that does not fit"));
                        }
                        let (hflags, index) = (u32::from(hops[2]), u32_at(hops, 4).unwrap_or(0));
                        if hflags & RTNH_F_DEAD == 0 {
                            out.push(Route { dest, len, via: router(family, &attrs(&hops[8..hlen])?), dev: dev(index) });
                        }
                        hops = &hops[align(hlen).min(hops.len())..];
                    }
                } else if rflags & RTNH_F_DEAD == 0 {
                    let index = a.iter().find(|&&(k, _)| k == RTA_OIF).and_then(|&(_, v)| u32_at(v, 0)).unwrap_or(0);
                    out.push(Route { dest, len, via: router(family, &a), dev: dev(index) });
                }
            }
            _ => {}
        }
    }
    Ok(false)
}

/// Every unicast route in every table, IPv4 and IPv6. A dump the tables changed under is asked
/// again, twice at most.
#[cfg(target_os = "linux")]
pub fn dump(name: &dyn Fn(u32) -> Option<String>) -> io::Result<Vec<Route>> {
    let mut last = io::Error::new(io::ErrorKind::Interrupted, "the route tables kept changing during the dump");
    for seq in 1..=3 {
        match dump_once(seq, name) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => last = e,
            r => return r,
        }
    }
    Err(last)
}

#[cfg(target_os = "linux")]
fn dump_once(seq: u32, name: &dyn Fn(u32) -> Option<String>) -> io::Result<Vec<Route>> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    // SAFETY: a socket we own (closed with `fd`); setsockopt, sendto and recvfrom each get a
    // pointer to a value or buffer that outlives the call, with its true size.
    unsafe {
        let raw = libc::socket(libc::AF_NETLINK, libc::SOCK_RAW | libc::SOCK_CLOEXEC, libc::NETLINK_ROUTE);
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = OwnedFd::from_raw_fd(raw);
        // Never more than a second waiting on the kernel.
        let tv = libc::timeval { tv_sec: 1, tv_usec: 0 };
        libc::setsockopt(fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_RCVTIMEO, &tv as *const _ as *const libc::c_void, std::mem::size_of::<libc::timeval>() as libc::socklen_t);
        // nlmsghdr, then an rtmsg of zeros: any family, any table.
        let mut req = [0u8; NLMSG_HDRLEN + RTMSG_LEN];
        req[0..4].copy_from_slice(&((NLMSG_HDRLEN + RTMSG_LEN) as u32).to_ne_bytes());
        req[4..6].copy_from_slice(&RTM_GETROUTE.to_ne_bytes());
        req[6..8].copy_from_slice(&(NLM_F_REQUEST | NLM_F_DUMP).to_ne_bytes());
        req[8..12].copy_from_slice(&seq.to_ne_bytes());
        let mut kernel: libc::sockaddr_nl = std::mem::zeroed();
        kernel.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        let n = libc::sendto(
            fd.as_raw_fd(),
            req.as_ptr() as *const libc::c_void,
            req.len(),
            0,
            &kernel as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        );
        if n != req.len() as isize {
            return Err(io::Error::last_os_error());
        }
        let mut buf = vec![0u8; 64 * 1024];
        let (mut out, mut total) = (Vec::new(), 0usize);
        loop {
            let mut from: libc::sockaddr_nl = std::mem::zeroed();
            let mut from_len = std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t;
            let n = libc::recvfrom(fd.as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, buf.len(), libc::MSG_TRUNC, &mut from as *mut _ as *mut libc::sockaddr, &mut from_len);
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            let n = n as usize;
            if n == 0 || n > buf.len() {
                return Err(bad(if n == 0 { "ended early" } else { "sent a message larger than its buffer" }));
            }
            // Only the kernel's word: anything else on this socket is not the route tables.
            if from.nl_pid != 0 {
                continue;
            }
            total += n;
            if total > MOST {
                return Err(bad("is larger than any machine's tables"));
            }
            if parse(&buf[..n], seq, name, &mut out)? {
                return Ok(out);
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub fn dump(_: &dyn Fn(u32) -> Option<String>) -> io::Result<Vec<Route>> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "no netlink here"))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// One attribute: length, type, payload, padded to four bytes.
    fn attr(kind: u16, payload: &[u8]) -> Vec<u8> {
        let mut a = Vec::new();
        a.extend_from_slice(&((4 + payload.len()) as u16).to_ne_bytes());
        a.extend_from_slice(&kind.to_ne_bytes());
        a.extend_from_slice(payload);
        a.resize(align(a.len()), 0);
        a
    }

    fn msg(kind: u16, flags: u16, seq: u32, body: &[u8]) -> Vec<u8> {
        let mut m = Vec::new();
        m.extend_from_slice(&((NLMSG_HDRLEN + body.len()) as u32).to_ne_bytes());
        m.extend_from_slice(&kind.to_ne_bytes());
        m.extend_from_slice(&flags.to_ne_bytes());
        m.extend_from_slice(&seq.to_ne_bytes());
        m.extend_from_slice(&0u32.to_ne_bytes());
        m.extend_from_slice(body);
        m.resize(align(m.len()), 0);
        m
    }

    /// An `RTM_NEWROUTE` as the kernel writes it: rtmsg (family, dst_len, src_len, tos, table,
    /// protocol, scope, type, flags), then its attributes.
    pub fn route(family: u8, len: u8, table: u8, kind: u8, flags: u32, attrs: &[Vec<u8>]) -> Vec<u8> {
        let mut body = vec![family, len, 0, 0, table, 4, 0, kind];
        body.extend_from_slice(&flags.to_ne_bytes());
        body.extend(attrs.iter().flatten());
        msg(RTM_NEWROUTE, 0x2, 7, &body)
    }

    fn v4(a: &str) -> Vec<u8> {
        a.parse::<Ipv4Addr>().unwrap().octets().to_vec()
    }

    fn v6(a: &str) -> Vec<u8> {
        a.parse::<Ipv6Addr>().unwrap().octets().to_vec()
    }

    fn oif(i: u32) -> Vec<u8> {
        attr(RTA_OIF, &i.to_ne_bytes())
    }

    /// A dump of a policy-routed host, as `ip route show table all` would list it:
    ///
    /// ```text
    /// default via 192.168.1.1 dev eth0                       (main)
    /// 192.168.1.0/24 dev eth0 scope link                     (main)
    /// 81.2.69.0/24 dev eth0 scope link table 100             an on-link /24 only in table 100
    /// default via 87.1.1.1 dev eth0 onlink table 200         a router only in table 200
    /// 0.0.0.0/1 dev tun0 table 51820                         wg-quick's
    /// 10.9.0.0/16 nexthop via 10.0.0.1 dev eth0 nexthop via 10.0.0.2 dev eth1 dead
    /// local 192.168.1.10 dev eth0 table local                not a route here
    /// broadcast 192.168.1.255 dev eth0 table local           nor this
    /// unreachable 203.0.113.0/24                             nor this
    /// 2a02:8070:abcd:1::/64 dev eth0 table 100
    /// default via fe80::1 dev eth0 table 200
    /// 93.184.215.0/24 via inet6 fe80::2 dev eth0             (RTA_VIA)
    /// ```
    ///
    /// then a message for another request, which is not this dump's, and the end.
    pub fn fixture() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(route(FAMILY_V4, 0, 254, RTN_UNICAST, 0, &[attr(RTA_GATEWAY, &v4("192.168.1.1")), oif(2)]));
        b.extend(route(FAMILY_V4, 24, 254, RTN_UNICAST, 0, &[attr(RTA_DST, &v4("192.168.1.0")), oif(2)]));
        b.extend(route(FAMILY_V4, 24, 100, RTN_UNICAST, 0, &[attr(RTA_DST, &v4("81.2.69.0")), oif(2)]));
        // RTNH_F_ONLINK (4) in its flags; the table, past 255 or not, also as RTA_TABLE (15).
        b.extend(route(FAMILY_V4, 0, 200, RTN_UNICAST, 4, &[attr(15, &200u32.to_ne_bytes()), attr(RTA_GATEWAY, &v4("87.1.1.1")), oif(2)]));
        b.extend(route(FAMILY_V4, 1, 252, RTN_UNICAST, 0, &[attr(15, &51820u32.to_ne_bytes()), attr(RTA_DST, &v4("0.0.0.0")), oif(9)]));
        let mut hops = Vec::new();
        for (gw, flags, index) in [("10.0.0.1", 0u8, 2u32), ("10.0.0.2", 1, 3)] {
            let a = attr(RTA_GATEWAY, &v4(gw));
            hops.extend_from_slice(&((8 + a.len()) as u16).to_ne_bytes());
            hops.extend_from_slice(&[flags, 0]);
            hops.extend_from_slice(&index.to_ne_bytes());
            hops.extend(a);
        }
        b.extend(route(FAMILY_V4, 16, 254, RTN_UNICAST, 0, &[attr(RTA_DST, &v4("10.9.0.0")), attr(RTA_MULTIPATH, &hops)]));
        b.extend(route(FAMILY_V4, 32, 255, 2, 0, &[attr(RTA_DST, &v4("192.168.1.10")), oif(2)]));
        b.extend(route(FAMILY_V4, 32, 255, 3, 0, &[attr(RTA_DST, &v4("192.168.1.255")), oif(2)]));
        b.extend(route(FAMILY_V4, 24, 254, 7, 0, &[attr(RTA_DST, &v4("203.0.113.0"))]));
        b.extend(route(FAMILY_V6, 64, 100, RTN_UNICAST, 0, &[attr(RTA_DST, &v6("2a02:8070:abcd:1::")), oif(2)]));
        b.extend(route(FAMILY_V6, 0, 200, RTN_UNICAST, 0, &[attr(RTA_GATEWAY, &v6("fe80::1")), oif(2)]));
        let mut via = u16::from(FAMILY_V6).to_ne_bytes().to_vec();
        via.extend(v6("fe80::2"));
        b.extend(route(FAMILY_V4, 24, 254, RTN_UNICAST, 0, &[attr(RTA_DST, &v4("93.184.215.0")), attr(RTA_VIA, &via), oif(2)]));
        let mut other = route(FAMILY_V4, 24, 254, RTN_UNICAST, 0, &[attr(RTA_DST, &v4("44.0.0.0")), oif(2)]);
        other[8..12].copy_from_slice(&8u32.to_ne_bytes());
        b.extend(other);
        b.extend(msg(NLMSG_DONE, 0x2, 7, &0u32.to_ne_bytes()));
        b
    }

    pub fn names(i: u32) -> Option<String> {
        match i {
            2 => Some("eth0".into()),
            3 => Some("eth1".into()),
            9 => Some("tun0".into()),
            _ => None,
        }
    }

    fn r(dest: &str, len: u8, via: Option<&str>, dev: &str) -> Route {
        Route { dest: dest.parse().unwrap(), len, via: via.map(|v| v.parse().unwrap()), dev: dev.into() }
    }

    #[test]
    fn every_table_is_read_from_the_dump() {
        let mut got = Vec::new();
        assert!(parse(&fixture(), 7, &names, &mut got).unwrap(), "the dump ended");
        assert_eq!(
            got,
            vec![
                r("0.0.0.0", 0, Some("192.168.1.1"), "eth0"),
                r("192.168.1.0", 24, None, "eth0"),
                r("81.2.69.0", 24, None, "eth0"),
                r("0.0.0.0", 0, Some("87.1.1.1"), "eth0"),
                r("0.0.0.0", 1, None, "tun0"),
                r("10.9.0.0", 16, Some("10.0.0.1"), "eth0"),
                r("2a02:8070:abcd:1::", 64, None, "eth0"),
                r("::", 0, Some("fe80::1"), "eth0"),
                r("93.184.215.0", 24, Some("fe80::2"), "eth0"),
            ],
            "not the dead next hop, the local table, the unreachable route, nor another request's"
        );
    }

    #[test]
    fn a_dump_that_is_cut_short_or_refused_is_an_error() {
        let f = fixture();
        let mut got = Vec::new();
        assert!(parse(&f[..f.len() - 3], 7, &names, &mut got).is_err(), "a message that does not fit");
        let err = msg(NLMSG_ERROR, 0, 7, &(-(libc::EPERM)).to_ne_bytes());
        assert_eq!(parse(&err, 7, &names, &mut Vec::new()).unwrap_err().raw_os_error(), Some(libc::EPERM));
        let intr = msg(RTM_NEWROUTE, 0x2 | NLM_F_DUMP_INTR, 7, &[0; RTMSG_LEN]);
        assert_eq!(parse(&intr, 7, &names, &mut Vec::new()).unwrap_err().kind(), io::ErrorKind::Interrupted);
        // Not ended yet: more to read.
        let first = route(FAMILY_V4, 24, 254, RTN_UNICAST, 0, &[attr(RTA_DST, &v4("192.168.1.0")), oif(2)]);
        assert!(!parse(&first, 7, &names, &mut Vec::new()).unwrap());
    }

    /// This machine's own tables, over its own netlink: loopback's routes are in the local table,
    /// which is not routes here, but the dump itself answers and ends.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_kernel_answers_a_dump() {
        let name = |i: u32| {
            let mut buf = [0 as libc::c_char; libc::IF_NAMESIZE];
            // SAFETY: a buffer of IF_NAMESIZE, as if_indextoname asks.
            let p = unsafe { libc::if_indextoname(i, buf.as_mut_ptr()) };
            (!p.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned())
        };
        dump(&name).expect("the kernel answers a route dump");
    }
}
