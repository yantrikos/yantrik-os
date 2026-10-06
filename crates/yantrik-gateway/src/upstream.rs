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
            .build();
        Http { agent }
    }
}

fn host(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?', '#']).next().unwrap_or("").rsplit('@').next().unwrap_or("").to_string()
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
