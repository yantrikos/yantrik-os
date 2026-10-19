//! Sending the rewritten request to the account, with its key. The only place a key is used.

use std::io::Read;
use std::time::Duration;

/// What came back: the status, its content type, and the body as it arrives.
pub struct Reply {
    pub status: u16,
    pub content_type: String,
    pub body: Box<dyn Read + Send>,
}

/// Why nothing came back, as a code for the log and a sentence for the harness. Never the key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unreachable {
    pub code: &'static str,
    pub message: String,
}

pub trait Upstream: Send + Sync {
    /// POST `body` to `<base_url>/chat/completions` with `key` as a bearer token.
    fn chat(&self, base_url: &str, key: Option<&str>, body: &[u8]) -> Result<Reply, Unreachable>;
}

/// The real one: ureq, honouring the proxy the shell runs behind.
pub struct Http {
    agent: ureq::Agent,
}

impl Default for Http {
    fn default() -> Self {
        // A thinking model can be silent for minutes before its first token; the connect is quick.
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(600))
            .try_proxy_from_env(true)
            // A redirect is passed back, never followed: the account's address is the only place
            // a call goes, so a local account cannot send it on off this network in Private mode.
            .redirects(0)
            .build();
        Http { agent }
    }
}

/// The account's host as a mind may read it in an error: never a part of the sign-in details,
/// query or fragment (`yantrik_shown_address`, review of #856). "its provider" when the host
/// cannot be told from those details.
fn host(url: &str) -> String {
    match yantrik_shown_address::shown_host(url) {
        host if host.is_empty() => "its provider".to_string(),
        host => host,
    }
}

impl Upstream for Http {
    fn chat(&self, base_url: &str, key: Option<&str>, body: &[u8]) -> Result<Reply, Unreachable> {
        let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
        let mut req = self.agent.post(&url).set("Content-Type", "application/json").set("Accept", "application/json, text/event-stream");
        if let Some(k) = key.filter(|k| !k.is_empty()) {
            req = req.set("Authorization", &format!("Bearer {k}"));
        }
        let reply = |r: ureq::Response| Reply {
            status: r.status(),
            content_type: r.header("content-type").unwrap_or("application/json").to_string(),
            body: Box::new(r.into_reader()),
        };
        match req.send_bytes(body) {
            Ok(r) => Ok(reply(r)),
            Err(ureq::Error::Status(_, r)) => Ok(reply(r)),
            Err(ureq::Error::Transport(t)) => Err(Unreachable {
                code: "unreachable",
                message: format!("Could not reach {} ({}).", host(base_url), t.kind()),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review of #856: "Could not reach …" goes back to the mind that asked, and names the
    /// account's host only as it may be drawn.
    #[test]
    fn could_not_reach_never_names_a_part_of_the_sign_in_details() {
        assert_eq!(host("https://user:pw@aig.example/v1?api-key=x#f"), "aig.example");
        assert_eq!(host("http://u@127.0.0.1:11434/v1"), "127.0.0.1:11434");
        assert_eq!(host("http://u:p@[::1]:4000/v1"), "[::1]:4000");
        assert_eq!(host("https://user:p/w@aig.example/v1"), "its provider");
        assert_eq!(host("https://aig.example/a@b"), "its provider");
        // Through the real send: the address does not parse, so nothing is sent, and the
        // sentence the mind reads does not say "user:p".
        let err = Http::default().chat("https://user:p/w@aig.example/v1", None, b"{}").err().expect("not sent");
        assert!(err.message.starts_with("Could not reach its provider"), "{}", err.message);
        for secret in ["user", "p/w", "u:p"] {
            assert!(!err.message.contains(secret), "{}", err.message);
        }
    }
}
