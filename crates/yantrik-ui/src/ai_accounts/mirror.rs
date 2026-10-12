//! The person's AI-accounts sharing, mirrored into the Yantrik Mind (design/specs/consent-mirror.md):
//! `POST /consent` on the person's memory socket, sent the way `provider_handoff::mind` sends
//! `/provider`. The Mind follows the OS's private-context rule with it: a private-grounded turn
//! goes to a cloud provider only when the person agreed to share with it.
//!
//! - The body is a full snapshot, `{"source":"yantrik-os","accounts":[{"provider":…,"share":…}]}`:
//!   one entry per catalogue provider of the person's cloud accounts ([`snapshot`]).
//! - Sent only from the shell's in-memory consent ([`consent::held`]), after the person's own
//!   press (Settings → AI accounts, onboarding's sharing step: [`changed`]), never from a file
//!   watch or a reload of `ai-accounts.json`, which a harness could write (#675 finding 4).
//! - Debounced: each snapshot restarts the Mind, so it is sent once, [`DEBOUNCE`] after the last
//!   change; within one attach, one the Mind already acknowledged is not sent again.
//! - On every attach of the Mind it is sent unconditionally ([`mind_attached`]): the Mind may
//!   have lost it (a reinstall, a restored backup). The one exception is the loop guard: the
//!   re-attach that follows a /consent, within [`LOOP_GUARD`] of the ack, carrying what the Mind
//!   said it saved.
//! - No socket (no Mind, or not running): kept, and sent at the next attach. A press never fails.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::account::Account;
use super::consent::{self, Consent, Sharing};

/// Who is talking, in what the Mind is sent.
pub const SOURCE: &str = "yantrik-os";
/// How long after the last change a snapshot is sent.
pub const DEBOUNCE: Duration = Duration::from_secs(1);
/// How long after an ack the Mind's own re-attach is not answered with the same snapshot.
pub const LOOP_GUARD: Duration = Duration::from_secs(10);

/// One provider's line in the snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub provider: String,
    pub share: bool,
}

/// What the Mind is sent, whole every time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    pub source: &'static str,
    pub accounts: Vec<Entry>,
}

impl Snapshot {
    pub fn body(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// The catalogue provider id the Mind knows an account by, case-folded (`nvidia-nim` is `nim`).
/// `None` for an account on this machine or the network (the Mind keeps those itself) and for a
/// custom endpoint, which is no catalogue provider.
pub fn provider_id(account: &Account) -> Option<String> {
    if account.local {
        return None;
    }
    let id = account.provider_type.trim().to_lowercase();
    match id.as_str() {
        "" | "custom" => None,
        "nvidia-nim" => Some("nim".into()),
        _ => Some(id),
    }
}

/// The snapshot for these accounts, in their order: `share` only where [`Consent::sharing_for`]
/// is Share for every account of that provider.
pub fn snapshot(consent: &Consent, accounts: &[Account]) -> Snapshot {
    let mut out: Vec<Entry> = Vec::new();
    for a in accounts {
        let Some(provider) = provider_id(a) else { continue };
        let share = consent.sharing_for(a) == Sharing::Share;
        match out.iter_mut().find(|e| e.provider == provider) {
            Some(e) => e.share &= share,
            None => out.push(Entry { provider, share }),
        }
    }
    Snapshot { source: SOURCE, accounts: out }
}

/// The Mind's reply: `{"saved":true,"shared":[…],"restarting":bool}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Reply {
    #[serde(default)]
    pub saved: bool,
    #[serde(default)]
    pub shared: Vec<String>,
    #[serde(default)]
    pub restarting: bool,
}

impl Reply {
    /// The reply in a response body (the JSON object in it, should it come chunked).
    pub fn parse(body: &str) -> Option<Reply> {
        let (start, end) = (body.find('{')?, body.rfind('}')?);
        serde_json::from_str(body.get(start..=end)?).ok()
    }

    /// Whether the Mind says it saved `sent`: saved, and sharing with nothing `sent` did not
    /// share. (An id it does not know it ignores, so it may share less.)
    fn saved(&self, sent: &Snapshot) -> bool {
        self.saved
            && self.shared.iter().all(|p| sent.accounts.iter().any(|e| e.share && e.provider == p.to_lowercase()))
    }
}

/// When to send, apart from the sending: pure, with the clock passed in.
#[derive(Debug, Default)]
pub struct Mirror {
    /// The latest change and when it is due.
    pending: Option<(Snapshot, Instant)>,
    /// What the Mind acknowledged within this attach.
    acked: Option<Snapshot>,
    /// What the Mind said it saved, and when, while a restart is coming.
    guard: Option<(Snapshot, Instant)>,
}

impl Mirror {
    /// The person changed something: send `now` this, [`DEBOUNCE`] after the last change.
    pub fn changed(&mut self, snapshot: Snapshot, now: Instant) {
        self.pending = Some((snapshot, now + DEBOUNCE));
    }

    /// The snapshot to send at `now`, if one is due and the Mind does not already have it.
    pub fn due(&mut self, now: Instant) -> Option<Snapshot> {
        if self.pending.as_ref().is_none_or(|(_, at)| now < *at) {
            return None;
        }
        let (snapshot, _) = self.pending.take()?;
        (self.acked.as_ref() != Some(&snapshot)).then_some(snapshot)
    }

    /// The Mind attached at `now`: the snapshot to send, which is `current` unless this is the
    /// Mind's own re-attach after taking exactly it.
    pub fn attached(&mut self, current: Snapshot, now: Instant) -> Option<Snapshot> {
        self.acked = None;
        if let Some((saved, at)) = self.guard.take() {
            if saved == current && now.saturating_duration_since(at) <= LOOP_GUARD {
                self.acked = Some(current);
                return None;
            }
        }
        Some(current)
    }

    /// What came of sending `sent`: the Mind's reply, or `None` when it could not be reached or
    /// refused (the next attach sends it again).
    pub fn answered(&mut self, sent: &Snapshot, reply: Option<Reply>, now: Instant) {
        let Some(reply) = reply else { return };
        self.acked = Some(sent.clone());
        self.guard = (reply.restarting && reply.saved(sent)).then(|| (sent.clone(), now));
    }
}

static MIRROR: Mutex<Mirror> = Mutex::new(Mirror { pending: None, acked: None, guard: None });
/// One send at a time, so acks land in the order they were sent.
static SENDING: Mutex<()> = Mutex::new(());
/// An attach came before the accounts were first listed: sent once they are.
static ATTACH_OWED: AtomicBool = AtomicBool::new(false);
static LISTEN: Once = Once::new();

/// The snapshot from what the shell holds: never the file read again.
fn current() -> Snapshot {
    snapshot(&consent::held(&consent::path()), &super::accounts())
}

fn listen() {
    LISTEN.call_once(|| {
        super::on_change(|| {
            if ATTACH_OWED.swap(false, Ordering::SeqCst) {
                attach_now();
            }
        })
    });
}

/// The person changed Share / Private lane / Off (Settings → AI accounts, or onboarding's
/// sharing step). Sent [`DEBOUNCE`] after the last such change; never fails the press.
pub fn changed() {
    listen();
    MIRROR.lock().unwrap_or_else(|e| e.into_inner()).changed(current(), Instant::now());
    let _ = std::thread::Builder::new().name("consent-mirror".into()).spawn(|| {
        std::thread::sleep(DEBOUNCE + Duration::from_millis(50));
        let due = MIRROR.lock().unwrap_or_else(|e| e.into_inner()).due(Instant::now());
        if let Some(s) = due {
            send(&s);
        }
    });
}

/// The person's Yantrik Mind attached to the harness socket. Off the attach's own thread: the
/// Mind is waiting for that answer.
pub fn mind_attached() {
    listen();
    if !super::listed() {
        // An empty list would withdraw every provider: wait for the accounts.
        ATTACH_OWED.store(true, Ordering::SeqCst);
        return;
    }
    let _ = std::thread::Builder::new().name("consent-mirror".into()).spawn(attach_now);
}

fn attach_now() {
    let s = MIRROR.lock().unwrap_or_else(|e| e.into_inner()).attached(current(), Instant::now());
    if let Some(s) = s {
        send(&s);
    }
}

fn send(s: &Snapshot) {
    let _one = SENDING.lock().unwrap_or_else(|e| e.into_inner());
    let reply = post(&s.body());
    MIRROR.lock().unwrap_or_else(|e| e.into_inner()).answered(s, reply, Instant::now());
}

fn post(body: &str) -> Option<Reply> {
    // Tests never talk to a Mind.
    if cfg!(test) {
        return None;
    }
    match crate::provider_handoff::mind::post_to("/consent", body) {
        Ok(text) => {
            let reply = Reply::parse(&text).unwrap_or_default();
            tracing::info!(shared = ?reply.shared, restarting = reply.restarting, "the Mind took the person's sharing");
            Some(reply)
        }
        Err(why) => {
            tracing::info!(?why, "the Mind was not sent the person's sharing; it is sent when the Mind next attaches");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_accounts::account::{KeyRef, Source};

    fn account(id: &str, provider: &str, local: bool, trains: bool) -> Account {
        Account {
            id: id.into(),
            name: id.into(),
            provider_type: provider.into(),
            base_url: String::new(),
            key: KeyRef::None,
            local,
            source: Source::Saved,
            trains_on_prompts: trains,
            saved_model: String::new(),
        }
    }

    fn choose(pairs: &[(&str, Sharing)]) -> Consent {
        Consent { sharing: pairs.iter().map(|(id, s)| (id.to_string(), *s)).collect(), ..Consent::default() }
    }

    fn snap(pairs: &[(&str, bool)]) -> Snapshot {
        Snapshot { source: SOURCE, accounts: pairs.iter().map(|(p, s)| Entry { provider: p.to_string(), share: *s }).collect() }
    }

    #[test]
    fn the_body_is_exactly_the_pinned_json() {
        let accounts = [
            account("ollama-cloud", "ollama-cloud", false, false),
            account("home", "ollama", true, false),
            account("nanogpt", "NanoGPT", false, false),
        ];
        let consent = choose(&[("ollama-cloud", Sharing::Share), ("nanogpt", Sharing::Lane)]);
        assert_eq!(
            snapshot(&consent, &accounts).body(),
            r#"{"source":"yantrik-os","accounts":[{"provider":"ollama-cloud","share":true},{"provider":"nanogpt","share":false}]}"#
        );
    }

    #[test]
    fn ids_are_case_folded_nim_is_mapped_and_local_and_custom_are_not_sent() {
        let accounts = [
            account("nvidia-nim", "nvidia-nim", false, false),
            account("OpenAI", "OpenAI", false, false),
            account("lan", "ollama", true, false),
            account("mine", "custom", false, false),
        ];
        let s = snapshot(&Consent::default(), &accounts);
        assert_eq!(s, snap(&[("nim", false), ("openai", false)]));
    }

    #[test]
    fn share_is_true_only_where_the_account_resolves_to_share() {
        let accounts = [
            account("a", "openai", false, false),
            account("b", "anthropic", false, false),
            account("c", "gemini", false, false),
            account("free-kilo", "kilo", false, true),
        ];
        let consent = choose(&[
            ("a", Sharing::Share),
            ("b", Sharing::Lane),
            ("c", Sharing::Off),
            ("free-kilo", Sharing::Share),
        ]);
        assert_eq!(
            snapshot(&consent, &accounts),
            snap(&[("openai", true), ("anthropic", false), ("gemini", false), ("kilo", false)]),
            "Share → true; Private lane → false; Off → false; a training free tier set to Share → false"
        );
    }

    #[test]
    fn two_accounts_of_one_provider_share_only_if_both_do() {
        let accounts = [account("groq", "groq", false, false), account("free-groq", "groq", false, false)];
        let one_off = choose(&[("groq", Sharing::Share), ("free-groq", Sharing::Off)]);
        assert_eq!(snapshot(&one_off, &accounts), snap(&[("groq", false)]));
        let both = choose(&[("groq", Sharing::Share), ("free-groq", Sharing::Share)]);
        assert_eq!(snapshot(&both, &accounts), snap(&[("groq", true)]));
    }

    #[test]
    fn three_changes_in_300ms_are_one_send() {
        let t = Instant::now();
        let mut m = Mirror::default();
        let mut sends = Vec::new();
        for (i, share) in [true, false, true].into_iter().enumerate() {
            m.changed(snap(&[("groq", share)]), t + Duration::from_millis(150 * i as u64));
            // Every timer from an earlier change fires before the last change is due.
            sends.extend(m.due(t + Duration::from_millis(150 * i as u64 + 100)));
        }
        sends.extend(m.due(t + Duration::from_millis(300) + DEBOUNCE));
        sends.extend(m.due(t + Duration::from_millis(300) + DEBOUNCE + Duration::from_millis(50)));
        assert_eq!(sends, [snap(&[("groq", true)])]);
    }

    #[test]
    fn an_unchanged_snapshot_within_one_attach_is_not_sent() {
        let t = Instant::now();
        let mut m = Mirror::default();
        let s = snap(&[("groq", true)]);
        m.changed(s.clone(), t);
        let sent = m.due(t + DEBOUNCE).unwrap();
        m.answered(&sent, Some(Reply { saved: true, shared: vec!["groq".into()], restarting: false }), t + DEBOUNCE);
        m.changed(s.clone(), t + Duration::from_secs(5));
        assert_eq!(m.due(t + Duration::from_secs(7)), None);
        m.changed(snap(&[("groq", false)]), t + Duration::from_secs(8));
        assert_eq!(m.due(t + Duration::from_secs(10)), Some(snap(&[("groq", false)])));
    }

    #[test]
    fn a_fresh_attach_sends_even_an_unchanged_snapshot() {
        let t = Instant::now();
        let mut m = Mirror::default();
        let s = snap(&[("ollama-cloud", true)]);
        assert_eq!(m.attached(s.clone(), t), Some(s.clone()));
        m.answered(&s, Some(Reply { saved: true, shared: vec!["ollama-cloud".into()], restarting: false }), t);
        // Acknowledged, and the Mind did not restart: a later attach (a reinstall) sends it again.
        assert_eq!(m.attached(s.clone(), t + Duration::from_secs(2)), Some(s.clone()));
        m.answered(&s, Some(Reply { saved: true, shared: vec!["ollama-cloud".into()], restarting: true }), t + Duration::from_secs(2));
        assert_eq!(m.attached(s.clone(), t + Duration::from_secs(60)), Some(s), "past the loop guard");
    }

    #[test]
    fn the_reattach_after_an_ack_carrying_what_was_saved_is_not_answered_again() {
        let t = Instant::now();
        let mut m = Mirror::default();
        let s = snap(&[("ollama-cloud", true), ("nanogpt", false)]);
        m.changed(s.clone(), t);
        let sent = m.due(t + DEBOUNCE).unwrap();
        m.answered(&sent, Some(Reply { saved: true, shared: vec!["ollama-cloud".into()], restarting: true }), t + DEBOUNCE);
        assert_eq!(m.attached(s.clone(), t + DEBOUNCE + Duration::from_millis(500)), None, "the Mind's own restart");
        // Only that one: the next attach (a reinstall) sends again.
        assert_eq!(m.attached(s.clone(), t + Duration::from_secs(5)), Some(s.clone()));
        // Within the attach the guard skipped, the same snapshot counts as acknowledged.
        let mut m = Mirror::default();
        m.answered(&s, Some(Reply { saved: true, shared: vec!["ollama-cloud".into()], restarting: true }), t);
        assert_eq!(m.attached(s.clone(), t + Duration::from_secs(1)), None);
        m.changed(s.clone(), t + Duration::from_secs(2));
        assert_eq!(m.due(t + Duration::from_secs(4)), None);
    }

    #[test]
    fn the_guard_holds_only_for_what_the_mind_said_it_saved() {
        let t = Instant::now();
        let s = snap(&[("ollama-cloud", true)]);
        // Changed since: sent.
        let mut m = Mirror::default();
        m.answered(&s, Some(Reply { saved: true, shared: vec!["ollama-cloud".into()], restarting: true }), t);
        assert!(m.attached(snap(&[("ollama-cloud", false)]), t + Duration::from_secs(1)).is_some());
        // Not saved: sent.
        let mut m = Mirror::default();
        m.answered(&s, Some(Reply { saved: false, shared: vec![], restarting: true }), t);
        assert!(m.attached(s.clone(), t + Duration::from_secs(1)).is_some());
        // Shares what was not shared: sent.
        let mut m = Mirror::default();
        m.answered(&snap(&[("ollama-cloud", false)]), Some(Reply { saved: true, shared: vec!["ollama-cloud".into()], restarting: true }), t);
        assert!(m.attached(snap(&[("ollama-cloud", false)]), t + Duration::from_secs(1)).is_some());
        // Later than 10 s: sent.
        let mut m = Mirror::default();
        m.answered(&s, Some(Reply { saved: true, shared: vec!["ollama-cloud".into()], restarting: true }), t);
        assert!(m.attached(s, t + LOOP_GUARD + Duration::from_millis(1)).is_some());
    }

    #[test]
    fn no_socket_fails_nothing_and_the_next_attach_sends_it() {
        let t = Instant::now();
        let mut m = Mirror::default();
        let s = snap(&[("groq", false)]);
        // No Mind on a test machine: the shell's own post says so and nothing panics.
        assert!(matches!(
            crate::provider_handoff::mind::post_to("/consent", &s.body()),
            Err(crate::provider_handoff::mind::Refused::Unreachable(_))
        ));
        m.changed(s.clone(), t);
        let sent = m.due(t + DEBOUNCE).unwrap();
        m.answered(&sent, None, t + DEBOUNCE);
        assert_eq!(m.attached(s.clone(), t + Duration::from_secs(30)), Some(s));
    }

    #[test]
    fn the_reply_is_read_plain_or_chunked() {
        let want = Reply { saved: true, shared: vec!["ollama-cloud".into()], restarting: true };
        assert_eq!(Reply::parse(r#"{"saved":true,"shared":["ollama-cloud"],"restarting":true}"#), Some(want.clone()));
        assert_eq!(Reply::parse("3a\r\n{\"saved\":true,\"shared\":[\"ollama-cloud\"],\"restarting\":true}\r\n0\r\n\r\n"), Some(want));
        assert_eq!(Reply::parse("not json"), None);
    }

    /// Finding 4: a hand edit of `ai-accounts.json` toward Share is no press. It sends nothing,
    /// the gateway does not widen, and what the Mind would be sent next stays the person's.
    #[test]
    fn editing_the_file_to_share_sends_nothing_and_does_not_widen_the_gateway() {
        let dir = std::env::temp_dir().join(format!("consent-mirror-f4-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ai-accounts.json");
        let _ = std::fs::remove_file(&path);
        let accounts = [account("ollama-cloud", "ollama-cloud", false, false)];
        let t = Instant::now();
        let mut m = Mirror::default();

        // The person's press: Off.
        consent::set(&path, "ollama-cloud", Sharing::Off).unwrap();
        m.changed(snapshot(&consent::held(&path), &accounts), t);
        let sent = m.due(t + DEBOUNCE).unwrap();
        assert_eq!(sent, snap(&[("ollama-cloud", false)]));
        m.answered(&sent, Some(Reply { saved: true, shared: vec![], restarting: false }), t + DEBOUNCE);

        // A harness writes Share, and a default of Share, into the file.
        std::fs::write(&path, r#"{"sharing":{"ollama-cloud":"share"},"default":"share"}"#).unwrap();
        let read = consent::load(&path);
        assert_eq!(crate::gateway::sharing(&accounts[0], &read), Sharing::Off, "the gateway does not widen");
        assert_eq!(read.sharing_for(&account("new", "openai", false, false)), Sharing::Off, "nor does the default");
        assert_eq!(m.due(t + Duration::from_secs(30)), None, "nothing is sent");
        assert_eq!(snapshot(&consent::held(&path), &accounts), snap(&[("ollama-cloud", false)]));

        // A narrowing from the file is honoured.
        consent::set(&path, "ollama-cloud", Sharing::Share).unwrap();
        std::fs::write(&path, r#"{"sharing":{"ollama-cloud":"lane"}}"#).unwrap();
        assert_eq!(crate::gateway::sharing(&accounts[0], &consent::load(&path)), Sharing::Lane);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
