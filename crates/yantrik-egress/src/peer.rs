//! Who is asking: the account that owns the other end of a loopback connection.
//!
//! A unix socket says who is on the other end (`SO_PEERCRED`); a TCP socket does not. But both
//! ends of a loopback connection are sockets on this machine, and the kernel lists every TCP
//! socket with its owner's uid in `/proc/net/tcp` (and `tcp6`). The caller's end is the one whose
//! local address is our peer's and whose remote address is our own; its `uid` column is the
//! account that opened it. Nothing the caller sends is asked.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// The uid owning the socket at `peer` that is connected to `ours`, from the kernel's tables.
pub fn uid_of(peer: SocketAddr, ours: SocketAddr) -> Option<u32> {
    // Through `self`: the unit's `ProcSubset=pid` hides `/proc/net`, which is only a link to it.
    let file = if peer.is_ipv4() { "/proc/self/net/tcp" } else { "/proc/self/net/tcp6" };
    let table = std::fs::read_to_string(file).ok()?;
    find_uid(&table, peer, ours)
}

/// The line in a `/proc/net/tcp`-shaped `table` whose local address is `peer` and remote is `ours`.
pub fn find_uid(table: &str, peer: SocketAddr, ours: SocketAddr) -> Option<u32> {
    for line in table.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        // sl local rem st tx:rx tr:when retrnsmt uid timeout inode
        if cols.len() < 8 {
            continue;
        }
        if parse_addr(cols[1]) == Some(peer) && parse_addr(cols[2]) == Some(ours) {
            return cols[7].parse().ok();
        }
    }
    None
}

/// `0100007F:1F90` → 127.0.0.1:8080; the 32-hex-digit form for IPv6. The kernel writes each
/// 32-bit word in host byte order.
fn parse_addr(s: &str) -> Option<SocketAddr> {
    let (addr, port) = s.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let ip = match addr.len() {
        8 => IpAddr::V4(Ipv4Addr::from(u32::from_str_radix(addr, 16).ok()?.to_ne_bytes())),
        32 => {
            let mut bytes = [0u8; 16];
            for (i, chunk) in bytes.chunks_mut(4).enumerate() {
                let word = u32::from_str_radix(&addr[i * 8..i * 8 + 8], 16).ok()?;
                chunk.copy_from_slice(&word.to_ne_bytes());
            }
            IpAddr::V6(Ipv6Addr::from(bytes))
        }
        _ => return None,
    };
    Some(SocketAddr::new(ip, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Written the way the kernel writes it on a little-endian machine.
    #[cfg(target_endian = "little")]
    #[test]
    fn the_callers_end_is_found_and_its_owner_read() {
        let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
   0: 0100007F:1D1A 00000000:0000 0A 00000000:00000000 00:00000000 00000000   998        0 1000 1 0000000000000000 100 0 0 10 0\n\
   1: 0100007F:D431 0100007F:1D1A 01 00000000:00000000 00:00000000 00000000   990        0 1001 1 0000000000000000 20 4 30 10 -1\n\
   2: 0100007F:1D1A 0100007F:D431 01 00000000:00000000 00:00000000 00000000   998        0 1002 1 0000000000000000 20 4 30 10 -1\n";
        let proxy: SocketAddr = "127.0.0.1:7450".parse().unwrap();
        let caller: SocketAddr = "127.0.0.1:54321".parse().unwrap();
        assert_eq!(find_uid(table, caller, proxy), Some(990), "the caller's socket, not ours");
        assert_eq!(find_uid(table, "127.0.0.1:1".parse().unwrap(), proxy), None);
    }

    #[cfg(target_endian = "little")]
    #[test]
    fn v6_addresses_read_word_by_word() {
        // ::1 port 443
        assert_eq!(parse_addr("00000000000000000000000001000000:01BB"), Some("[::1]:443".parse().unwrap()));
        assert_eq!(parse_addr("zz:01BB"), None);
    }

    /// The real table on this machine: a connection we make ourselves is found, owned by us.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_real_loopback_connection_is_found_with_its_owner() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let ours = listener.local_addr().unwrap();
        let client = std::net::TcpStream::connect(ours).unwrap();
        let (_server, peer) = listener.accept().unwrap();
        assert_eq!(peer, client.local_addr().unwrap());
        // SAFETY: getuid cannot fail.
        assert_eq!(uid_of(peer, ours), Some(unsafe { libc::getuid() }));
    }
}
