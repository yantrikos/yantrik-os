//! Taking a picture's bytes from a server, carefully.
//!
//! A server decides what it sends back, and Studio runs as the person and files what it gets in a
//! folder every image tool reads. So every body Studio takes as a picture, whichever backend and
//! whichever way it arrives (a ComfyUI `/view`, a hosted service's inline `b64_json`, or the URL
//! such a service offers), is held to the same three rules:
//!
//! - it is read through a cap ([`MOST_BYTES`], 64 MiB), so a server cannot make Studio hold a body
//!   of any size in memory;
//! - it has to be a picture Studio can read (`gallery::png_size`: the PNG and JPEG decoders it is
//!   built with), so a page, a script or a secret fetched from somewhere is never saved as `.png`;
//! - a URL the service offers is fetched only when it is `https` and every address the host
//!   resolves to is the internet (`yantrik_address_place::place_of`), so a reply cannot send
//!   Studio to the machine's own services, the LAN or a metadata address. The address that was
//!   checked is the address connected to (the check is the resolver ureq connects through), and
//!   a redirect is not followed.

use std::io::{self, Read};
use std::net::{SocketAddr, ToSocketAddrs};

use yantrik_address_place::{place_of, Place};

/// The most one picture, or one reply that carries a picture, may be.
pub const MOST_BYTES: u64 = 64 * 1024 * 1024;

/// Read `reader` to its end, refusing more than `most` bytes. One byte past the cap is read to
/// tell a body that is exactly the cap from one that is longer.
pub fn read_capped(reader: impl Read, most: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(most.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| format!("the body could not be read ({e})"))?;
    if bytes.len() as u64 > most {
        return Err(format!("the body is larger than the {} MiB Studio will take", most / (1024 * 1024)));
    }
    Ok(bytes)
}

/// Bytes that Studio can read as a picture, or why not.
pub fn picture(bytes: Vec<u8>, from: &str) -> Result<Vec<u8>, String> {
    if bytes.is_empty() {
        return Err(format!("{from} sent an empty file"));
    }
    if crate::gallery::png_size(&bytes).0.is_none() {
        return Err(format!("what {from} sent is not a picture Studio can read (it reads PNG and JPEG)"));
    }
    Ok(bytes)
}

/// Resolves a name once and hands ureq only a list in which every address is the internet.
struct InternetOnly;

impl ureq::Resolver for InternetOnly {
    fn resolve(&self, netloc: &str) -> io::Result<Vec<SocketAddr>> {
        let all: Vec<SocketAddr> = netloc.to_socket_addrs()?.collect();
        if all.is_empty() || all.iter().any(|addr| place_of(addr.ip()) != Place::Internet) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, REFUSED));
        }
        Ok(all)
    }
}

const REFUSED: &str = "that address is on this machine or a private network";

/// A URL a service offered, refused before anything is connected to when it is not `https`.
pub fn url_allowed(location: &str) -> Result<url::Url, String> {
    let parsed = url::Url::parse(location).map_err(|_| "the image was offered at an address that is not a URL".to_string())?;
    if parsed.scheme() != "https" {
        return Err(format!("the image was offered at a {} address; only https is fetched", parsed.scheme()));
    }
    if parsed.host().is_none() {
        return Err("the image was offered at an address with no host".to_string());
    }
    Ok(parsed)
}

/// Fetch the picture a service offered at `location`.
pub fn fetch_offered(location: &str) -> Result<Vec<u8>, String> {
    let url = url_allowed(location)?;
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .resolver(InternetOnly)
        .timeout(std::time::Duration::from_secs(5 * 60))
        .build();
    let response = agent
        .get(url.as_str())
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("the image was offered at a URL that answered HTTP {code}"),
            ureq::Error::Transport(e) => format!("the image was offered at a URL that could not be fetched ({})", e.kind()),
        })?;
    let bytes = read_capped(response.into_reader(), MOST_BYTES).map_err(|e| format!("the offered image: {e}"))?;
    picture(bytes, "the offered address")
}

#[cfg(test)]
#[path = "fetch_tests.rs"]
mod tests;
