//! Just enough HTTP/1.1 for a loopback endpoint: one request per connection, a Content-Length
//! body, and a response that ends when the connection closes. Every client a harness uses
//! (Python's urllib and httpx, Node's fetch, Rust's ureq and reqwest) reads that.

use std::io::{BufRead, BufReader, Read, Write};

/// The largest request taken: a turn with images handed over can be big, a request this size is
/// not a turn.
pub const MAX_BODY: usize = 32 * 1024 * 1024;
const MAX_HEAD: usize = 32 * 1024;

#[derive(Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// The bearer token, if the request carries one.
    pub fn bearer(&self) -> Option<&str> {
        let v = self.header("authorization")?.trim();
        let (scheme, token) = v.split_once(' ')?;
        scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
    }
}

/// Read one request. `Err` carries the status to answer with.
pub fn read_request(stream: impl Read) -> Result<Request, (u16, &'static str)> {
    let mut reader = BufReader::new(stream);
    let mut head = 0usize;
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|_| (400, "unreadable request"))?;
    head += line.len();
    let mut parts = line.split_whitespace();
    let (method, path) = match (parts.next(), parts.next()) {
        (Some(m), Some(p)) => (m.to_string(), p.to_string()),
        _ => return Err((400, "no request line")),
    };
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        let n = reader.read_line(&mut h).map_err(|_| (400, "unreadable headers"))?;
        head += n;
        if head > MAX_HEAD {
            return Err((431, "headers too large"));
        }
        let h = h.trim_end_matches(['\r', '\n']);
        if n == 0 || h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let mut req = Request { method, path, headers, body: Vec::new() };
    if req.header("transfer-encoding").is_some() {
        return Err((411, "send a Content-Length"));
    }
    let len: usize = match req.header("content-length") {
        Some(v) => v.parse().map_err(|_| (400, "bad Content-Length"))?,
        None => 0,
    };
    if len > MAX_BODY {
        return Err((413, "request too large"));
    }
    req.body = vec![0; len];
    reader.read_exact(&mut req.body).map_err(|_| (400, "body shorter than its Content-Length"))?;
    Ok(req)
}

pub fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Status",
    }
}

/// The head of a response that ends when the connection closes.
pub fn write_head(out: &mut impl Write, status: u16, content_type: &str) -> std::io::Result<()> {
    write!(out, "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", reason(status))
}

/// A whole JSON response.
pub fn write_json(out: &mut impl Write, status: u16, body: &serde_json::Value) -> std::io::Result<()> {
    write_head(out, status, "application/json")?;
    out.write_all(body.to_string().as_bytes())?;
    out.flush()
}

/// An OpenAI-shaped error, which every client already knows how to show.
pub fn error_body(code: &str, message: &str) -> serde_json::Value {
    serde_json::json!({ "error": { "message": message, "type": "yantrik_gateway", "code": code } })
}
