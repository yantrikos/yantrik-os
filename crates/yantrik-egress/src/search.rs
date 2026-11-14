//! One-time search approvals: the person's yes for one query, bound to the one request that
//! carries it (#669).
//!
//! On a host a `search_bound` rule names, the Mind's search goes out only when the person said
//! yes to that exact query. The yes reaches the proxy as an approval on the control socket
//! (`approve_search`), which answers root and the desktop's owner only, never the mind account, so
//! the Mind cannot approve its own queries. An approval names the host, the port and
//! `sha256(query)` (the hash #667 already journals; the query's text never reaches the proxy) and
//! lasts a short while ([`DEFAULT_TTL`], at most [`MOST_TTL`]). The first request whose one `q`
//! hashes to it spends it: Q1 goes out once, and Q2 or a second Q1 is refused.
//!
//! A `session` or `always` grant in force carries no query. It is mapped the same way: the host
//! that holds the grant approves each search it covers, one at a time. The proxy never holds a
//! standing yes, so a grant revoked on the desktop stops at the next search.
//!
//! Approvals live in memory only. A restart forgets them, and the Mind asks again: closed, never
//! open. An approval is spent when the request is let through, before the upstream answers, so a
//! search whose server does not answer has spent its approval too.

use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

/// How long an approval lasts when the caller says nothing.
pub const DEFAULT_TTL: Duration = Duration::from_secs(60);
/// The longest an approval may last.
pub const MOST_TTL: Duration = Duration::from_secs(300);
/// The most approvals waiting at once. Past it, a new one is refused rather than an old one
/// dropped: nothing the person said is lost without the caller being told.
pub const MOST_PENDING: usize = 64;

struct Approval {
    host: String,
    port: u16,
    sha: [u8; 32],
    until: Instant,
}

#[derive(Default)]
pub struct Approvals {
    pending: Vec<Approval>,
}

/// `sha256(query)`, the query as the proxy decoded it from the request (`crate::request`).
pub fn digest(query: &str) -> [u8; 32] {
    Sha256::digest(query.as_bytes()).into()
}

/// 64 lowercase hex digits, the form #667 journals, and nothing else.
pub fn parse_sha256(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

impl Approvals {
    /// Hold one approval for `host`:`port` and the query hashing to `sha`, for `ttl`.
    pub fn approve(&mut self, host: &str, port: u16, sha: [u8; 32], ttl: Duration, now: Instant) -> Result<Instant, String> {
        self.pending.retain(|a| a.until > now);
        if self.pending.len() >= MOST_PENDING {
            return Err(format!("{MOST_PENDING} search approvals are already waiting; let some be used or run out first"));
        }
        let until = now + ttl.min(MOST_TTL);
        self.pending.push(Approval { host: host.to_string(), port, sha, until });
        Ok(until)
    }

    /// Spend the approval for `query` on `host`:`port`, if one is waiting and has not run out.
    /// `true`: it was there, and now it is gone.
    pub fn spend(&mut self, host: &str, port: u16, query: &str, now: Instant) -> bool {
        self.pending.retain(|a| a.until > now);
        let sha = digest(query);
        match self.pending.iter().position(|a| a.host == host && a.port == port && a.sha == sha) {
            Some(i) => {
                self.pending.swap_remove(i);
                true
            }
            None => false,
        }
    }

    /// How many are waiting, not yet run out.
    pub fn waiting(&self, now: Instant) -> usize {
        self.pending.iter().filter(|a| a.until > now).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_approved_query_is_spent_once() {
        let now = Instant::now();
        let mut a = Approvals::default();
        a.approve("searxng.lan", 8888, digest("rust async"), DEFAULT_TTL, now).unwrap();
        assert!(!a.spend("searxng.lan", 8888, "rust async tutorial", now), "Q2");
        assert!(!a.spend("other.lan", 8888, "rust async", now), "another host");
        assert!(!a.spend("searxng.lan", 443, "rust async", now), "another port");
        assert!(a.spend("searxng.lan", 8888, "rust async", now), "Q1, once");
        assert!(!a.spend("searxng.lan", 8888, "rust async", now), "a second Q1");
    }

    #[test]
    fn an_approval_runs_out_and_never_lasts_past_the_most() {
        let now = Instant::now();
        let mut a = Approvals::default();
        let until = a.approve("searxng.lan", 8888, digest("q"), Duration::from_secs(86_400), now).unwrap();
        assert_eq!(until, now + MOST_TTL, "a day is cut to the most");
        assert!(!a.spend("searxng.lan", 8888, "q", now + MOST_TTL), "run out");
        assert_eq!(a.waiting(now + MOST_TTL), 0);
    }

    #[test]
    fn past_the_most_waiting_a_new_approval_is_refused_and_none_is_dropped() {
        let now = Instant::now();
        let mut a = Approvals::default();
        for i in 0..MOST_PENDING {
            a.approve("searxng.lan", 8888, digest(&format!("q{i}")), DEFAULT_TTL, now).unwrap();
        }
        assert!(a.approve("searxng.lan", 8888, digest("one more"), DEFAULT_TTL, now).is_err());
        assert!(a.spend("searxng.lan", 8888, "q0", now), "the first is still there");
        assert!(a.approve("searxng.lan", 8888, digest("one more"), DEFAULT_TTL, now).is_ok(), "room again");
    }

    #[test]
    fn a_hash_is_64_lowercase_hex_digits_and_nothing_else() {
        let good = "4a1e8b0f4c7e6d2b".repeat(4);
        assert!(parse_sha256(&good).is_some());
        assert_eq!(parse_sha256(&"00".repeat(32)), Some([0; 32]));
        for bad in [&good[..63], &format!("{good}0"), &good.to_ascii_uppercase(), &format!("+{}", &good[1..]), ""] {
            assert!(parse_sha256(bad).is_none(), "{bad}");
        }
        // The hash of the text, as #667 journals it.
        let hex: String = digest("rust async").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(parse_sha256(&hex), Some(digest("rust async")));
    }
}
