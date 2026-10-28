use std::io::Cursor;
use std::net::TcpListener;

use super::*;

/// A listener nothing should ever connect to, and how many did.
struct Watch {
    listener: TcpListener,
    port: u16,
}

impl Watch {
    fn new() -> Watch {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        Watch { listener, port }
    }

    fn connections(&self) -> usize {
        let mut seen = 0;
        while self.listener.accept().is_ok() {
            seen += 1;
        }
        seen
    }
}

#[test]
fn a_body_over_the_cap_is_refused_and_one_at_the_cap_is_taken() {
    assert_eq!(read_capped(Cursor::new(vec![7u8; 10]), 10).unwrap().len(), 10);
    let over = read_capped(Cursor::new(vec![7u8; 11]), 10).unwrap_err();
    assert!(over.contains("larger than"), "{over}");
    // Never read past the cap plus one byte, whatever the reader holds.
    let mut reader = Cursor::new(vec![0u8; 1000]);
    assert!(read_capped(&mut reader, 10).is_err());
    assert_eq!(reader.position(), 11);
    assert_eq!(MOST_BYTES, 64 * 1024 * 1024);
}

#[test]
fn only_a_picture_studio_can_read_is_taken_as_one() {
    let png = crate::backend::draw("a", 1, 16, 16);
    assert_eq!(picture(png.clone(), "x").unwrap(), png);
    for not_one in [Vec::new(), b"<html>secret</html>".to_vec(), b"#!/bin/sh\nrm -rf ~".to_vec(), png[..8].to_vec()] {
        let refused = picture(not_one.clone(), "the server").unwrap_err();
        assert!(refused.contains("the server"), "{refused}");
    }
}

#[test]
fn an_offered_url_is_fetched_only_over_https_from_the_internet() {
    let watch = Watch::new();
    for offered in [
        format!("http://127.0.0.1:{}/offered", watch.port),
        format!("https://127.0.0.1:{}/offered", watch.port),
        format!("https://localhost:{}/offered", watch.port),
        format!("https://[::1]:{}/offered", watch.port),
        "https://192.168.1.1/offered".to_string(),
        "https://169.254.169.254/latest/meta-data".to_string(),
        "https://10.0.0.1/offered".to_string(),
        "file:///etc/passwd".to_string(),
        "ftp://example.com/x.png".to_string(),
        "not a url".to_string(),
    ] {
        let refused = fetch_offered(&offered).unwrap_err();
        assert!(!refused.is_empty(), "{offered}");
    }
    assert_eq!(watch.connections(), 0, "an address that is not the internet was connected to");
    assert!(url_allowed("http://example.com/x.png").unwrap_err().contains("only https"));
    assert!(url_allowed("https://example.com/x.png").is_ok());
}
