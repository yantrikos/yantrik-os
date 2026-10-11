//! Asking a provider whether a key works, before it is kept.
//!
//! One call, to the provider's own host (`signup::Check`), over HTTPS, following no redirect, and
//! spending no free use (a listing, or the key's own record). The answer is a verdict and the
//! host it came from; the key is never in it, nor in any error.

use super::signup::{Auth, Signup};

/// What the provider said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// It works.
    Accepted,
    /// It works, and the provider is at its limit right now (429): kept, and the pool waits.
    AcceptedResting,
    /// The provider refused it (401 or 403): wrong, cut short, or revoked.
    Refused(u16),
    /// Cloudflare: no account with that id, or the token cannot reach it (404).
    NoSuchAccount,
    /// The provider could not be reached; nothing was sent, or nothing came back.
    Unreachable,
    /// An answer this check does not know how to read.
    Unexpected(u16),
}

impl Verdict {
    /// Whether the key is to be kept.
    pub fn keep(&self) -> bool {
        matches!(self, Verdict::Accepted | Verdict::AcceptedResting)
    }
}

/// The address a check goes to, with Cloudflare's account id filled in.
pub fn check_url(signup: &Signup, account_id: Option<&str>) -> Option<String> {
    let url = signup.check.url;
    if url.contains("{account_id}") {
        let id = account_id?;
        // Only an id of the account shape goes into the path: it cannot move the host or the path.
        if id.len() != 32 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        return Some(url.replace("{account_id}", id));
    }
    Some(url.to_string())
}

/// The host a check goes to, for the sentence that says where.
pub fn check_host(signup: &Signup) -> &'static str {
    signup.check.url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or("the provider")
}

/// Ask the provider. `account_id` is Cloudflare's, and only Cloudflare's.
pub fn check(signup: &Signup, key: &str, account_id: Option<&str>) -> Verdict {
    let Some(url) = check_url(signup, account_id) else {
        return Verdict::NoSuchAccount;
    };
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(20)))
            .http_status_as_error(false)
            .https_only(true)
            .max_redirects(0)
            .build(),
    );
    let request = agent.get(&url);
    let request = match signup.check.auth {
        Auth::Bearer => request.header("Authorization", &format!("Bearer {key}")),
        Auth::GoogApiKey => request.header("x-goog-api-key", key),
    };
    match request.call() {
        Ok(response) => verdict(response.status().as_u16()),
        Err(e) => {
            tracing::info!(provider = signup.id, host = check_host(signup), kind = %error_kind(&e), "a key check could not reach its provider");
            Verdict::Unreachable
        }
    }
}

/// The status, read.
pub fn verdict(status: u16) -> Verdict {
    match status {
        200..=299 => Verdict::Accepted,
        429 => Verdict::AcceptedResting,
        401 | 403 => Verdict::Refused(status),
        404 => Verdict::NoSuchAccount,
        other => Verdict::Unexpected(other),
    }
}

/// What kind of failure, without the error's own text (which can carry the address).
fn error_kind(e: &ureq::Error) -> &'static str {
    match e {
        ureq::Error::Timeout(_) => "timed out",
        ureq::Error::Io(_) => "connection failed",
        ureq::Error::HostNotFound => "host not found",
        _ => "request failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::pool::signup::signup;

    #[test]
    fn a_status_reads_as_a_verdict() {
        assert_eq!(verdict(200), Verdict::Accepted);
        assert_eq!(verdict(429), Verdict::AcceptedResting);
        assert!(verdict(429).keep() && verdict(200).keep());
        assert_eq!(verdict(401), Verdict::Refused(401));
        assert!(!verdict(401).keep() && !verdict(500).keep() && !verdict(302).keep());
    }

    /// The account id goes into the path only in the account's shape: it cannot point the key
    /// at another path or host.
    #[test]
    fn cloudflare_s_account_id_cannot_move_where_the_key_goes() {
        let cf = signup("cloudflare").unwrap();
        let id = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            check_url(cf, Some(id)).unwrap(),
            format!("https://api.cloudflare.com/client/v4/accounts/{id}/ai/models/search")
        );
        for hostile in ["../../../evil", "0123@evil.com/x", "0123456789abcdef0123456789abcdeg", ""] {
            assert_eq!(check_url(cf, Some(hostile)), None, "{hostile}");
        }
        assert_eq!(check_url(cf, None), None);
        assert_eq!(check(cf, "token", Some("not-an-id")), Verdict::NoSuchAccount, "nothing is sent");
        assert_eq!(check_url(signup("groq").unwrap(), None).unwrap(), "https://api.groq.com/openai/v1/models");
    }
}
