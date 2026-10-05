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
//! Plain `http://` only to this machine or the local network. Anywhere else every query would
//! cross the internet in clear text, readable and changeable by anyone on the way, so it has to
//! be `https://`.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// The longest address accepted.
pub const MAX_LEN: usize = 256;

/// Where an address leads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// This machine: `localhost`, 127.0.0.0/8, `::1`.
    Loopback,
    /// The local network: 10/8, 172.16/12, 192.168/16, 100.64/10 (CGNAT, as Tailscale uses),
    /// fc00::/7, and the names reserved for one (`*.local`, `*.home.arpa`).
    Lan,
    /// Anywhere else.
    Internet,
}

/// An address that passed [`check`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearxUrl {
    /// Normalised: no trailing slash. What is saved, shown and sent to harnesses.
    pub url: String,
    /// The host as written, without brackets for IPv6.
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
    let (host, place) = match parsed.host() {
        Some(url::Host::Domain(name)) => (name.to_string(), place_of_name(name)),
        Some(url::Host::Ipv4(ip)) => (ip.to_string(), place_of_ip(IpAddr::V4(ip))),
        Some(url::Host::Ipv6(ip)) => (ip.to_string(), place_of_ip(IpAddr::V6(ip))),
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

fn place_of_name(name: &str) -> Place {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    if name == "localhost" || name.ends_with(".localhost") {
        Place::Loopback
    } else if name.ends_with(".local") || name.ends_with(".home.arpa") {
        Place::Lan
    } else {
        Place::Internet
    }
}

/// Where an IP address leads. Link-local (169.254/16, fe80::/10) is not the local network here:
/// the cloud metadata address is one, and a search service has no business there.
pub fn place_of_ip(ip: IpAddr) -> Place {
    match ip {
        IpAddr::V4(v4) => place_of_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return place_of_v4(v4);
            }
            if v6 == Ipv6Addr::LOCALHOST {
                Place::Loopback
            } else if (v6.segments()[0] & 0xfe00) == 0xfc00 {
                Place::Lan
            } else {
                Place::Internet
            }
        }
    }
}

fn place_of_v4(v4: Ipv4Addr) -> Place {
    let o = v4.octets();
    if v4.is_loopback() {
        Place::Loopback
    } else if v4.is_private() || (o[0] == 100 && (64..128).contains(&o[1])) {
        Place::Lan
    } else {
        Place::Internet
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
        for bad in ["http://search.example.com", "http://8.8.8.8:8888", "http://[2001:db8::1]:8888", "http://169.254.169.254"] {
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
            ("http://searx.local:8888", Place::Lan),
            ("http://nas.home.arpa", Place::Lan),
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
}
