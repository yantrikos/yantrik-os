//! The header as it goes over a socket: what `ApiLLM` sends, read from the request itself.

use std::io::{Read, Write};
use std::net::TcpListener;

use crate::traits::LLMBackend;
use crate::types::{ChatMessage, GenerationConfig};
use crate::ApiLLM;

/// One request's head, as a client sent it to a listener that answers 500 and hangs up.
fn head_sent_by(build: impl FnOnce(&str) -> ApiLLM) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut seen = Vec::new();
        let mut buf = [0u8; 4096];
        while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = conn.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            seen.extend_from_slice(&buf[..n]);
        }
        let _ = conn.write_all(b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        String::from_utf8_lossy(&seen).to_ascii_lowercase()
    });
    let llm = build(&url);
    let _ = llm.chat(&[ChatMessage::user("hi")], &GenerationConfig::default(), None);
    server.join().unwrap()
}

#[test]
fn the_gateways_client_sends_the_header_only_while_the_mark_is_set() {
    let gateway = |url: &str| ApiLLM::new(url, Some("fake-token".into()), "picked").for_gateway();
    let marked = {
        let _mine = crate::asked::self_started();
        head_sent_by(gateway)
    };
    assert!(marked.contains("x-yantrik-asked: self"), "{marked}");
    let asked = head_sent_by(gateway);
    assert!(!asked.contains("x-yantrik-asked"), "{asked}");
}

#[test]
fn a_client_of_a_provider_never_sends_it() {
    let _mine = crate::asked::self_started();
    let head = head_sent_by(|url| ApiLLM::new(url, Some("fake-token".into()), "gpt-oss-120b").for_provider("groq"));
    assert!(!head.contains("x-yantrik-asked"), "{head}");
    let plain = head_sent_by(|url| ApiLLM::new(url, Some("fake-token".into()), "m"));
    assert!(!plain.contains("x-yantrik-asked"), "{plain}");
}
