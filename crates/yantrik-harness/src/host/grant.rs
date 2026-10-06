//! The `grant_request` event: the Mind asks to search the web in its own words.
//!
//! The Mind's egress planner lets a search carry only the person's own words unless the person
//! granted more (design/mind-egress-2026-09-29.md, section 6). Before such a search the Mind sends
//! `{"kind": "grant_request", "request_id", "capability": "web_search_own_words", "query"}` on the
//! turn it is answering, and the host either
//!
//! - finds a grant in force that covers it (root's file, [`crate::grants`]) and answers at once
//!   `{"granted": {"id", "scope", "expires_at"}}`, telling the shell, which journals the use; or
//! - asks the person: the shell's ordinary question card, with the host's own words and the exact
//!   query, and four fixed answers ([`OPTIONS`]). The answer reaches the Mind as any answer does,
//!   on a later poll: `answers: [{turn_id, request_id, answer: "once"|"session"|"always"|"no",
//!   scope_id?}]`, `scope_id` with `session` (the grant's, [`crate::grants::session_scope_id`]).
//!
//! Nothing the Mind sends sets a grant: its words are only ever the query shown on the card. Only
//! the person's press of *This session* or *Always* makes the shell ask root to write one (the
//! [`GrantNotice`] hook); a typed answer, or any other, is `no`. *Once* stores nothing.
//!
//! Nothing the Mind sends picks the grant either. A session grant is the host's own session id
//! for the attach; a run grant covers a request only on a turn the desktop stamped with that run
//! (`turn["run"]`, from the person's `send_message … run=ID`). A request that still names a
//! `run_id` is refused. And only a `mind` the kernel said is the `yantrik-mind` account is
//! listened to ([`is_the_mind`]).
//!
//! What this does not do: bind the approved query to the search the Mind then makes. The answer is
//! a word to the Mind, and the Mind's own planner holds itself to searching what was approved; the
//! egress proxy sees no grants. Every answer and use is journalled with the query and its SHA-256
//! so an audit can compare (design/mind-egress-2026-09-29.md, section 6).

use std::path::PathBuf;
use std::sync::Arc;

use crate::grants::{self, Grant};

/// The answers on the card, in order, and what each tells the Mind.
pub const OPTIONS: [(&str, &str); 4] = [("Once", "once"), ("This session", "session"), ("Always", "always"), ("No", "no")];

pub use super::screen::MOST_QUERY_CHARS;

/// What the shell is told, to journal and to have root write a grant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantNotice {
    /// The person answered a card: `answer` is one of [`OPTIONS`]' keys.
    Answered { harness: String, query: String, scope_id: String, answer: &'static str },
    /// A grant in force covered a search, so nobody was asked.
    Used { harness: String, query: String, grant: Grant },
}

/// Where the grants are read from and who is told: see `Host::with_grants`.
#[derive(Clone)]
pub(super) struct Grants {
    pub path: PathBuf,
    pub owner: (u32, u32),
    /// The `yantrik-mind` account's uid; `None` where there is none, and then nobody is the Mind.
    pub mind_uid: Option<u32>,
    pub hook: Arc<dyn Fn(GrantNotice) + Send + Sync>,
}

impl Grants {
    /// The grants in force, or none when the file is not believed.
    pub fn in_force(&self) -> Vec<Grant> {
        grants::read(&self.path, self.owner, grants::now()).unwrap_or_default()
    }
}

/// A question asked of the person, remembered on its turn until it is answered.
#[derive(Clone, Debug)]
pub(super) struct Asked {
    pub query: String,
    pub scope_id: String,
}

/// Whether `harness` may ask this at all; why not, for the `refused` reply (the Mind logs it).
/// The query itself is screened by [`super::screen::query`]: shown exactly or not at all.
pub fn screen(harness: &str, capability: &str, query: &str) -> Result<(), String> {
    if capability != grants::CAPABILITY {
        return Err(format!("no such capability: `{capability}` (there is only {})", grants::CAPABILITY));
    }
    if harness != grants::AGENT {
        return Err(format!("search grants are the Yantrik Mind's (`{}`), not `{harness}`'s", grants::AGENT));
    }
    super::screen::query(query)
}

/// At most this many grant cards on one turn.
pub const MOST_CARDS_PER_TURN: usize = 2;

/// At most this many grant cards from one harness within [`CARD_WINDOW`].
pub const MOST_CARDS_PER_WINDOW: usize = 3;

/// The window [`MOST_CARDS_PER_WINDOW`] counts over.
pub const CARD_WINDOW: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Whether one more grant card may be raised, by the OS's own limits on asking, whatever the
/// Mind holds itself to: one card open at a time for the harness (`open`: one of its turns has a
/// card the person has not answered, which includes a card the person closed), at most
/// [`MOST_CARDS_PER_TURN`] on a turn (`on_turn` raised so far), none for the rest of a turn after
/// a No or a typed answer (`declined`), and at most [`MOST_CARDS_PER_WINDOW`] per harness per
/// [`CARD_WINDOW`] (`raised`: when its cards were raised, oldest first; ones older than the window
/// are dropped here). The window is kept in memory, so a shell restart starts it empty. Why not,
/// for the `refused` reply.
pub fn may_raise(
    open: bool,
    on_turn: usize,
    declined: bool,
    raised: &mut std::collections::VecDeque<std::time::Instant>,
    now: std::time::Instant,
) -> Result<(), String> {
    while raised.front().is_some_and(|at| now.saturating_duration_since(*at) >= CARD_WINDOW) {
        raised.pop_front();
    }
    if open {
        return Err("a search grant card is already waiting on the person; ask again once it is answered".into());
    }
    if declined {
        return Err("the person said no to a search on this turn; ask no more on it".into());
    }
    if on_turn >= MOST_CARDS_PER_TURN {
        return Err(format!("this turn has asked {MOST_CARDS_PER_TURN} search grants, the most one turn may"));
    }
    if raised.len() >= MOST_CARDS_PER_WINDOW {
        return Err(format!(
            "{MOST_CARDS_PER_WINDOW} search grant cards in the last {} minutes is the most; ask later",
            CARD_WINDOW.as_secs() / 60
        ));
    }
    Ok(())
}

/// Whether the attach that sent a `grant_request` is the Mind by the kernel's word: the uid
/// `SO_PEERCRED` gave at attach is the `yantrik-mind` account's. Why not, for the refusal.
pub(super) fn is_the_mind(grants: Option<&Grants>, attached_uid: Option<u32>) -> Result<(), String> {
    let Some(grants) = grants else {
        return Err("this desktop reads no search grants, so it asks nobody about one".into());
    };
    match (grants.mind_uid, attached_uid) {
        (Some(mind), Some(uid)) if mind == uid => Ok(()),
        (None, _) => Err("there is no yantrik-mind account on this machine, so nothing attached is the Mind".into()),
        (Some(_), Some(uid)) => Err(format!("`mind` attached as uid {uid}, not the yantrik-mind account; only the Mind asks to search")),
        (Some(_), None) => Err("the kernel did not say which account attached as `mind`, so it is not taken for the Mind".into()),
    }
}

/// Whether a harness's own `request` looks like the grant card: refused, so a harness cannot draw a
/// copy of the card whose *Always* grants nothing and that it could still read as consent. Any
/// option that is `always` or `this session` (case-folded, trimmed) is the card's, in any order and
/// among any others, and so is a prompt that opens with the card's own first line.
pub fn copies_the_card(prompt: &str, options: &[String]) -> bool {
    let fold = |o: &str| o.trim().to_lowercase();
    let card_only = ["always", "this session"];
    let first_line = self::prompt("").lines().next().unwrap_or_default().to_string();
    options.iter().any(|o| card_only.contains(&fold(o).as_str())) || prompt.trim_start().starts_with(&first_line)
}

/// The card's words: the host's, around the exact query.
pub fn prompt(query: &str) -> String {
    format!(
        "The Mind wants to search the web in its own words:\n\n\u{201c}{query}\u{201d}\n\n\
         It may go to the search service and its fallback engine. Once: this search only. This session: until the Mind restarts, 24 hours at most. \
         Always: until you revoke it in Settings \u{2192} Harnesses."
    )
}

/// The options the card offers, as labels.
pub fn labels() -> Vec<String> {
    OPTIONS.iter().map(|(label, _)| label.to_string()).collect()
}

/// What an answer tells the Mind. Only a pressed option counts; anything else is `no`.
pub fn answer_key(answer: &serde_json::Value, by_option: bool) -> &'static str {
    let said = answer.as_str().unwrap_or_default();
    OPTIONS.iter().find(|(label, _)| by_option && *label == said).map(|(_, key)| *key).unwrap_or("no")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    use crate::event::Event;
    use crate::run_store::RunStore;
    use crate::{protocol, Chunk, Host, Turn};

    /// The `yantrik-mind` account's uid, as these tests have it.
    const MIND_UID: u32 = 990;

    /// A host keeping runs, reading grants from `dir`, and the notices it gave.
    fn host_reading(dir: &std::path::Path) -> (Host, Arc<Mutex<Vec<GrantNotice>>>) {
        let told = Arc::new(Mutex::new(Vec::new()));
        let sink = told.clone();
        // SAFETY: neither can fail.
        let me = unsafe { (libc::geteuid(), libc::getegid()) };
        let host = Host::new(vec![])
            .with_runs(Arc::new(RunStore::in_memory().unwrap()))
            .with_grants(dir.join("grants.json"), me, Some(MIND_UID), move |n| sink.lock().unwrap().push(n));
        (host, told)
    }

    /// One call on the wire from a process running as `uid`, as the kernel would say it.
    fn call(host: &Host, uid: Option<u32>, method: &str, params: serde_json::Value) -> serde_json::Value {
        host.handle_from(method, &params, None, uid).unwrap()
    }

    /// The Mind attached as `uid`, with one turn in flight: (session, turn id, what the shell
    /// reads, the turn as the harness was handed it).
    fn turn_as(host: &Host, uid: Option<u32>, turn: Turn) -> (String, u64, crate::Answer, serde_json::Value) {
        let attach = json!({"id": "mind", "name": "Yantrik Mind", "conversations": true});
        let session = call(host, uid, protocol::ATTACH, attach)["session"].as_str().unwrap().to_string();
        let agent = host.start_agent("mind").unwrap();
        let answer = host.send_to(&agent, turn).unwrap();
        let handed = call(host, uid, protocol::POLL, json!({"session": session}));
        (session, handed["turn_id"].as_u64().unwrap(), answer, handed)
    }

    /// The Mind, attached as the mind account, with one turn in flight.
    fn minds_turn(host: &Host) -> (String, u64, crate::Answer) {
        let (session, turn, answer, _) = turn_as(host, Some(MIND_UID), Turn::new("research rust editions"));
        (session, turn, answer)
    }

    fn send_event(host: &Host, session: &str, turn: u64, event: serde_json::Value) -> serde_json::Value {
        call(host, Some(MIND_UID), crate::event::EVENT, json!({"session": session, "turn_id": turn, "event": event}))
    }

    fn grant_request(host: &Host, session: &str, turn: u64, id: &str, query: &str) -> serde_json::Value {
        let event = json!({"kind": "grant_request", "request_id": id, "capability": "web_search_own_words", "query": query});
        send_event(host, session, turn, event)
    }

    fn poll(host: &Host, session: &str) -> serde_json::Value {
        call(host, Some(MIND_UID), protocol::POLL, json!({"session": session}))
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("yantrik-host-grant-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// `dir` holding root's grants file, as the reader believes it in a test: these grants.
    fn with_file(dir: &std::path::Path, grants: Vec<serde_json::Value>) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.join("grants.json"), json!({"version": 1, "written_at": 1000, "grants": grants}).to_string()).unwrap();
        std::fs::set_permissions(dir.join("grants.json"), std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    /// A run grant for `run`, in force for the next hour.
    fn run_grant(run: &str) -> serde_json::Value {
        let now = crate::grants::now();
        json!({"id": "g-00000000a00a", "agent": "mind", "capability": "web_search_own_words", "scope": "run",
               "scope_id": run, "granted_at": now - 10, "expires_at": now + 3600, "granted_by": "run-starter"})
    }

    fn cards(answer: &crate::Answer) -> usize {
        answer.try_iter().filter(|c| matches!(c, Chunk::Event(Event::Request { .. }))).count()
    }

    #[test]
    fn with_no_grant_the_person_gets_a_card_with_the_exact_query_and_four_answers() {
        let (host, told) = host_reading(&scratch("card"));
        let (session, turn, answer) = minds_turn(&host);
        assert_eq!(grant_request(&host, &session, turn, "g1", "rust 2027 edition changes"), json!({}));
        let shown: Vec<Chunk> = answer.try_iter().collect();
        let Some(Chunk::Event(Event::Request { request_id, prompt, options, by_host })) = shown.last() else {
            panic!("a question reaches the shell: {shown:?}");
        };
        assert_eq!(request_id, "g1");
        assert!(prompt.contains("\u{201c}rust 2027 edition changes\u{201d}"), "{prompt}");
        assert_eq!(options, &["Once", "This session", "Always", "No"]);
        assert!(by_host, "the card is marked as the desktop's");

        host.answer(turn, "g1", &json!("This session"), true).unwrap();
        let polled = poll(&host, &session);
        let scope_id = crate::grants::session_scope_id(&session);
        assert_eq!(polled["answers"], json!([{"turn_id": turn, "request_id": "g1", "answer": "session", "scope_id": scope_id}]));
        assert_eq!(
            told.lock().unwrap().as_slice(),
            [GrantNotice::Answered { harness: "mind".into(), query: "rust 2027 edition changes".into(), scope_id, answer: "session" }]
        );
    }

    #[test]
    fn once_stores_nothing_and_a_typed_answer_is_no() {
        let (host, told) = host_reading(&scratch("once"));
        let (session, turn, _answer) = minds_turn(&host);
        assert_eq!(grant_request(&host, &session, turn, "a", "q one"), json!({}));
        host.answer(turn, "a", &json!("Once"), true).unwrap();
        assert_eq!(grant_request(&host, &session, turn, "b", "q two"), json!({}));
        host.answer(turn, "b", &json!("Always"), false).unwrap();
        let polled = poll(&host, &session);
        let answers: Vec<&str> = polled["answers"].as_array().unwrap().iter().map(|a| a["answer"].as_str().unwrap()).collect();
        assert_eq!(answers, ["once", "no"]);
        let keys: Vec<&str> = told.lock().unwrap().iter().map(|n| match n { GrantNotice::Answered { answer, .. } => *answer, _ => "used" }).collect();
        assert_eq!(keys, ["once", "no"]);
    }

    #[test]
    fn a_grant_in_force_answers_at_once_and_is_told_as_a_use() {
        let dir = scratch("used");
        let grant = json!({"id": "g-0123456789ab", "agent": "mind", "capability": "web_search_own_words", "scope": "always",
                           "scope_id": null, "granted_at": 1000, "expires_at": null, "granted_by": "person"});
        with_file(&dir, vec![grant]);
        let (host, told) = host_reading(&dir);
        let (session, turn, answer) = minds_turn(&host);
        let reply = grant_request(&host, &session, turn, "g1", "rust 2027 edition changes");
        assert_eq!(reply["granted"]["scope"], "always", "{reply}");
        assert_eq!(cards(&answer), 0, "nobody is asked");
        assert!(matches!(&told.lock().unwrap()[..], [GrantNotice::Used { query, .. }] if query == "rust 2027 edition changes"));
    }

    #[test]
    fn another_capability_or_another_harness_is_refused_before_anyone_is_asked() {
        let (host, told) = host_reading(&scratch("refused"));
        let (session, turn, _answer) = minds_turn(&host);
        let event = json!({"kind": "grant_request", "request_id": "x", "capability": "shell", "query": "q"});
        let reply = send_event(&host, &session, turn, event);
        assert!(reply["refused"].as_str().unwrap().contains("no such capability"), "{reply}");
        assert!(told.lock().unwrap().is_empty());
    }

    // ── M2: a run grant is the turn's run, never one the Mind names ──

    #[test]
    fn the_run_rides_on_the_turn_the_harness_is_handed_and_only_there() {
        let (host, _) = host_reading(&scratch("run-wire"));
        let (_, _, _, handed) = turn_as(&host, Some(MIND_UID), Turn::new("start the research").with_run("research-42"));
        assert_eq!(handed["run"], "research-42", "{handed}");
        let (host, _) = host_reading(&scratch("run-wire-none"));
        let (_, _, _, handed) = turn_as(&host, Some(MIND_UID), Turn::new("just a question"));
        assert!(handed.get("run").is_none(), "a turn in no run says none: {handed}");
        let agent = host.start_agent("mind").unwrap();
        assert!(host.send_to(&agent, Turn::new("x").with_run("not a run id")).is_err(), "a run id is checked");
    }

    #[test]
    fn a_run_grant_covers_a_turn_stamped_with_its_run() {
        let dir = scratch("run-yes");
        with_file(&dir, vec![run_grant("research-42")]);
        let (host, told) = host_reading(&dir);
        let (session, turn, answer, _) = turn_as(&host, Some(MIND_UID), Turn::new("research").with_run("research-42"));
        let reply = grant_request(&host, &session, turn, "g1", "rust 2027 edition");
        assert_eq!(reply["granted"]["scope"], "run", "{reply}");
        assert_eq!(cards(&answer), 0);
        assert!(matches!(&told.lock().unwrap()[..], [GrantNotice::Used { grant, .. }] if grant.scope_id.as_deref() == Some("research-42")));
    }

    #[test]
    fn a_run_grant_is_not_used_by_a_turn_without_the_run() {
        let dir = scratch("run-no");
        with_file(&dir, vec![run_grant("research-42")]);
        let (host, told) = host_reading(&dir);
        let (session, turn, answer) = minds_turn(&host);
        assert_eq!(grant_request(&host, &session, turn, "g1", "rust 2027 edition"), json!({}), "the person is asked");
        assert_eq!(cards(&answer), 1);
        assert!(told.lock().unwrap().is_empty(), "no grant was used");
        // Nor by a turn in another run.
        let (host, _) = host_reading(&dir);
        let (session, turn, answer, _) = turn_as(&host, Some(MIND_UID), Turn::new("other").with_run("research-43"));
        assert_eq!(grant_request(&host, &session, turn, "g2", "rust 2027 edition"), json!({}));
        assert_eq!(cards(&answer), 1);
    }

    #[test]
    fn a_request_naming_a_run_is_refused_and_uses_no_grant() {
        let dir = scratch("run-named");
        with_file(&dir, vec![run_grant("research-42")]);
        let (host, told) = host_reading(&dir);
        // The review's scenario: the run id is world-readable, and an interactive turn names it.
        let (session, turn, answer) = minds_turn(&host);
        let event = json!({"kind": "grant_request", "request_id": "g1", "capability": "web_search_own_words",
                           "query": "rust 2027 edition", "run_id": "research-42"});
        let reply = send_event(&host, &session, turn, event);
        assert!(reply["refused"].as_str().unwrap().contains("no longer names a run"), "{reply}");
        assert_eq!(cards(&answer), 0, "no card either");
        assert!(told.lock().unwrap().is_empty());
        // Even on the run's own turn, naming it is refused: the turn says the run, not the Mind.
        let (host, _) = host_reading(&dir);
        let (session, turn, _, _) = turn_as(&host, Some(MIND_UID), Turn::new("r").with_run("research-42"));
        let event = json!({"kind": "grant_request", "request_id": "g1", "capability": "web_search_own_words",
                           "query": "q", "run_id": "research-42"});
        assert!(send_event(&host, &session, turn, event)["refused"].is_string());
        let event = json!({"kind": "grant_request", "request_id": "g1", "capability": "web_search_own_words",
                           "query": "q", "run_id": null});
        assert!(send_event(&host, &session, turn, event)["refused"].is_string(), "a null run_id is still a run_id");
    }

    // ── L1: the Mind by the kernel's word ──

    #[test]
    fn a_mind_that_is_not_the_mind_account_gets_no_grant_and_raises_no_card() {
        let dir = scratch("uid");
        let always = json!({"id": "g-0123456789ab", "agent": "mind", "capability": "web_search_own_words", "scope": "always",
                            "scope_id": null, "granted_at": 1000, "expires_at": null, "granted_by": "person"});
        with_file(&dir, vec![always]);
        for uid in [Some(1000), None] {
            let (host, told) = host_reading(&dir);
            let (session, turn, answer, _) = turn_as(&host, uid, Turn::new("x"));
            let event = json!({"kind": "grant_request", "request_id": "g1", "capability": "web_search_own_words", "query": "q"});
            let reply = call(&host, uid, crate::event::EVENT, json!({"session": session, "turn_id": turn, "event": event}));
            assert!(reply["refused"].is_string(), "{uid:?}: {reply}");
            assert!(reply.get("granted").is_none());
            assert_eq!(cards(&answer), 0, "{uid:?}: no card");
            assert!(told.lock().unwrap().is_empty());
        }
        // And on a machine with no yantrik-mind account, nobody is the Mind.
        let me = unsafe { (libc::geteuid(), libc::getegid()) };
        let host = Host::new(vec![]).with_runs(Arc::new(RunStore::in_memory().unwrap())).with_grants(dir.join("grants.json"), me, None, |_| {});
        let (session, turn, _, _) = turn_as(&host, Some(MIND_UID), Turn::new("x"));
        let event = json!({"kind": "grant_request", "request_id": "g1", "capability": "web_search_own_words", "query": "q"});
        let reply = call(&host, Some(MIND_UID), crate::event::EVENT, json!({"session": session, "turn_id": turn, "event": event}));
        assert!(reply["refused"].as_str().unwrap().contains("no yantrik-mind account"), "{reply}");
    }

    // ── L4: the card is the host's ──

    #[test]
    fn a_harness_request_offering_the_cards_answers_is_refused_and_none_is_marked_the_hosts() {
        let (host, _) = host_reading(&scratch("copy"));
        let (session, turn, answer) = minds_turn(&host);
        let copy = json!({"kind": "request", "request_id": "r1", "prompt": prompt("rust"),
                          "options": ["Once", "This session", "Always", "No"]});
        assert!(send_event(&host, &session, turn, copy)["refused"].as_str().unwrap().contains("grant card"));
        let copy = json!({"kind": "request", "request_id": "r2", "prompt": "May I?", "options": [" once", "THIS SESSION", "always ", "no"]});
        assert!(send_event(&host, &session, turn, copy)["refused"].is_string(), "case and spaces do not make it another card");
        assert_eq!(cards(&answer), 0);
        // An ordinary question passes, and a harness cannot mark it the host's.
        let own = json!({"kind": "request", "request_id": "r3", "prompt": "Which folder?", "options": ["Photos", "Documents"], "by_host": true});
        assert_eq!(send_event(&host, &session, turn, own), json!({}));
        let shown: Vec<Chunk> = answer.try_iter().collect();
        assert!(matches!(shown.last(), Some(Chunk::Event(Event::Request { by_host: false, .. }))), "{shown:?}");
        assert!(copies_the_card("x", &labels()));
    }

    // ── L4-r2: anything like the card is the card ──

    #[test]
    fn a_request_offering_always_or_this_session_in_any_order_is_refused() {
        let (host, _) = host_reading(&scratch("copy-r2"));
        let (session, turn, answer) = minds_turn(&host);
        for (i, options) in [
            json!(["Always", "Once", "This session", "No"]),
            json!(["Once", "This session", "Always"]),
            json!(["Once", "This session", "Always", "No", "Cancel"]),
            json!(["Yes", " ALWAYS "]),
            json!(["this session", "no"]),
        ]
        .into_iter()
        .enumerate()
        {
            let copy = json!({"kind": "request", "request_id": format!("r{i}"), "prompt": "May I?", "options": options});
            assert!(send_event(&host, &session, turn, copy)["refused"].is_string(), "{options}");
        }
        assert_eq!(cards(&answer), 0);
        let s = |v: &[&str]| v.iter().map(|o| o.to_string()).collect::<Vec<_>>();
        assert!(copies_the_card("May I?", &s(&["Yes", "always"])));
        assert!(copies_the_card("May I?", &s(&["\tThis Session "])));
        assert!(!copies_the_card("May I?", &s(&["Once", "No", "Always ask me"])), "a longer answer is not the card's");
    }

    #[test]
    fn a_request_whose_prompt_opens_with_the_cards_words_is_refused() {
        let (host, _) = host_reading(&scratch("copy-prompt"));
        let (session, turn, answer) = minds_turn(&host);
        let first = prompt("").lines().next().unwrap().to_string();
        assert_eq!(first, "The Mind wants to search the web in its own words:");
        assert!(prompt("rust").contains("\n\nIt may go to the search service and its fallback engine. Once:"), "where a search goes");
        for (i, p) in [prompt("rust"), first.clone(), format!("  {first} please")].into_iter().enumerate() {
            let copy = json!({"kind": "request", "request_id": format!("p{i}"), "prompt": p, "options": ["Yes", "No"]});
            assert!(send_event(&host, &session, turn, copy)["refused"].is_string(), "{p}");
        }
        assert_eq!(cards(&answer), 0);
        assert!(!copies_the_card("Should the Mind search the web?", &["Yes".into(), "No".into()]));
    }

    // ── M4: the OS limits how often the person is asked ──

    /// A second turn for the Mind's session, after closing `turn`.
    fn next_turn(host: &Host, session: &str, turn: u64) -> (u64, crate::Answer) {
        call(host, Some(MIND_UID), protocol::COMPLETE, json!({"session": session, "turn_id": turn}));
        let agent = host.start_agent("mind").unwrap();
        let answer = host.send_to(&agent, Turn::new("more")).unwrap();
        let handed = poll(host, session);
        (handed["turn_id"].as_u64().expect("a turn"), answer)
    }

    #[test]
    fn one_open_grant_card_at_a_time() {
        let (host, told) = host_reading(&scratch("limit-open"));
        let (session, turn, answer) = minds_turn(&host);
        assert_eq!(grant_request(&host, &session, turn, "a", "q one"), json!({}));
        // Unanswered, or closed by the person (which tells the host nothing): still open.
        let reply = grant_request(&host, &session, turn, "b", "q two");
        assert!(reply["refused"].as_str().unwrap().contains("already waiting"), "{reply}");
        assert_eq!(cards(&answer), 1);
        assert!(told.lock().unwrap().is_empty());
        host.answer(turn, "a", &json!("Once"), true).unwrap();
        assert_eq!(grant_request(&host, &session, turn, "c", "q three"), json!({}), "answered, so another may be asked");
    }

    #[test]
    fn at_most_two_grant_cards_per_turn() {
        let (host, _) = host_reading(&scratch("limit-turn"));
        let (session, turn, answer) = minds_turn(&host);
        for id in ["a", "b"] {
            assert_eq!(grant_request(&host, &session, turn, id, "q"), json!({}));
            host.answer(turn, id, &json!("Once"), true).unwrap();
        }
        let reply = grant_request(&host, &session, turn, "c", "q");
        assert!(reply["refused"].as_str().unwrap().contains("the most one turn may"), "{reply}");
        assert_eq!(cards(&answer), 2);
    }

    #[test]
    fn no_grant_card_for_the_rest_of_a_turn_after_a_no_or_a_typed_answer() {
        for (said, pressed) in [("No", true), ("Always", false), ("sure, go ahead", false)] {
            let (host, _) = host_reading(&scratch("limit-no"));
            let (session, turn, answer) = minds_turn(&host);
            assert_eq!(grant_request(&host, &session, turn, "a", "q"), json!({}));
            host.answer(turn, "a", &json!(said), pressed).unwrap();
            let reply = grant_request(&host, &session, turn, "b", "q");
            assert!(reply["refused"].as_str().unwrap().contains("said no"), "{said}: {reply}");
            assert_eq!(cards(&answer), 1);
            // The next turn may ask again.
            let (next, answer) = next_turn(&host, &session, turn);
            assert_eq!(grant_request(&host, &session, next, "c", "q"), json!({}), "{said}");
            assert_eq!(cards(&answer), 1);
        }
    }

    #[test]
    fn at_most_three_grant_cards_per_harness_in_ten_minutes() {
        let (host, _) = host_reading(&scratch("limit-window"));
        let (session, turn, _first) = minds_turn(&host);
        for id in ["a", "b"] {
            assert_eq!(grant_request(&host, &session, turn, id, "q"), json!({}));
            host.answer(turn, id, &json!("Once"), true).unwrap();
        }
        let (next, answer) = next_turn(&host, &session, turn);
        assert_eq!(grant_request(&host, &session, next, "c", "q"), json!({}));
        host.answer(next, "c", &json!("Once"), true).unwrap();
        let reply = grant_request(&host, &session, next, "d", "q");
        assert!(reply["refused"].as_str().unwrap().contains("10 minutes"), "{reply}");
        assert_eq!(cards(&answer), 1);

        // The window itself: cards older than ten minutes no longer count.
        let now = std::time::Instant::now();
        let mut raised: std::collections::VecDeque<_> = [now; 3].into();
        assert!(may_raise(false, 0, false, &mut raised, now).is_err());
        assert_eq!(may_raise(false, 0, false, &mut raised, now + CARD_WINDOW), Ok(()));
        assert!(raised.is_empty());
    }

    // ── H1: the card shows the exact query or none (each case is in `host::screen`'s tests) ──

    #[test]
    fn only_the_minds_one_capability_with_a_screened_query_is_asked() {
        assert!(screen("mind", "web_search_own_words", "rust 1.97 release notes").is_ok());
        assert!(screen("mind", "web_search_anything", "x").unwrap_err().contains("no such capability"));
        assert!(screen("pi", "web_search_own_words", "x").is_err());
        assert!(screen("mind", "web_search_own_words", " padded ").is_err());
        assert!(screen("mind", "web_search_own_words", "cafe\u{301}").unwrap_err().contains("NFKC"), "the query screen runs");
        assert!(screen("mind", "web_search_own_words", "!wp foo").unwrap_err().contains("search-engine syntax"));
    }

    #[test]
    fn surrogates_cs_cannot_reach_the_screen() {
        // A Rust string cannot hold a lone surrogate, and the wire's JSON parser refuses one
        // (`\ud800` alone is not a string serde_json reads), so Cs never arrives as a char. The
        // screen refuses the category all the same; this pins that the wire does too.
        let raw = r#"{"kind": "grant_request", "request_id": "g", "capability": "web_search_own_words", "query": "a\ud800b"}"#;
        assert!(serde_json::from_str::<serde_json::Value>(raw).is_err());
    }

    #[test]
    fn the_card_has_the_four_answers_and_the_exact_query() {
        assert_eq!(labels(), ["Once", "This session", "Always", "No"]);
        assert!(prompt("weather in Pune").contains("\u{201c}weather in Pune\u{201d}"));
    }

    #[test]
    fn only_a_pressed_option_counts() {
        assert_eq!(answer_key(&json!("Once"), true), "once");
        assert_eq!(answer_key(&json!("This session"), true), "session");
        assert_eq!(answer_key(&json!("Always"), true), "always");
        assert_eq!(answer_key(&json!("No"), true), "no");
        assert_eq!(answer_key(&json!("Always"), false), "no", "typed, not pressed");
        assert_eq!(answer_key(&json!("always please"), true), "no");
    }
}
