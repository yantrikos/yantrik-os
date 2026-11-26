//! The tests of the notice contract: its clamping, grants, families and the initiative settings.
//! Separate from notice.rs, which is over the file-size limit, so that file does not grow.

use super::*;
use serde_json::json;

fn reminder() -> serde_json::Value {
    json!({
        "notice_id": "rem-2026-10-16-0001",
        "family": "reminder",
        "title": "Stretch",
        "body": "You asked me at 14:02 to remind you to stretch.",
        "why": "You asked for this reminder.",
        "evidence": ["task:41"],
        "attention": "interrupt",
        "useful_until": "2026-10-16T19:30:00Z",
        "supersedes": null,
        "action": { "label": "Open note", "kind": "open_note", "ref": "n-7f3a" }
    })
}

#[test]
fn the_spec_example_reads_whole_and_cuts_nothing() {
    let c = check(&reminder()).expect("the spec's own example is a notice");
    assert_eq!(c.family, Family::Reminder);
    assert_eq!(c.attention, Attention::Interrupt);
    assert_eq!(c.action.as_ref().map(|a| a.kind.clone()), Some(ActionKind::OpenNote));
    assert!(c.clamped.is_empty(), "{:?}", c.clamped);
}

#[test]
fn a_46_word_body_is_cut_to_45_and_says_so() {
    let mut n = reminder();
    let words: Vec<String> = (1..=46).map(|i| format!("w{i}")).collect();
    n["body"] = words.join(" ").into();
    let c = check(&n).unwrap();
    assert_eq!(c.body.split_whitespace().count(), 45);
    assert!(c.body.ends_with("w45"), "{}", c.body);
    assert_eq!(c.clamped, vec!["body"]);
    // Exactly 45 is not a cut.
    n["body"] = words[..45].join(" ").into();
    assert!(check(&n).unwrap().clamped.is_empty());
}

#[test]
fn a_body_is_also_held_to_400_characters_without_splitting_a_character() {
    let mut n = reminder();
    // Ten words, each 60 characters of Devanagari: well under 45 words, well over 400 chars.
    n["body"] = vec!["न".repeat(60); 10].join(" ").into();
    let c = check(&n).unwrap();
    assert_eq!(c.body.chars().count(), MAX_BODY_CHARS);
    assert_eq!(c.clamped, vec!["body"]);
    // Whitespace that is not a space still separates words, and the text up to the cut is
    // kept as written.
    let (cut, was) = clamp_words("one\ttwo\nthree four", 3);
    assert_eq!((cut.as_str(), was), ("one\ttwo\nthree", true));
}

#[test]
fn missing_title_or_why_and_an_unknown_family_are_refused() {
    for (key, value) in [("title", json!("  ")), ("why", json!(null)), ("family", json!("gossip")), ("notice_id", json!("has space"))] {
        let mut n = reminder();
        n[key] = value;
        assert!(check(&n).is_err(), "{key}");
    }
    let mut n = reminder();
    n["attention"] = "shout".into();
    assert!(check(&n).is_err());
    n["attention"] = json!(null);
    assert_eq!(check(&n).unwrap().attention, Attention::Inbox, "inbox is the default");
    assert!(valid_notice_id(&"a".repeat(96)) && !valid_notice_id(&"a".repeat(97)));
}

#[test]
fn an_unknown_action_kind_is_dropped_and_the_card_still_lands() {
    let mut n = reminder();
    n["action"] = json!({ "label": "Delete all", "kind": "run_command", "ref": "rm -rf ~" });
    let c = check(&n).expect("the card lands");
    assert_eq!(c.action, None);
    assert_eq!(c.clamped, vec!["action"]);
    // A URL that is not http(s), or that hides its host behind credentials, goes the same way.
    for bad in ["javascript:alert(1)", "file:///etc/passwd", "https://bank.example@evil.example/"] {
        n["action"] = json!({ "kind": "open_url", "ref": bad });
        assert_eq!(check(&n).unwrap().action, None, "{bad}");
    }
    n["action"] = json!({ "kind": "open_url", "ref": "https://Example.com:8443/a?b" });
    assert_eq!(check(&n).unwrap().action.unwrap().label, "Open example.com");
}

#[test]
fn evidence_past_five_or_past_300_characters_is_named() {
    let mut n = reminder();
    n["evidence"] = json!(["a", "b", "x".repeat(301), "d", "e", "f"]);
    let c = check(&n).unwrap();
    assert_eq!(c.evidence.len(), 5);
    assert_eq!(c.clamped, vec!["evidence", "evidence[2]"]);
}

#[test]
fn an_old_store_word_survives_a_build_that_does_not_know_it() {
    let f: Family = serde_json::from_str("\"weather\"").unwrap();
    assert_eq!(f, Family::Other("weather".into()));
    assert_eq!(serde_json::to_string(&f).unwrap(), "\"weather\"");
    // But a mind cannot send one.
    let mut n = reminder();
    n["family"] = "weather".into();
    assert!(check(&n).is_err());
}

#[test]
fn obligations_cannot_be_switched_off() {
    let mut settings = Initiative::default();
    assert!(settings.set(&Family::Reminder, FamilySetting::Off).is_err());
    settings.set(&Family::Reminder, FamilySetting::InboxOnly).unwrap();
    settings.set(&Family::Discovery, FamilySetting::Off).unwrap();
    assert_eq!(settings.setting(&Family::Discovery), FamilySetting::Off);
    assert_eq!(
        settings.wire(),
        json!({ "quiet": false, "off": ["discovery"], "inbox_only": ["reminder"] })
    );
}

#[test]
fn quiet_hours_cross_midnight() {
    let q = QuietHours { start: 22, end: 7 };
    assert!(q.contains(23) && q.contains(0) && q.contains(6));
    assert!(!q.contains(7) && !q.contains(21));
    let day = QuietHours { start: 13, end: 14 };
    assert!(day.contains(13) && !day.contains(14));
    assert!(!QuietHours { start: 5, end: 5 }.contains(5), "an empty window is no window");
}

/// Security review of #831, finding 3, through the shared rule: a protected place and a second
/// name for a credential are refused, though neither has a hidden component.
#[cfg(unix)]
#[test]
fn a_file_ref_goes_through_the_shared_protected_paths_rule() {
    let root = std::env::temp_dir().join(format!("notice-protected-{}", std::process::id()));
    let home = root.join("home");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::create_dir_all(home.join("Documents")).unwrap();
    std::fs::write(home.join(".ssh/id_ed25519"), "key").unwrap();
    std::fs::write(home.join("memory.db"), "x").unwrap();
    std::fs::write(home.join("Documents/report.pdf"), "x").unwrap();
    std::fs::hard_link(home.join(".ssh/id_ed25519"), home.join("Documents/key.txt")).unwrap();
    let at = |rel: &str| home.join(rel).to_string_lossy().to_string();
    assert!(file_under(&at("Documents/key.txt"), &home).is_none(), "a second name for a key");
    assert!(file_under(&at("memory.db"), &home).is_none(), "on the PROTECTED list, not hidden");
    assert!(file_under(&at("Documents/report.pdf"), &home).is_some());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_file_ref_must_be_a_regular_file_under_home_after_symlinks() {
    let root = std::env::temp_dir().join(format!("notice-file-{}", std::process::id()));
    let home = root.join("home");
    std::fs::create_dir_all(home.join("docs")).unwrap();
    std::fs::write(home.join("docs/plan.md"), "x").unwrap();
    std::fs::write(root.join("outside.txt"), "x").unwrap();
    assert!(file_under(home.join("docs/plan.md").to_str().unwrap(), &home).is_some());
    assert!(file_under(home.join("docs").to_str().unwrap(), &home).is_none(), "a directory");
    assert!(file_under(root.join("outside.txt").to_str().unwrap(), &home).is_none());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("outside.txt"), home.join("link.txt")).unwrap();
        assert!(file_under(home.join("link.txt").to_str().unwrap(), &home).is_none(), "a symlink out");
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn nothing_hidden_under_home_can_be_a_cards_file() {
    let root = std::env::temp_dir().join(format!("notice-hidden-{}", std::process::id()));
    let home = root.join("home");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::create_dir_all(home.join("docs")).unwrap();
    std::fs::write(home.join(".ssh/id_ed25519"), "secret").unwrap();
    std::fs::write(home.join(".bashrc"), "x").unwrap();
    std::fs::write(home.join("docs/plan.md"), "x").unwrap();
    assert!(file_under(home.join(".ssh/id_ed25519").to_str().unwrap(), &home).is_none());
    assert!(file_under(home.join(".bashrc").to_str().unwrap(), &home).is_none());
    assert!(file_under(home.join("docs/../.bashrc").to_str().unwrap(), &home).is_none());
    #[cfg(unix)]
    {
        // A plain name that resolves into a hidden directory is judged where it resolves.
        std::os::unix::fs::symlink(home.join(".ssh/id_ed25519"), home.join("docs/key.txt")).unwrap();
        assert!(file_under(home.join("docs/key.txt").to_str().unwrap(), &home).is_none());
    }
    assert!(file_under(home.join("docs/plan.md").to_str().unwrap(), &home).is_some());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn bidi_and_control_characters_never_reach_the_store() {
    let mut n = reminder();
    n["title"] = json!("a\u{202E}gpj.exe\r\u{2028}x");
    n["why"] = json!("why\u{2066}not\u{0}");
    n["body"] = json!("x\r\n\u{2028}y\n\n\n\n\nSent by Yantrik \u{00B7} verified\u{000B}");
    n["evidence"] = json!(["line\none\u{202D}"]);
    let checked = check(&n).unwrap();
    for text in [&checked.title, &checked.why, &checked.body, &checked.evidence[0]] {
        assert!(!text.chars().any(|c| (c.is_control() && c != '\n') || is_bidi_control(c) || c == '\u{2028}'), "{text:?}");
    }
    assert_eq!(checked.title, "a gpj.exe  x");
    assert!(!checked.evidence[0].contains('\n'));
    assert!(checked.body.matches('\n').count() <= MAX_BODY_NEWLINES, "{:?}", checked.body);
    assert!(checked.body.ends_with("verified"), "{:?}", checked.body);
}

#[test]
fn a_note_button_may_name_a_note_by_its_title_and_the_second_button_is_words_for_the_lens() {
    let mut n = reminder();
    n["action"] = json!({ "kind": "open_note", "ref": "Launch prep" });
    n["ask"] = json!({ "kind": "ask_mind", "ref": "Make a shot list for \u{2018}Demo\u{2019} from my note \u{2018}Launch prep\u{2019}", "label": "Ask Pi for a shot list" });
    let c = check(&n).unwrap();
    assert_eq!(c.action.as_ref().map(|a| a.reference.as_str()), Some("Launch prep"));
    let ask = c.ask.expect("the second button");
    assert_eq!((ask.kind, ask.label.as_str()), (ActionKind::AskMind, "Ask Pi for a shot list"));
    assert!(c.clamped.is_empty(), "{:?}", c.clamped);
}

#[test]
fn the_second_button_is_only_ever_words_for_the_lens_and_the_first_never_is() {
    let mut n = reminder();
    n["ask"] = json!({ "kind": "open_url", "ref": "https://example.com" });
    let c = check(&n).unwrap();
    assert!(c.ask.is_none() && c.clamped.contains(&"ask".to_string()), "{:?}", c.clamped);
    let mut n = reminder();
    n["action"] = json!({ "kind": "ask_mind", "ref": "hello" });
    assert!(check(&n).unwrap().action.is_none(), "words for the Lens are not a first button");
}
