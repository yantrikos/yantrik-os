//! The Yantrik Mind: a separate program, running as an account of its own, that keeps its
//! settings itself. It takes a provider on the person's memory socket, `POST /provider`
//! (yantrik-mind E.PROV1: person's account only, written into the Mind's own 0600 settings, the
//! key never in chat), so "Use Yantrik models" is sent there instead of written to a file: the
//! gateway's address, `picked` as the model (whichever the person picks in the ask bar), the
//! Mind's own gateway token, and
//! `private_context: true`. Revert sends `{"source": "yantrik-gateway", "remove": true}`, and
//! the token is withdrawn either way. What the Mind must do with these is in the PR for #673.

use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;

use super::{Handoff, Offer, Plan, Token};
use crate::wire::settings::ProviderStoreEntry;

pub(crate) const ID: &str = "mind";
/// Who is talking, in what the Mind is sent.
pub(crate) const SOURCE: &str = "yantrik-gateway";

pub(crate) struct Mind;

/// What the Mind is sent to take Yantrik models.
pub(crate) fn offer_body(offer: &Offer) -> String {
    json!({
        "source": SOURCE,
        // The Mind keeps it as a provider of its own, by this id (yantrik-mind E.PROV2).
        "provider": SOURCE,
        "name": super::GATEWAY_NAME,
        "base_url": offer.base_url(),
        "model": yantrik_gateway::PICKED,
        "api_key": offer.token.0,
        "private_context": true,
    })
    .to_string()
}

/// What the Mind is sent on Revert.
pub(crate) fn revert_body() -> String {
    json!({ "source": SOURCE, "remove": true }).to_string()
}

/// The person's memory socket, only when the mind account owns it (the same check the harness
/// host makes before it hands a credential to it).
fn socket() -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let person = unsafe { libc::geteuid() };
    let path = PathBuf::from(format!("/run/yantrik-mind/{person}/memory.sock"));
    let owner = std::fs::symlink_metadata(&path).ok().map(|m| m.uid());
    owner.is_some_and(yantrik_ipc_transport::mind_door::is_mind).then_some(path)
}

/// `POST /provider` on the memory socket with `body`. `Ok` on a 2xx; otherwise a sentence.
pub(crate) fn post(body: &str) -> Result<(), String> {
    match post_to("/provider", body) {
        Ok(_) => {
            tracing::info!("the Mind took the provider setting");
            Ok(())
        }
        Err(Refused::Status(status)) => {
            Err(format!("The Mind did not take it (HTTP {status}): it may not take Yantrik models this way yet"))
        }
        Err(Refused::Unreachable(why)) => Err(why),
    }
}

/// Why the Mind did not take a POST.
#[derive(Debug)]
pub(crate) enum Refused {
    /// No socket, or it could not be reached: nothing was said to the Mind.
    Unreachable(String),
    /// The Mind answered with this non-2xx status.
    Status(u16),
}

/// `POST <route>` on the person's memory socket (`/provider`, and `/consent` from
/// `ai_accounts::mirror`): the reply's body on a 2xx.
pub(crate) fn post_to(route: &str, body: &str) -> Result<String, Refused> {
    let path = socket().ok_or_else(|| Refused::Unreachable("The Mind's memory socket is not there: start the Mind, then try again".into()))?;
    let mut s = std::os::unix::net::UnixStream::connect(&path).map_err(|e| Refused::Unreachable(format!("could not reach the Mind: {e}")))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let request = format!(
        "POST {route} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(request.as_bytes()).map_err(|e| Refused::Unreachable(format!("could not reach the Mind: {e}")))?;
    let mut answer = String::new();
    let _ = s.take(64 * 1024).read_to_string(&mut answer);
    let status: u16 = answer.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    if (200..300).contains(&status) {
        Ok(answer.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default())
    } else {
        Err(Refused::Status(status))
    }
}

impl Handoff for Mind {
    fn harness(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Yantrik Mind"
    }

    fn files(&self, _home: &Path) -> Vec<PathBuf> {
        Vec::new()
    }

    fn unit(&self) -> Option<&'static str> {
        None
    }

    fn plan(&self, _home: &Path, _provider: &ProviderStoreEntry) -> Result<Plan, String> {
        Err("The Mind keeps its provider itself (Settings → AI & Intelligence → Yantrik Mind); give it Yantrik models instead".into())
    }

    fn plan_gateway(&self, _home: &Path, offer: &Offer) -> Result<Plan, String> {
        let mut plan = super::gateway_plan(self, offer, Vec::new());
        plan.mind_post = Some(Token(offer_body(offer)));
        Ok(plan)
    }
}
