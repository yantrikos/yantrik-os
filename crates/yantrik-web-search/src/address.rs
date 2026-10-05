//! Whether an address may be the person's search service, and what it is once it may.
//!
//! Every search the person's minds make goes to this address, so what it may be is narrow:
//!
//! - `http` or `https`, nothing else;
//! - no user name or password in it (they would be sent with every query and shown on screen);
//! - no query or fragment: the client adds `/search?q=…`, and anything already there would be
//!   sent along with it;
//! - at most 256 characters;
//! - a trailing slash is dropped, so `http://h:8888/` and `http://h:8888` are the same setting.
//!
//! Plain `http://` only to this machine or the local network, and only where the address itself
//! says so: an IP address on loopback or a private range, or the exact name `localhost`. Any other
//! name could lead anywhere (`evil.com.localhost` and `nas.local` are names like any other until
//! something resolves them), and every query would then cross the internet in clear text,
//! readable and changeable by anyone on the way, so a name has to be `https://` or its IP.
//!
//! Where an IP leads is the egress proxy's own reading of it, [`yantrik_egress::policy::place_of`],
//! so the two never disagree. What that calls never a destination (0.0.0.0, `[::]`, link-local,
//! the cloud metadata address, multicast, broadcast) is refused over `https://` too.

use std::net::IpAddr;

use yantrik_egress::policy;

/// The longest address accepted.
pub const MAX_LEN: usize = 256;

/// Where an address leads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// This machine: the exact name `localhost`, 127.0.0.0/8, `::1`, and 127/8 mapped into IPv6.
    Loopback,
    /// The local network, written as an IP: 10/8, 172.16/12, 192.168/16, 100.64/10 (CGNAT, as
    /// Tailscale uses), fc00::/7. Never a name.
    Lan,
    /// Anywhere else, and every name but `localhost`.
    Internet,
}

/// An address that passed [`check`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearxUrl {
    /// Normalised: no trailing slash. What is saved, shown and sent to harnesses.
    pub url: String,
    /// The host as it stands in [`SearxUrl::url`], without brackets for IPv6. The egress rule is
    /// for exactly this, so it matches what a client connecting to the URL asks for.
    pub host: String,
    /// The port, explicit or the scheme's default.
    pub port: u16,
    /// Plain `http://`.
    pub http: bool,
    pub place: Place,
}

/// Check `raw` as the person's search service address. The error is a sentence for the screen.
pub fn check(raw: &str) -> Result<SearxUrl, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("Enter the address of your SearXNG, for example http://192.168.1.20:8888.".into());
    }
    if raw.len() > MAX_LEN {
        return Err(format!("That address is longer than {MAX_LEN} characters."));
    }
    let parsed = url::Url::parse(raw).map_err(|_| {
        "That is not a web address. It should look like http://192.168.1.20:8888 or https://search.example.com.".to_string()
    })?;
    let http = match parsed.scheme() {
        "http" => true,
        "https" => false,
        other => return Err(format!("Use http:// or https://, not {other}://.")),
    };
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("Leave the user name and password out of the address: they would go with every search.".into());
    }
    if parsed.query().is_some() {
        return Err("Leave out the ?… part: the search is added to the address.".into());
    }
    if parsed.fragment().is_some() {
        return Err("Leave out the #… part of the address.".into());
    }
    let Some(host) = parsed.host_str() else {
        return Err("The address needs a host name or an IP address.".into());
    };
    let host = host.trim_start_matches('[').trim_end_matches(']').to_string();
    let place = match parsed.host() {
        Some(url::Host::Domain("localhost")) => Place::Loopback,
        Some(url::Host::Domain(_)) if http => {
            return Err(format!(
                "Use the address's IP, or https://, for {host}. Plain http:// goes by name only to \
                 localhost: any other name could lead off your network, and your searches would \
                 cross the internet in clear text."
            ))
        }
        Some(url::Host::Domain(_)) => Place::Internet,
        Some(url::Host::Ipv4(ip)) => place_of_ip(IpAddr::V4(ip)).ok_or_else(|| never(&host))?,
        Some(url::Host::Ipv6(ip)) => place_of_ip(IpAddr::V6(ip)).ok_or_else(|| never(&host))?,
        None => return Err("The address needs a host name or an IP address.".into()),
    };
    if http && place == Place::Internet {
        return Err(format!(
            "Use https:// for {host}. Plain http:// is only for this machine or your local network: \
             anywhere else your searches would cross the internet in clear text."
        ));
    }
    let port = parsed.port_or_known_default().unwrap_or(if http { 80 } else { 443 });
    let url = parsed.as_str().trim_end_matches('/').to_string();
    Ok(SearxUrl { url, host, port, http, place })
}

fn never(host: &str) -> String {
    format!(
        "{host} can never be a search service: it is unspecified, link-local (as the cloud \
         metadata address is), multicast or broadcast."
    )
}

/// Where an IP address leads, or `None` for one that is never a destination. Loopback is this
/// machine; everything else is the egress proxy's reading ([`policy::place_of`]), which also has
/// no place for loopback because minds reach this machine without it.
pub fn place_of_ip(ip: IpAddr) -> Option<Place> {
    let loopback = match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
    };
    if loopback {
        return Some(Place::Loopback);
    }
    match policy::place_of(ip) {
        policy::Place::Forbidden => None,
        policy::Place::Lan => Some(Place::Lan),
        policy::Place::Internet => Some(Place::Internet),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_and_https() {
        for bad in ["ftp://192.168.1.2", "file:///etc/passwd", "javascript:alert(1)", "192.168.1.2:8888", "searx"] {
            assert!(check(bad).is_err(), "{bad} must be refused");
        }
        assert!(check("https://search.example.com").is_ok());
        assert!(check("http://192.168.1.2:8888").is_ok());
    }

    #[test]
    fn a_user_name_or_password_is_refused() {
        for bad in ["https://me:secret@search.example.com", "http://me@192.168.1.2:8888", "http://:pw@10.0.0.2"] {
            let e = check(bad).unwrap_err();
            assert!(e.contains("user name and password"), "{bad}: {e}");
        }
    }

    #[test]
    fn a_query_or_fragment_is_refused() {
        assert!(check("http://192.168.1.2:8888/?q=x").is_err());
        assert!(check("http://192.168.1.2:8888/?").is_err());
        assert!(check("http://192.168.1.2:8888/#top").is_err());
        assert!(check("http://192.168.1.2:8888/#").is_err());
    }

    #[test]
    fn plain_http_to_a_public_host_is_refused_and_says_why() {
        for bad in ["http://search.example.com", "http://8.8.8.8:8888", "http://[2001:db8::1]:8888"] {
            let e = check(bad).unwrap_err();
            assert!(e.contains("clear text") && e.contains("https://"), "{bad}: {e}");
        }
        let ok = check("https://search.example.com").unwrap();
        assert_eq!((ok.place, ok.http, ok.port), (Place::Internet, false, 443));
    }

    #[test]
    fn plain_http_on_this_machine_or_the_lan_is_accepted() {
        for (url, place) in [
            ("http://127.0.0.1:8888", Place::Loopback),
            ("http://localhost:8888", Place::Loopback),
            ("http://[::1]:8888", Place::Loopback),
            ("http://192.168.4.42:8888", Place::Lan),
            ("http://10.1.2.3", Place::Lan),
            ("http://172.20.0.5:8080", Place::Lan),
            ("http://100.101.102.103:8888", Place::Lan),
            ("http://[fd00::42]:8888", Place::Lan),
        ] {
            let got = check(url).unwrap_or_else(|e| panic!("{url}: {e}"));
            assert_eq!(got.place, place, "{url}");
            assert!(got.http);
        }
        assert_eq!(check("http://10.1.2.3").unwrap().port, 80);
        assert_eq!(check("http://[fd00::42]:8888").unwrap().host, "fd00::42");
    }

    #[test]
    fn it_is_normalised() {
        assert_eq!(check("  http://192.168.4.42:8888/  ").unwrap().url, "http://192.168.4.42:8888");
        assert_eq!(check("http://192.168.4.42:8888").unwrap().url, "http://192.168.4.42:8888");
        assert_eq!(check("https://example.com/searx/").unwrap().url, "https://example.com/searx");
        assert_eq!(check("HTTPS://Search.Example.COM:443/").unwrap().url, "https://search.example.com");
    }

    #[test]
    fn at_most_256_characters() {
        let long = format!("https://example.com/{}", "a".repeat(MAX_LEN));
        assert!(check(&long).unwrap_err().contains("256"));
        let fits = format!("https://example.com/{}", "a".repeat(MAX_LEN - 20));
        assert_eq!(fits.len(), MAX_LEN);
        assert!(check(&fits).is_ok());
    }

    /// A name is only a name until something resolves it: plain http goes by name to `localhost`
    /// alone, and the egress rule's `lan` is never taken from one.
    #[test]
    fn plain_http_by_name_is_only_localhost() {
        for bad in [
            "http://evil.com.localhost",
            "http://searx.localhost:8888",
            "http://searx.local:8888",
            "http://nas.home.arpa",
            "http://localhost.",
            "http://bücher.local",
            "http://LOCALHOST.example",
        ] {
            let e = check(bad).unwrap_err();
            assert!(e.starts_with("Use the address's IP, or https://"), "{bad}: {e}");
        }
        assert_eq!(check("http://LocalHost:8888").unwrap().place, Place::Loopback);
        // Over https a name is the internet, wherever it claims to be.
        for name in ["https://evil.com.localhost", "https://searx.local", "https://nas.home.arpa"] {
            assert_eq!(check(name).unwrap().place, Place::Internet, "{name}");
        }
        let idn = check("https://bücher.example/").unwrap();
        assert_eq!((idn.host.as_str(), idn.url.as_str()), ("xn--bcher-kva.example", "https://xn--bcher-kva.example"));
    }

    /// Every way of writing an IP is read as the IP it is, and placed by it.
    #[test]
    fn ip_forms_are_read_as_the_address_they_are() {
        for (raw, url, place) in [
            ("http://3232235777:8888", "http://192.168.1.1:8888", Place::Lan),
            ("http://0300.0250.0.1", "http://192.168.0.1", Place::Lan),
            ("http://0xc0.0xa8.0x00.0x01", "http://192.168.0.1", Place::Lan),
            ("http://2130706433:8888", "http://127.0.0.1:8888", Place::Loopback),
            ("http://127.1", "http://127.0.0.1", Place::Loopback),
            ("http://192.168.1.2.:8888", "http://192.168.1.2:8888", Place::Lan),
            ("http://[::ffff:127.0.0.1]:8888", "http://[::ffff:7f00:1]:8888", Place::Loopback),
        ] {
            let got = check(raw).unwrap_or_else(|e| panic!("{raw}: {e}"));
            assert_eq!((got.url.as_str(), got.place), (url, place), "{raw}");
        }
        // Public, however it is written, is still not for plain http.
        for bad in ["http://134744072", "http://0x08080808", "http://[::ffff:8.8.8.8]"] {
            assert!(check(bad).unwrap_err().contains("clear text"), "{bad}");
        }
    }

    /// The egress rule is made from `host`, so it has to be the host the URL itself carries.
    #[test]
    fn the_host_is_the_one_in_the_url() {
        let mapped = check("http://[::ffff:192.168.1.2]:8888/").unwrap();
        assert_eq!(mapped.url, "http://[::ffff:c0a8:102]:8888");
        assert_eq!(mapped.host, "::ffff:c0a8:102");
        assert_eq!(mapped.place, Place::Lan);
        assert!(mapped.url.contains(&format!("[{}]", mapped.host)));
        let v4 = check("http://0300.0250.0.1:8080").unwrap();
        assert_eq!(v4.host, "192.168.0.1");
        assert!(v4.url.contains(&v4.host));
    }

    /// What the egress proxy calls never a destination is refused whatever the scheme.
    #[test]
    fn never_a_destination_is_refused_over_https_too() {
        for bad in [
            "https://0.0.0.0",
            "http://0.0.0.0:8888",
            "http://0:8888",
            "https://[::]",
            "https://169.254.169.254",
            "http://169.254.169.254",
            "https://[fe80::1]",
            "https://224.0.0.1",
            "https://255.255.255.255",
            "https://[ff02::1]",
            "https://[::ffff:169.254.169.254]",
            "https://[64:ff9b::a9fe:a9fe]",
        ] {
            let e = check(bad).unwrap_err();
            assert!(e.contains("can never be a search service"), "{bad}: {e}");
        }
    }

    /// The classification is the proxy's: the two agree on every LAN and internet address.
    #[test]
    fn place_of_ip_agrees_with_the_egress_proxy() {
        for ip in ["10.0.0.1", "172.16.0.1", "192.168.0.1", "100.64.0.1", "fd00::1", "8.8.8.8", "2001:db8::1", "64:ff9b::c0a8:1"] {
            let ip: IpAddr = ip.parse().unwrap();
            let ours = place_of_ip(ip).unwrap();
            let theirs = policy::place_of(ip);
            assert_eq!(ours == Place::Lan, theirs == policy::Place::Lan, "{ip}");
        }
        assert_eq!(place_of_ip("::1".parse().unwrap()), Some(Place::Loopback));
        assert_eq!(place_of_ip("127.9.9.9".parse().unwrap()), Some(Place::Loopback));
    }
}
