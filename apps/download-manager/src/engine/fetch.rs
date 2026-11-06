//! Opening the request. A person's fetch is whatever ureq does; an agent's is held to the public
//! internet.
//!
//! An agent that names a URL asks this app, which runs as the person, to fetch it: on the person's
//! machine that reaches the model gateway, the egress doors and the companion's server on the
//! loopback, and anything on the LAN, and the body is saved where the agent can read it. So an
//! agent's fetch refuses any address `yantrik_address_place::place_of` does not call the
//! internet: loopback, private, link-local, CGNAT, unspecified, multicast, and the ranges that
//! carry an IPv4 address inside an IPv6 one.
//!
//! Two things make that hold. The check lives in the resolver ureq connects through, so the
//! address that was checked is the address connected to: there is no second lookup for a rebinding
//! DNS answer to change. And redirects are not followed by ureq: each `Location` is joined,
//! checked for its scheme, and sent through the same resolver, up to [`MOST_HOPS`] times.
//!
//! The person's own add is unchanged: it follows redirects wherever they go, and may fetch from
//! the local network (a NAS, a router's firmware page) as it always could.

use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};

use yantrik_address_place::{place_of, Place};

/// How many redirects an agent's fetch follows.
const MOST_HOPS: usize = 5;

const REFUSED: &str = "an agent may not fetch from this machine or a private network";

/// Whether an agent may connect to `addr`.
fn allowed(addr: &SocketAddr) -> bool {
    if place_of(addr.ip()) == Place::Internet {
        return true;
    }
    // The tests' own web server is on the loopback. Only the ports they list are let through, so
    // a second server the test did not list stands for the machine's own services.
    #[cfg(test)]
    {
        addr.ip().is_loopback() && listed::contains(addr.port())
    }
    #[cfg(not(test))]
    false
}

/// Resolves a name once and hands ureq only the addresses an agent may reach.
struct PublicOnly;

impl ureq::Resolver for PublicOnly {
    fn resolve(&self, netloc: &str) -> io::Result<Vec<SocketAddr>> {
        let kept: Vec<SocketAddr> = netloc.to_socket_addrs()?.filter(allowed).collect();
        if kept.is_empty() {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, REFUSED));
        }
        Ok(kept)
    }
}

/// An agent's URL that names a forbidden address outright, before anything is queued: the
/// literals (`127.0.0.1`, `[::1]`, `localhost`) need no lookup, so they are refused at once. A
/// name that merely resolves to one is caught when the fetch connects.
pub(crate) fn literal_refusal(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let port = parsed.port_or_known_default().unwrap_or(80);
    let ip = match parsed.host()? {
        url::Host::Ipv4(ip) => IpAddr::V4(ip),
        url::Host::Ipv6(ip) => IpAddr::V6(ip),
        url::Host::Domain(name) if name.eq_ignore_ascii_case("localhost") => IpAddr::from([127, 0, 0, 1]),
        url::Host::Domain(_) => return None,
    };
    (!allowed(&SocketAddr::new(ip, port))).then(|| format!("{REFUSED}: {ip}"))
}

/// Send the request for `url`, from byte `resume_from` if that is not zero. `Err` is a sentence
/// for the row.
pub(super) fn open(url: &str, resume_from: u64, agent: bool) -> Result<ureq::Response, String> {
    if !agent {
        return send(ureq::get(url), resume_from);
    }
    let client = ureq::AgentBuilder::new().redirects(0).resolver(PublicOnly).build();
    let mut here = url.to_string();
    for _ in 0..=MOST_HOPS {
        let response = send(client.get(&here), resume_from)?;
        if !(300..400).contains(&response.status()) {
            return Ok(response);
        }
        let Some(location) = response.header("location") else { return Ok(response) };
        let next = url::Url::parse(&here)
            .and_then(|base| base.join(location))
            .map_err(|_| format!("the server sent a redirect this app cannot follow: {location}"))?;
        if !matches!(next.scheme(), "http" | "https") {
            return Err(format!("the server redirected to a {} address; only http and https are fetched", next.scheme()));
        }
        here = next.to_string();
    }
    Err(format!("more than {MOST_HOPS} redirects"))
}

fn send(mut request: ureq::Request, resume_from: u64) -> Result<ureq::Response, String> {
    if resume_from > 0 {
        request = request.set("Range", &format!("bytes={resume_from}-"));
    }
    match request.call() {
        Ok(response) => Ok(response),
        Err(ureq::Error::Status(code, _)) => Err(format!("server said HTTP {code}")),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
pub(crate) mod listed {
    use std::sync::Mutex;

    static PORTS: Mutex<Vec<u16>> = Mutex::new(Vec::new());

    /// Let an agent's fetch reach this loopback port.
    pub(crate) fn allow(port: u16) {
        PORTS.lock().unwrap().push(port);
    }

    /// Take it back when the server that asked goes. The port is what the list is keyed on, and the
    /// OS gives a freed one to the next `bind(0)`: a port left listed after its server shut down
    /// would be handed to whichever server binds it next, listed or not.
    pub(crate) fn deny(port: u16) {
        PORTS.lock().unwrap().retain(|p| *p != port);
    }

    pub(super) fn contains(port: u16) -> bool {
        PORTS.lock().unwrap().contains(&port)
    }
}
