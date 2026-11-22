//! The four questions against the shell's real surface machinery (#1313, round 5): M1, that no
//! action name is declared twice and each question publishes its own grade and sentence; and L2, that
//! nothing but the four changed which mode asks.

use std::time::Instant;

use yantrik_companion::tools::asks::Card;
use yantrik_ipc_transport::gate;
use yantrik_surface::LocalRegistry;

use super::*;
use crate::mind_mode::{Bypass, Declared, Decision, Mode, Modes};

/// Every literal `Action::new("name"` of the shell's `control*.rs`, with its grade (`.risk("x")`, else
/// `standard`, the grade `Action::new` gives), outside test code.
fn declared_in_the_shell() -> Vec<(String, String, String)> {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files: Vec<_> = std::fs::read_dir(&src)
        .expect("src")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("control") && n.ends_with(".rs") && !n.ends_with("tests.rs")))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        let code = text.split("#[cfg(test)]").next().unwrap_or("");
        let marker = "Action::new(";
        for (at, _) in code.match_indices(marker) {
            // rustfmt may put the name on the next line.
            let Some(literal) = code[at + marker.len()..].trim_start().strip_prefix('"') else { continue };
            let name = literal.split('"').next().unwrap_or("").to_string();
            let end = code[at + marker.len()..].find(marker).map_or(code.len(), |e| at + marker.len() + e);
            let span = &code[at..end];
            let grade = span
                .find(".risk(\"")
                .and_then(|p| span[p + 7..].split('"').next())
                .unwrap_or("standard")
                .to_string();
            out.push((name, grade, file.file_name().unwrap().to_string_lossy().to_string()));
        }
    }
    out
}

/// M1: no name is declared twice on the shell, none of the four is declared anywhere but here, and a
/// registry holding the Lens's `send_message` as it is on `main` and the four questions answers each
/// name with its own grade and sentence.
#[test]
fn no_action_name_is_declared_twice_on_the_shell_and_each_question_is_its_own() {
    let declared = declared_in_the_shell();
    assert!(declared.len() > 100, "the scan read {} actions", declared.len());
    let mut seen = std::collections::HashMap::new();
    for (name, _, file) in &declared {
        if let Some(first) = seen.insert(name.clone(), file.clone()) {
            panic!("the shell declares `{name}` twice: in {first} and in {file}");
        }
        assert!(QUESTIONS.iter().all(|q| q.name != name), "`{name}` is a question's name, declared in {file}");
    }
    // The Lens's own action is still there, under the name it always had.
    assert_eq!(seen.get("send_message").map(String::as_str), Some("control.rs"));

    let mut registry = LocalRegistry::new("shell");
    registry.add(
        Action::new("send_message", "Ask the desktop something, as if typed into the Lens").open_ended().arg(Param::text("text").describe("What to say")),
        Box::new(|_| Ok(serde_json::Value::Null)),
    );
    for q in &QUESTIONS {
        registry.add(publish(q), Box::new(|_| Err("a question".to_string())));
    }
    let names: Vec<String> = registry.specs().into_iter().map(|a| a.name).collect();
    let mut unique = names.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(names.len(), unique.len(), "a name is declared twice: {names:?}");
    for q in &QUESTIONS {
        assert_eq!(registry.published_grade(q.name), Some(q.grade), "{}: its own grade", q.name);
        let said = registry.published_description(q.name).unwrap_or_default();
        assert!(said.contains(q.sentence), "{}: its own sentence, not another action's: {said}", q.name);
    }
    // The Lens action is untouched by any of it.
    assert_eq!(registry.published_grade("send_message"), Some("standard"));
    assert!(registry.published_description("send_message").unwrap().starts_with("Ask the desktop something"));
}

/// The companion's chat send reaches a card: the card it raises (`tool`, `to`, `text`) passes the
/// registry's own check (`published_detail_full_in` runs it before any card is put up), the shell
/// asks about it in every mode but plan, and plan refuses. Before the rename, the Lens action of the
/// same name took every lookup and the check refused the card.
#[test]
fn the_companions_chat_send_reaches_a_card_in_every_mode() {
    let mut registry = LocalRegistry::new("shell");
    registry.add(Action::new("send_message", "Ask the desktop something, as if typed into the Lens").open_ended().arg(Param::text("text")), Box::new(|_| Ok(serde_json::Value::Null)));
    for q in &QUESTIONS {
        registry.add(publish(q), Box::new(|_| Err("a question".to_string())));
    }
    let name = yantrik_companion::tools::asks::Question::SendChatMessage.action();
    let card = Card::new("whatsapp_send").plain("to", "WhatsApp +1 555 0100").text("text", "Running late, start without me").args();
    registry.check_call(name, &card).expect("the card is one the registry accepts");
    let q = QUESTIONS.iter().find(|q| q.name == name).expect("the question is published");
    let now = Instant::now();
    for mode in [Mode::Ask, Mode::Auto, Mode::Bypass, Mode::BypassAll] {
        let modes = in_mode(mode, now);
        let said = modes.decide(registry.published_grade(name).unwrap(), "shell", name, gate::unrecoverable(&registry.published_description(name).unwrap()), "dangerous", now);
        // Full bypass asks nothing (#1353, decision D); every other mode asks.
        assert_eq!(matches!(said, Decision::Ask), mode != Mode::BypassAll, "{mode:?}: {said:?}");
    }
    let plan = in_mode(Mode::Plan, now).decide(q.grade, "shell", name, true, "dangerous", now);
    assert!(matches!(plan, Decision::Refuse { .. }));
    // And the Lens's `send_message` is asked about as it was on `main`: as an open-ended standard action.
    let lens = |mode| in_mode(mode, now).decide("standard", "shell", "send_message", Declared { cannot_be_undone: false, open_ended: true }, "dangerous", now);
    assert!(matches!(lens(Mode::BypassAll), Decision::Run { .. }), "full bypass runs the Lens's send as before");
}

fn in_mode(mode: Mode, now: Instant) -> Modes {
    let mut modes = Modes::new(Mode::Ask);
    if mode != Mode::Ask {
        modes.person_set_mode(mode, Bypass::Hour, now, 0);
    }
    modes
}

/// What a decision says, with the names left out of a refusal's sentence.
fn kind(d: Decision) -> String {
    match d {
        Decision::Refuse { .. } => "refuse".to_string(),
        other => format!("{other:?}"),
    }
}

/// L2: every action the shell publishes, in all five modes and with each combination of what its
/// sentence may declare, is decided exactly as a name that is on no list: so putting the four
/// questions on `ALWAYS_ASKED` changed which mode asks for nothing but them. The three older
/// consents are `ALWAYS_ASKED` on `main` and are pinned by `control_research`'s test.
#[test]
fn nothing_else_changed() {
    let now = Instant::now();
    let older = ["allow_location_lookup", "allow_background_research", "report_problem"];
    let mut rows: Vec<(String, String, String)> = declared_in_the_shell().into_iter().map(|(n, g, _)| ("shell".to_string(), n, g)).collect();
    // The four questions' names under another app are another app's actions, as before.
    for q in &QUESTIONS {
        for grade in ["safe", "standard", "sensitive", "dangerous"] {
            rows.push(("notes".to_string(), q.name.to_string(), grade.to_string()));
        }
    }
    let mut compared = 0;
    for (app, action, grade) in &rows {
        if app == "shell" && (QUESTIONS.iter().any(|q| q.name == action) || older.contains(&action.as_str())) {
            continue;
        }
        for mode in [Mode::Plan, Mode::Ask, Mode::Auto, Mode::Bypass, Mode::BypassAll] {
            let modes = in_mode(mode, now);
            for (cannot_be_undone, open_ended) in [(false, false), (true, false), (false, true), (true, true)] {
                let declared = Declared { cannot_be_undone, open_ended };
                let this = kind(modes.decide(grade, app, action, declared, "dangerous", now));
                let neutral = kind(modes.decide(grade, app, "zz_on_no_list", declared, "dangerous", now));
                assert_eq!(this, neutral, "{app}.{action} ({grade}) in {mode:?}, declared {declared:?}: not decided as an action on no list");
                compared += 1;
            }
        }
    }
    assert!(compared > 2000, "{compared} decisions compared");
}
