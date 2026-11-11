//! Tests only: the one host the shell reaches through [`super::host`], with a mind attached.

use yantrik_harness::{protocol, Host};

/// The mind these tests attach, and the name it lists itself under.
pub(crate) const MIND: &str = "hermes-1162";
pub(crate) const NAME: &str = "Hermes Agent";

/// The shell's host as the rest of the shell finds it, made once per test process.
pub(crate) fn host_with_a_mind() -> &'static Host {
    super::HOST.get_or_init(|| {
        let host = Host::new(Vec::new());
        host.handle(protocol::ATTACH, &serde_json::json!({ "id": MIND, "name": NAME, "detail": "Hermes 0.14.0" })).expect("the mind attaches");
        host
    })
}
