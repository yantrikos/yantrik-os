//! What Studio takes as a picture from a hosted service: only an image, never more than the cap,
//! and an offered URL only when it is https and the internet. Each through the real HTTP path, to
//! an in-process server.

use std::os::unix::fs::PermissionsExt;

use serde_json::json;

use super::server::{Manners, Server};
use super::started;

fn fetched_by(server: &Server) -> Vec<String> {
    server.with(|recorded| recorded.viewed.clone())
}

#[test]
fn a_url_the_service_offers_on_this_machine_is_not_fetched() {
    let server = Server::start(Manners::default());
    let base = server.base_url.clone();
    for offered in [
        // The server's own address, plain and over https: this machine either way.
        format!("{base}/offered"),
        format!("https://{}/offered", base.trim_start_matches("http://")),
    ] {
        let service = Server::start(Manners { offer_url: Some(offered.clone()), ..Manners::default() });
        std::env::set_var("STUDIO_TEST_OFFER_KEY", "dummy");
        let f = started(
            "offer",
            &format!(
                r#"{{"backend":{{"kind":"openai-images","base_url":"{}","model":"m","api_key_env":"STUDIO_TEST_OFFER_KEY"}}}}"#,
                service.base_url
            ),
        );
        f.act("generate", json!({ "prompt": "a lighthouse", "width": 64, "height": 64 })).unwrap();
        let notice = f.settled();
        assert!(notice.contains("could not make the picture"), "{offered}: {notice}");
        assert!(fetched_by(&server).is_empty(), "{offered}: the offered address was fetched");
        assert_eq!(f.app.model(|m| m.gallery.len()), 0, "{offered}: something was filed");
    }
    assert!(fetched_by(&server).is_empty());
}

#[test]
fn bytes_that_are_not_a_picture_are_never_filed() {
    for body in [b"<html>not a picture</html>".to_vec(), Vec::new(), b"\x89PNG\r\n\x1a\n".to_vec()] {
        let server = Server::start(Manners { inline_bytes: Some(body.clone()), ..Manners::default() });
        std::env::set_var("STUDIO_TEST_OFFER_KEY", "dummy");
        let f = started(
            "inline",
            &format!(
                r#"{{"backend":{{"kind":"openai-images","base_url":"{}","model":"m","api_key_env":"STUDIO_TEST_OFFER_KEY"}}}}"#,
                server.base_url
            ),
        );
        f.act("generate", json!({ "prompt": "a lighthouse", "width": 64, "height": 64 })).unwrap();
        let notice = f.settled();
        assert!(notice.contains("not a picture") || notice.contains("empty"), "{body:?}: {notice}");
        assert_eq!(f.app.model(|m| m.gallery.len()), 0, "{body:?}: something was filed");
        assert!(!f.gallery().exists() || crate::gallery::listing(&f.gallery(), 12, false).is_empty());
    }
}

/// The settings file is private: an address in it may carry sign-in details.
#[test]
fn the_choice_is_written_privately() {
    let f = super::fake("private-config");
    f.act("set_backend", json!({ "kind": "comfyui", "base_url": "http://127.0.0.1:8188" })).unwrap();
    let path = f.world.places.settings.clone().unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(!path.with_extension("json.new").exists());
}
