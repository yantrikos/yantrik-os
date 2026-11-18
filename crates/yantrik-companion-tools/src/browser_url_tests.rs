//! The browser tools navigate the web, never the person's disk: a local page can frame other
//! local files, and the mind can read the screen back (#942).

use super::validate_url;

#[test]
fn a_local_or_scripted_address_is_refused() {
    for url in [
        "file:///etc/passwd",
        "file:///home/p/.ssh/id_ed25519",
        "FILE:///etc/passwd",
        " file:///etc/passwd",
        "view-source:file:///etc/passwd",
        "javascript:alert(1)",
        "data:text/html,<iframe src=file:///etc/passwd>",
        "chrome://settings",
        "about:blank",
    ] {
        assert!(validate_url(url).is_err(), "{url}");
    }
    assert!(validate_url("https://example.com/a").is_ok());
}
