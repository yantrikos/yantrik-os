//! The head of a proxy request: where the caller wants to go.
//!
//! Two forms are understood. `CONNECT host:port HTTP/1.1` opens a tunnel, and is how every HTTPS
//! request arrives — the proxy sees the name and the port, never what goes through. An
//! absolute-form request (`GET http://host:port/path HTTP/1.1`) is plain HTTP, forwarded only
//! where a rule says so (a LAN model server). Anything else is refused.
//!
//! The head is read whole before anything is decided, with a size limit, so a caller cannot hold
//! a half-read request open or send one that is larger than a head.

/// The largest request head read.
pub const MOST_HEAD: usize = 16 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    /// A tunnel to `host:port`.
    Connect { host: String, port: u16 },
    /// A plain HTTP request; `head` is the whole head rewritten for the origin: the path alone on
    /// the request line, `Host` the authority that was decided on, the proxy's own headers
    /// dropped, and `Connection: close`. `body` is how many bytes follow it (`Content-Length`):
    /// exactly that many are forwarded and no more, so one tunnel carries one request, to the
    /// host that was decided on. `queries` is every `q=` parameter of the path, percent-decoded,
    /// empty when the path has none: what a search rule binds to what is sent.
    Http { host: String, port: u16, head: Vec<u8>, body: u64, queries: Vec<String> },
}

/// Why a head was not a request this proxy serves.
#[derive(Debug, PartialEq, Eq)]
pub struct Bad(pub &'static str);

/// Where the head ends (`\r\n\r\n`), if it has.
pub fn head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

/// Read what `head` asks for.
pub fn parse(head: &[u8]) -> Result<Target, Bad> {
    // Every line ends CRLF, and nothing else is a line end: a bare CR or LF, read one way here and
    // another way upstream, is how a second request hides inside the first.
    for (i, &b) in head.iter().enumerate() {
        let ok = match b {
            b'\r' => head.get(i + 1) == Some(&b'\n'),
            b'\n' => i > 0 && head[i - 1] == b'\r',
            _ => true,
        };
        if !ok {
            return Err(Bad("a line in the head does not end CRLF"));
        }
    }
    let text = std::str::from_utf8(head).map_err(|_| Bad("the request is not text"))?;
    let mut lines = text.split("\r\n");
    let first = lines.next().unwrap_or_default();
    let mut parts = first.split(' ');
    let (method, target, version) = (parts.next(), parts.next(), parts.next());
    let (Some(method), Some(target), Some(version)) = (method, target, version) else {
        return Err(Bad("the request line is not METHOD TARGET VERSION"));
    };
    if parts.next().is_some() || !version.starts_with("HTTP/1.") {
        return Err(Bad("the request line is not METHOD TARGET VERSION"));
    }
    if method == "CONNECT" {
        let (host, port) = host_port(target, None)?;
        return Ok(Target::Connect { host, port });
    }
    if !method.bytes().all(|b| b.is_ascii_uppercase()) || method.is_empty() {
        return Err(Bad("not a method"));
    }
    let rest = target.strip_prefix("http://").ok_or(Bad("only CONNECT, or plain http:// in absolute form"))?;
    let (authority, path) = match rest.find(['/', '?']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let path = if path.starts_with('?') { format!("/{path}") } else { path.to_string() };
    let (host, port) = host_port(authority, Some(80))?;
    if path.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return Err(Bad("the path is not one word"));
    }
    let mut out = format!("{method} {path} {version}\r\n");
    // The authority that was decided on, and no other: a Host header naming another site on the
    // same address would reach it.
    let authority = if host.contains(':') { format!("[{host}]") } else { host.clone() };
    out.push_str(&if port == 80 { format!("Host: {authority}\r\n") } else { format!("Host: {authority}:{port}\r\n") });
    let mut body = 0u64;
    let mut lengths = 0;
    for line in lines.take_while(|l| !l.is_empty()) {
        if line.starts_with([' ', '\t']) {
            return Err(Bad("a folded header line"));
        }
        let (name, value) = line.split_once(':').ok_or(Bad("a header line without a colon"))?;
        if name.is_empty() || name.bytes().any(|b| !(b.is_ascii_alphanumeric() || b"-_".contains(&b))) {
            return Err(Bad("not a header name"));
        }
        // No control byte in a value but tab: a NUL or a stray escape reads differently upstream.
        if value.bytes().any(|b| (b < 0x20 && b != b'\t') || b == 0x7f) {
            return Err(Bad("a control character in a header"));
        }
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "host" | "connection" | "keep-alive" | "upgrade" | "te" | "trailer" => continue,
            _ if lower.starts_with("proxy-") => continue,
            // Only a body whose length is said: a chunked one has its end where the upstream
            // reads it, which is not always where this proxy would.
            "transfer-encoding" => return Err(Bad("a chunked body; send it with Content-Length")),
            // Digits only, and written again in the one form: `+5` or `5\x0b` read as 5 here and
            // as something else upstream would put the body's end somewhere else.
            "content-length" => {
                let v = value.trim_matches([' ', '\t']);
                if v.is_empty() || v.len() > 18 || !v.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(Bad("Content-Length is not a number"));
                }
                body = v.parse().map_err(|_| Bad("Content-Length is not a number"))?;
                lengths += 1;
                continue;
            }
            _ => {}
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    if lengths > 1 {
        return Err(Bad("more than one Content-Length"));
    }
    if lengths == 1 {
        out.push_str(&format!("Content-Length: {body}\r\n"));
    }
    out.push_str("Connection: close\r\n\r\n");
    Ok(Target::Http { host, port, head: out.into_bytes(), body, queries: queries_of(&path) })
}

/// The percent-decoded value of every `q` parameter of `path`, in order. All of them, not the
/// first: a server that reads the last of two would otherwise send one nobody checked. A pair
/// with no `=` is skipped unless it is a bare `q` (an empty query), not the end of the search.
///
/// Pairs are split on `&` only, and everything after the first `?` is the query, `#` included:
/// a `;` or a `#` stays inside the value, so text after one is part of what must match what was
/// approved. Splitting on them here when the server does not (SearXNG's does not) would let
/// `q=approved;anything` out as `approved`. The path is forwarded byte for byte; this is only
/// read to bind a search to what is sent.
fn queries_of(path: &str) -> Vec<String> {
    let Some((_, query)) = path.split_once('?') else { return Vec::new() };
    query
        .split('&')
        .map(|pair| pair.split_once('=').unwrap_or((pair, "")))
        .filter(|(name, _)| percent_decode(name) == "q")
        .map(|(_, value)| percent_decode(value))
        .collect()
}

/// `+` is a space and `%XX` is the byte it names, as a form writes a query; anything else stays.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 3;
                    }
                    _ => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `host:port`, `[v6]:port`, or `host` with a default port. The host is lowercased, and must be a
/// name or an address — no user part, no path, nothing a resolver would read as more.
fn host_port(s: &str, default: Option<u16>) -> Result<(String, u16), Bad> {
    if s.contains('@') || s.is_empty() {
        return Err(Bad("not host:port"));
    }
    let (host, port) = if let Some(rest) = s.strip_prefix('[') {
        let (h, after) = rest.split_once(']').ok_or(Bad("an unclosed [address]"))?;
        let port = match after.strip_prefix(':') {
            Some(p) => Some(p),
            None if after.is_empty() => None,
            None => return Err(Bad("not host:port")),
        };
        (h, port)
    } else {
        match s.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (s, None),
        }
    };
    let port = match port {
        Some(p) => p.parse::<u16>().ok().filter(|p| *p != 0).ok_or(Bad("not a port"))?,
        None => default.ok_or(Bad("a tunnel needs a port"))?,
    };
    let host = host.to_ascii_lowercase();
    let ok = !host.is_empty()
        && host.len() <= 253
        && host.bytes().all(|b| b.is_ascii_alphanumeric() || b"-.:_".contains(&b))
        && !host.starts_with('.')
        && !host.starts_with('-');
    if !ok {
        return Err(Bad("not a host name or address"));
    }
    Ok((host.trim_end_matches('.').to_string(), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tunnel_names_its_host_and_port() {
        let t = parse(b"CONNECT API.Example.com:443 HTTP/1.1\r\nHost: api.example.com:443\r\n\r\n").unwrap();
        assert_eq!(t, Target::Connect { host: "api.example.com".into(), port: 443 });
        let t = parse(b"CONNECT [2001:db8::1]:8443 HTTP/1.1\r\n\r\n").unwrap();
        assert_eq!(t, Target::Connect { host: "2001:db8::1".into(), port: 8443 });
        assert!(parse(b"CONNECT api.example.com HTTP/1.1\r\n\r\n").is_err(), "a port is required");
    }

    #[test]
    fn plain_http_is_rewritten_for_the_origin_one_request_per_tunnel() {
        let t = parse(
            b"POST http://192.168.4.20:11434/api/chat?x=1 HTTP/1.1\r\nHost: 192.168.4.20:11434\r\nProxy-Authorization: Basic xyz\r\nConnection: keep-alive\r\nContent-Length: 2\r\n\r\n",
        )
        .unwrap();
        let Target::Http { host, port, head, body, queries } = t else { panic!() };
        assert!(queries.is_empty(), "no q= parameter");
        assert_eq!((host.as_str(), port, body), ("192.168.4.20", 11434, 2));
        let head = String::from_utf8(head).unwrap();
        assert!(head.contains("Host: 192.168.4.20:11434\r\n"));
        assert!(head.starts_with("POST /api/chat?x=1 HTTP/1.1\r\n"), "{head}");
        assert!(!head.to_ascii_lowercase().contains("proxy-authorization"));
        assert!(!head.contains("keep-alive"));
        assert!(head.contains("Content-Length: 2\r\n"));
        assert!(head.ends_with("Connection: close\r\n\r\n"));
        let Target::Http { port, body, .. } = parse(b"GET http://example.com HTTP/1.1\r\n\r\n").unwrap() else { panic!() };
        assert_eq!((port, body), (80, 0));
    }

    /// Found by the security review: each of these reached the upstream as more than the one
    /// request, or as a request to another site on the same address.
    #[test]
    fn no_request_hides_another_or_names_another_site() {
        let Target::Http { head, .. } = parse(b"GET http://a.example/x HTTP/1.1\r\nHost: evil.example\r\n\r\n").unwrap() else { panic!() };
        let head = String::from_utf8(head).unwrap();
        assert!(head.contains("Host: a.example\r\n") && !head.contains("evil"), "{head}");
        for bad in [
            &b"GET http://a.example/ HTTP/1.1\r\nX-A: 1\nGET /smuggled HTTP/1.1\r\n\r\n"[..],
            b"GET http://a.example/ HTTP/1.1\r\nX-A: 1\rX\r\n\r\n",
            b"GET http://a.example/ HTTP/1.1\r\nX-A: 1\r\n folded\r\n\r\n",
            b"POST http://a.example/ HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
            b"POST http://a.example/ HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\n",
            b"POST http://a.example/ HTTP/1.1\r\nContent-Length: -1\r\n\r\n",
            b"GET http://a.example/ HTTP/1.1\r\nBad Name: 1\r\n\r\n",
            b"POST http://a.example/ HTTP/1.1\r\nContent-Length: +5\r\n\r\n",
            b"POST http://a.example/ HTTP/1.1\r\nContent-Length: 5\x0b\r\n\r\n",
            b"GET http://a.example/ HTTP/1.1\r\nX-A: a\x00b\r\n\r\n",
        ] {
            assert!(parse(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn anything_else_is_refused() {
        for bad in [
            &b"GET /path HTTP/1.1\r\n\r\n"[..],
            b"GET https://example.com/ HTTP/1.1\r\n\r\n",
            b"CONNECT user@example.com:443 HTTP/1.1\r\n\r\n",
            b"CONNECT example.com:0 HTTP/1.1\r\n\r\n",
            b"CONNECT example.com:99999 HTTP/1.1\r\n\r\n",
            b"CONNECT exa mple.com:443 HTTP/1.1\r\n\r\n",
            b"CONNECT example.com/x:443 HTTP/1.1\r\n\r\n",
            b"CONNECT example.com:443 SPDY/3\r\n\r\n",
            b"\xff\xfe\r\n\r\n",
        ] {
            assert!(parse(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn every_q_parameter_is_kept_percent_decoded() {
        let queries = |head: &[u8]| match parse(head).unwrap() {
            Target::Http { queries, .. } => queries,
            _ => panic!(),
        };
        assert_eq!(queries(b"GET http://searxng.lan:8888/search?q=rust+async&x=1 HTTP/1.1\r\n\r\n"), ["rust async"]);
        assert_eq!(queries(b"GET http://searxng.lan:8888/search?x=1&q=rust%20async HTTP/1.1\r\n\r\n"), ["rust async"]);
        assert!(queries(b"GET http://searxng.lan:8888/search?x=1 HTTP/1.1\r\n\r\n").is_empty());
        // A pair with no `=` before it does not hide the query behind it.
        assert_eq!(queries(b"GET http://searxng.lan:8888/search?flag&q=other HTTP/1.1\r\n\r\n"), ["other"]);
        // A second `q`, or one spelled `%71`, is still a query.
        assert_eq!(queries(b"GET http://searxng.lan:8888/search?q=ok&q=other HTTP/1.1\r\n\r\n"), ["ok", "other"]);
        assert_eq!(queries(b"GET http://searxng.lan:8888/search?%71=other HTTP/1.1\r\n\r\n"), ["other"]);
        // A `;` or a `#` is part of the value, as the server reads it; a bare `q` is an empty one.
        assert_eq!(queries(b"GET http://searxng.lan:8888/search?q=ok;secret HTTP/1.1\r\n\r\n"), ["ok;secret"]);
        assert_eq!(queries(b"GET http://searxng.lan:8888/search?q=ok#secret HTTP/1.1\r\n\r\n"), ["ok#secret"]);
        assert_eq!(queries(b"GET http://searxng.lan:8888/search?q&q=ok HTTP/1.1\r\n\r\n"), ["", "ok"]);
    }

    #[test]
    fn the_head_ends_at_the_blank_line() {
        assert_eq!(head_end(b"CONNECT a:1 HTTP/1.1\r\n\r\nrest"), Some(24));
        assert_eq!(head_end(b"CONNECT a:1 HTTP/1.1\r\n"), None);
    }
}
