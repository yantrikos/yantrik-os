//! The `grant_request` event: the Mind asks to search the web in its own words.
//!
//! The Mind's egress planner lets a search carry only the person's own words unless the person
//! granted more (design/mind-egress-2026-09-29.md, section 6). Before such a search the Mind sends
//! `{"kind": "grant_request", "request_id", "capability": "web_search_own_words", "query",
//! "run_id"?}` on the turn it is answering, and the host either
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

use std::path::PathBuf;
use std::sync::Arc;

use crate::grants::{self, Grant};

/// The answers on the card, in order, and what each tells the Mind.
pub const OPTIONS: [(&str, &str); 4] = [("Once", "once"), ("This session", "session"), ("Always", "always"), ("No", "no")];

/// The longest query a card shows.
pub const MOST_QUERY_CHARS: usize = 300;

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

/// Whether `harness` may ask this at all; why not, for the `refused` reply.
pub fn screen(harness: &str, capability: &str, query: &str) -> Result<(), String> {
    if capability != grants::CAPABILITY {
        return Err(format!("no such capability: `{capability}` (there is only {})", grants::CAPABILITY));
    }
    if harness != grants::AGENT {
        return Err(format!("search grants are the Yantrik Mind's (`{}`), not `{harness}`'s", grants::AGENT));
    }
    if query.trim().is_empty() || query.trim() != query {
        return Err("the query is the exact search, with no space around it".into());
    }
    if query.chars().count() > MOST_QUERY_CHARS {
        return Err(format!("a query is at most {MOST_QUERY_CHARS} characters"));
    }
    // Shown exactly, so nothing that changes how the rest reads: no control or bidi characters.
    let hidden = |c: char| c.is_control() || matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
    if query.chars().any(hidden) {
        return Err("the query holds a control or direction character, and is shown exactly or not at all".into());
    }
    Ok(())
}

/// The card's words: the host's, around the exact query.
pub fn prompt(query: &str) -> String {
    format!(
        "The Mind wants to search the web in its own words:\n\n\u{201c}{query}\u{201d}\n\n\
         Once: this search only. This session: until the Mind restarts, 24 hours at most. \
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

    /// A host keeping runs, reading grants from `dir`, and the notices it gave.
    fn host_reading(dir: &std::path::Path) -> (Host, Arc<Mutex<Vec<GrantNotice>>>) {
        let told = Arc::new(Mutex::new(Vec::new()));
        let sink = told.clone();
        // SAFETY: neither can fail.
        let me = unsafe { (libc::geteuid(), libc::getegid()) };
        let host = Host::new(vec![])
            .with_runs(Arc::new(RunStore::in_memory().unwrap()))
            .with_grants(dir.join("grants.json"), me, move |n| sink.lock().unwrap().push(n));
        (host, told)
    }

    /// The Mind attached, with one turn in flight: (session, turn id, what the shell reads).
    fn minds_turn(host: &Host) -> (String, u64, crate::Answer) {
        let session = host.handle(protocol::ATTACH, &json!({"id": "mind", "name": "Yantrik Mind", "conversations": true})).unwrap()["session"]
            .as_str()
            .unwrap()
            .to_string();
        let agent = host.start_agent("mind").unwrap();
        let answer = host.send_to(&agent, Turn::new("research rust editions")).unwrap();
        let turn = host.handle(protocol::POLL, &json!({"session": session})).unwrap()["turn_id"].as_u64().unwrap();
        (session, turn, answer)
    }

    fn grant_request(host: &Host, session: &str, turn: u64, id: &str, query: &str) -> serde_json::Value {
        let event = json!({"kind": "grant_request", "request_id": id, "capability": "web_search_own_words", "query": query});
        host.handle(crate::event::EVENT, &json!({"session": session, "turn_id": turn, "event": event})).unwrap()
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("yantrik-host-grant-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn with_no_grant_the_person_gets_a_card_with_the_exact_query_and_four_answers() {
        let (host, told) = host_reading(&scratch("card"));
        let (session, turn, answer) = minds_turn(&host);
        assert_eq!(grant_request(&host, &session, turn, "g1", "rust 2027 edition changes"), json!({}));
        let shown: Vec<Chunk> = answer.try_iter().collect();
        let Some(Chunk::Event(Event::Request { request_id, prompt, options })) = shown.last() else {
            panic!("a question reaches the shell: {shown:?}");
        };
        assert_eq!(request_id, "g1");
        assert!(prompt.contains("\u{201c}rust 2027 edition changes\u{201d}"), "{prompt}");
        assert_eq!(options, &["Once", "This session", "Always", "No"]);

        host.answer(turn, "g1", &json!("This session"), true).unwrap();
        let polled = host.handle(protocol::POLL, &json!({"session": session})).unwrap();
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
        grant_request(&host, &session, turn, "a", "q one");
        grant_request(&host, &session, turn, "b", "q two");
        host.answer(turn, "a", &json!("Once"), true).unwrap();
        host.answer(turn, "b", &json!("Always"), false).unwrap();
        let polled = host.handle(protocol::POLL, &json!({"session": session})).unwrap();
        let answers: Vec<&str> = polled["answers"].as_array().unwrap().iter().map(|a| a["answer"].as_str().unwrap()).collect();
        assert_eq!(answers, ["once", "no"]);
        let keys: Vec<&str> = told.lock().unwrap().iter().map(|n| match n { GrantNotice::Answered { answer, .. } => *answer, _ => "used" }).collect();
        assert_eq!(keys, ["once", "no"]);
    }

    #[test]
    fn a_grant_in_force_answers_at_once_and_is_told_as_a_use() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("used");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let grant = json!({"id": "g-0123456789ab", "agent": "mind", "capability": "web_search_own_words", "scope": "always",
                           "scope_id": null, "granted_at": 1000, "expires_at": null, "granted_by": "person"});
        std::fs::write(dir.join("grants.json"), json!({"version": 1, "written_at": 1000, "grants": [grant]}).to_string()).unwrap();
        std::fs::set_permissions(dir.join("grants.json"), std::fs::Permissions::from_mode(0o644)).unwrap();
        let (host, told) = host_reading(&dir);
        let (session, turn, answer) = minds_turn(&host);
        let reply = grant_request(&host, &session, turn, "g1", "rust 2027 edition changes");
        assert_eq!(reply["granted"]["scope"], "always", "{reply}");
        assert!(answer.try_iter().all(|c| !matches!(c, Chunk::Event(Event::Request { .. }))), "nobody is asked");
        assert!(matches!(&told.lock().unwrap()[..], [GrantNotice::Used { query, .. }] if query == "rust 2027 edition changes"));
    }

    #[test]
    fn another_capability_or_another_harness_is_refused_before_anyone_is_asked() {
        let (host, told) = host_reading(&scratch("refused"));
        let (session, turn, _answer) = minds_turn(&host);
        let event = json!({"kind": "grant_request", "request_id": "x", "capability": "shell", "query": "q"});
        let reply = host.handle(crate::event::EVENT, &json!({"session": session, "turn_id": turn, "event": event})).unwrap();
        assert!(reply["refused"].as_str().unwrap().contains("no such capability"), "{reply}");
        assert!(told.lock().unwrap().is_empty());
    }

    #[test]
    fn only_the_minds_one_capability_with_a_plain_query_is_asked() {
        assert!(screen("mind", "web_search_own_words", "rust 1.97 release notes").is_ok());
        assert!(screen("mind", "web_search_anything", "x").unwrap_err().contains("no such capability"));
        assert!(screen("pi", "web_search_own_words", "x").is_err());
        assert!(screen("mind", "web_search_own_words", "").is_err());
        assert!(screen("mind", "web_search_own_words", " padded ").is_err());
        assert!(screen("mind", "web_search_own_words", "two\nlines").is_err());
        assert!(screen("mind", "web_search_own_words", "abc\u{202e}fed").is_err());
        assert!(screen("mind", "web_search_own_words", &"x".repeat(MOST_QUERY_CHARS + 1)).is_err());
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
