//! `set_web_search`, as an agent asks for it: the card's sentence, the change itself, and what
//! `describe shell` says about the setting.
//!
//! The action is graded `dangerous` (control.rs): asked about every time, in auto too, and never
//! answered by an "Allow for this session" rule, because a rule is per action and not per address.
//! What a model reads here never carries the address (docs/harness.md); the card the person reads
//! does.
//!
//! An agent's Test probe reaches out to an address the agent chose, so there is at most one at a
//! time and at most one started every [`PROBE_EVERY`].

use std::sync::Mutex;
use std::time::{Duration, Instant};

use slint::ComponentHandle;
use yantrik_web_search::{client, Service, Target};

use super::{apply, check_egress, show_saved};
use crate::{App, WebSearchState};

/// The least time between two agent-initiated Test probes.
pub const PROBE_EVERY: Duration = Duration::from_secs(10);

/// Agents' Test probes: whether one is running, and when the last one started.
#[derive(Debug, Default)]
pub struct ProbeGate {
    running: bool,
    last: Option<Instant>,
}

impl ProbeGate {
    pub const fn new() -> Self {
        ProbeGate { running: false, last: None }
    }

    /// Start one at `now`, or say why not. The refusal is for the agent to relay.
    pub fn start(&mut self, now: Instant) -> Result<(), String> {
        if self.running {
            return Err("A test search an agent asked for is still running; nothing was changed. Ask again once `describe shell` shows how it went.".into());
        }
        if let Some(last) = self.last {
            let since = now.saturating_duration_since(last);
            if since < PROBE_EVERY {
                let wait = (PROBE_EVERY - since).as_secs().max(1);
                return Err(format!(
                    "Agents may start one test search every {} seconds; nothing was changed. Ask again in {wait} s.",
                    PROBE_EVERY.as_secs()
                ));
            }
        }
        self.running = true;
        self.last = Some(now);
        Ok(())
    }

    pub fn finish(&mut self) {
        self.running = false;
    }
}

static PROBES: Mutex<ProbeGate> = Mutex::new(ProbeGate::new());

/// Ends the running probe when dropped, so a probe that panics does not hold the gate shut.
struct Running;

impl Drop for Running {
    fn drop(&mut self) {
        PROBES.lock().unwrap_or_else(|e| e.into_inner()).finish();
    }
}

/// For `describe shell` → `settings` → `web_search`. Says which service and whether the saved
/// address is usable, not the address: that is configuration, not something to show the model.
pub fn describe() -> serde_json::Value {
    let ws = crate::wire::settings::web_search();
    serde_json::json!({
        "service": ws.service.as_str(),
        "valid": !matches!(ws.target(), Target::Invalid { .. }),
        "saved_at": ws.saved_at,
        "change_with": "set_web_search (dangerous: the person is asked every time, since every search would go to the new address)",
    })
}

/// The sentence on an agent's approval card for `set_web_search`: where every search would go.
/// For the person, so it names the address.
pub fn explain_set(service: &str, url: &str) -> String {
    let now = match crate::wire::settings::web_search().target() {
        Target::Searxng(u) => format!("the person's SearXNG at {}", u.url),
        _ => "DuckDuckGo".to_string(),
    };
    match service {
        "builtin" => format!(
            "Sends every web search the person's minds and tools make to DuckDuckGo's HTML search, and nowhere else, instead of {now}."
        ),
        "searxng" => match yantrik_web_search::check(url) {
            Ok(u) => format!(
                "Sends every web search the person's minds and tools make to {} instead of {now}, after a test search there finds results. Whoever runs that address sees every query and chooses what comes back.",
                u.url
            ),
            Err(why) => format!("This would not go ahead: {why}"),
        },
        other => format!("This would not go ahead: `{other}` is not a service; use builtin or searxng."),
    }
}

/// `set_web_search`, for an agent: the same checks as Save, the Test included. Answers at once;
/// the test and the save happen off the UI thread, and `describe` shows the outcome.
pub fn set_from_agent(service: &str, url: &str, ui: slint::Weak<App>) -> Result<String, String> {
    match service {
        "builtin" => {
            apply(Service::Builtin, None)?;
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui.upgrade() {
                    show_saved(&ui);
                }
            });
            Ok("Saved: built-in (DuckDuckGo).".into())
        }
        "searxng" => {
            let checked = yantrik_web_search::check(url)?;
            PROBES.lock().unwrap_or_else(|e| e.into_inner()).start(Instant::now())?;
            std::thread::spawn(move || {
                let running = Running;
                let probe = client::probe(&checked, client::TIMEOUT);
                drop(running);
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = ui.upgrade() else { return };
                    let g = ui.global::<WebSearchState>();
                    if !probe.ok {
                        tracing::info!(summary = %probe.summary, "an agent's web search address was not saved");
                        g.set_save_error(format!("Not saved (asked by an agent): {}", probe.summary).into());
                        return;
                    }
                    match apply(Service::Searxng, Some(checked.url.clone())) {
                        Ok(_) => {
                            show_saved(&ui);
                            check_egress(&ui, checked);
                        }
                        Err(e) => g.set_save_error(format!("Not saved: {e}").into()),
                    }
                });
            });
            Ok("Testing the address; it is saved only if the test search finds results.".into())
        }
        other => Err(format!("`{other}` is not a service; use builtin or searxng")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::{self, Store, Verified};
    use crate::mind_mode::{Bypass, Decision, Mode, Modes};

    /// `set_web_search`'s declaration in control.rs, from its quoted name to the next `.action(`.
    fn declaration() -> String {
        let src = include_str!("../../control.rs");
        let src = src.split("#[cfg(test)]").next().unwrap();
        let from = src.find("\"set_web_search\"").expect("the shell publishes set_web_search");
        src[from..from + src[from..].find(".action(").unwrap()].to_string()
    }

    fn grade() -> &'static str {
        let decl = declaration();
        for grade in ["safe", "standard", "sensitive", "dangerous"] {
            if decl.contains(&format!(".risk(\"{grade}\")")) {
                return grade;
            }
        }
        panic!("set_web_search declares no grade:\n{decl}");
    }

    fn purpose() -> String {
        let decl = declaration();
        let at = decl.find("\",\n").unwrap() + 3;
        let open = at + decl[at..].find('"').unwrap() + 1;
        decl[open..open + decl[open..].find("\",").unwrap()].to_string()
    }

    fn mind() -> Verified {
        Verified {
            line: "python -m hermes_cli.main gateway (pid 696) \u{b7} the attached mind".into(),
            exe: "/usr/bin/python3".into(),
            pid: 696,
            attached_mind: "Hermes Agent".into(),
            mind_by_pid: true,
            ..Verified::default()
        }
    }

    /// Security review of #656, finding 1: graded sensitive, auto ran it unasked, and "Allow for
    /// this session" for a LAN address stood for any later one. Dangerous is asked every time.
    #[test]
    fn a_second_address_is_asked_about_whatever_was_allowed_before() {
        let (grade, purpose) = (grade(), purpose());
        assert_eq!(grade, "dangerous", "{}", declaration());
        assert!(purpose.starts_with("Choose the web search service"), "{purpose}");
        assert!(declaration().contains(".explain("), "the card says where searches would go");
        assert!(!approvals::may_offer_session_rule(grade, &purpose), "the card offers no session rule");

        let now = Instant::now();
        for mode in [Mode::Ask, Mode::Auto] {
            let mut modes = Modes::new(Mode::Ask);
            modes.person_set_mode(mode, Bypass::Hour, now, 0);
            // The first call, for a LAN address, is asked about.
            assert_eq!(modes.decide(grade, "shell", "set_web_search", false, "dangerous", now), Decision::Ask, "{mode:?}");
            // Allowed once. A session rule for it cannot be made, not even by the UI's own path.
            let mut store = Store::new();
            let lan = serde_json::json!({ "service": "searxng", "url": "http://192.168.4.42:8888" });
            let other = serde_json::json!({ "service": "searxng", "url": "https://attacker.example" });
            let id = store
                .request("hermes", mind(), "shell", "set_web_search", lan.clone(), grade, &purpose, "", "", now, "12:03")
                .unwrap()
                .id;
            store.grant(&id, now, "12:03").unwrap();
            assert!(modes.person_add_rule_at("shell", "set_web_search", grade, &purpose, now).is_err(), "{mode:?}");
            assert!(modes.rules().is_empty());
            // That grant does not stand for another address, and the next call is asked about.
            let refused = store.consume(&id, "shell", "set_web_search", &other, now).unwrap_err();
            assert!(refused.contains("argument") || refused.contains("differ"), "{refused}");
            assert_eq!(modes.decide(grade, "shell", "set_web_search", false, "dangerous", now), Decision::Ask, "{mode:?}");
            // The grant is still good for the address it was given for, once.
            store.consume(&id, "shell", "set_web_search", &lan, now).unwrap();
            assert!(store.consume(&id, "shell", "set_web_search", &lan, now).is_err());
        }

        // What it was before: in auto, sensitive ran unasked, and a session rule covered every call.
        let mut auto = Modes::new(Mode::Ask);
        auto.person_set_mode(Mode::Auto, Bypass::Hour, now, 0);
        assert_eq!(auto.decide("sensitive", "shell", "set_web_search", false, "dangerous", now), Decision::Run { unasked: true });
    }

    #[test]
    fn agent_probes_are_one_at_a_time_and_one_per_ten_seconds() {
        let mut gate = ProbeGate::new();
        let t0 = Instant::now();
        gate.start(t0).unwrap();
        let busy = gate.start(t0 + Duration::from_secs(30)).unwrap_err();
        assert!(busy.contains("still running"), "{busy}");
        gate.finish();
        let soon = gate.start(t0 + Duration::from_secs(4)).unwrap_err();
        assert!(soon.contains("every 10 seconds") && soon.contains("in 6 s"), "{soon}");
        gate.start(t0 + PROBE_EVERY).unwrap();
        gate.finish();
        assert!(gate.start(t0 + PROBE_EVERY + Duration::from_secs(9)).is_err());
        gate.start(t0 + PROBE_EVERY * 2).unwrap();
    }

    #[test]
    fn the_card_names_the_address_and_what_the_model_reads_does_not() {
        let card = explain_set("searxng", "http://192.168.4.42:8888/");
        assert!(card.contains("http://192.168.4.42:8888 instead of") && card.contains("sees every query"), "{card}");
        assert!(explain_set("searxng", "http://search.example.com").starts_with("This would not go ahead"));
        assert!(explain_set("builtin", "").contains("DuckDuckGo's HTML search, and nowhere else"));
        assert!(explain_set("google", "").starts_with("This would not go ahead"));

        let src = include_str!("agent.rs");
        let describe = &src[src.find("pub fn describe()").unwrap()..src.find("pub fn explain_set").unwrap()];
        assert!(!describe.contains("\"url\"") && !describe.contains("searxng_url"), "{describe}");
        let replies = &src[src.find("pub fn set_from_agent").unwrap()..src.find("#[cfg(test)]").unwrap()];
        assert!(replies.contains("Ok(\"Testing the address;"), "the reply does not echo the address");
    }
}
