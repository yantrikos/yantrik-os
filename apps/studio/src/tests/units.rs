//! The parts that need no window: the plan and the numbers it is held to, the sentences, the
//! summary, and the rule for an agent's files.

use std::path::Path;

use super::{apply, bare};
use crate::agent_rule;
use crate::command::Command;
use crate::config::Config;
use crate::describe;
use crate::gallery;
use crate::model::{display, short_prompt};
use crate::plan::{Plan, DEFAULT_HEIGHT, MAX_COUNT, MAX_EDGE, MAX_STEPS};
use crate::words;

#[test]
fn an_ask_that_cannot_work_is_refused_before_anything_is_queued() {
    let (_world, mut m) = bare("plan", r#"{"backend":{"kind":"fake"}}"#);
    for plan in [
        Plan::new(""),
        Plan::new("   "),
        Plan { count: 0, ..Plan::new("a lighthouse") },
        Plan { count: 9, ..Plan::new("a lighthouse") },
    ] {
        let fx = apply(&mut m, Command::Generate(Some(Box::new(plan.clone()))));
        assert!(!m.notice.is_empty());
        assert!(m.queue.is_empty(), "{} left a job in the queue", m.notice);
        assert!(fx.requests().iter().all(|r| r.name() != "spawn"), "a refused ask started a worker");
    }
}

#[test]
fn the_refusals_say_what_would_work() {
    let count = Plan { count: 12, ..Plan::new("x") }.checked().unwrap_err().agent;
    assert!(count.contains("12") && count.contains(&MAX_COUNT.to_string()), "{count}");
    assert!(count.contains("charges"), "{count}");
    let prompt = Plan::new("  ").checked().unwrap_err();
    assert!(prompt.agent.contains("prompt="), "{}", prompt.agent);
    assert!(!prompt.person.contains("prompt="), "the person is shown the action's shape: {}", prompt.person);
    let zero = Plan { count: 0, ..Plan::new("x") }.checked().unwrap_err().agent;
    assert!(zero.contains("count=0"), "{zero}");
    let huge = Plan { prompt: "a".repeat(5000), ..Plan::new("x") }.checked().unwrap_err().agent;
    assert!(huge.contains("5000"), "{huge}");
}

#[test]
fn the_numbers_are_brought_onto_the_grid_rather_than_refused() {
    let plan = Plan { width: 10_000, height: 0, steps: 900, cfg: 99.0, ..Plan::new("x") }.checked().unwrap();
    assert_eq!(plan.width, MAX_EDGE);
    assert_eq!(plan.height, DEFAULT_HEIGHT, "a height of 0 means the default, not an error");
    assert_eq!(plan.steps, MAX_STEPS);
    assert_eq!(plan.cfg, 30.0);
    let plan = Plan { negative: "   ".into(), ..Plan::new("x") }.checked().unwrap();
    assert_eq!(plan.negative, "", "an empty negative had this app's opinion substituted into it");
    let plan = Plan { prompt: "  a lighthouse  ".into(), ..Plan::new("x") }.checked().unwrap();
    assert_eq!(plan.prompt, "a lighthouse");
}

#[test]
fn the_summary_leads_with_trouble_and_says_where_the_pictures_are() {
    let (_world, mut m) = bare("summary", r#"{"backend":{"kind":"fake"}}"#);
    let idle = describe::summary(&m);
    assert!(idle.starts_with("Studio \u{2014} "), "{idle}");
    assert!(idle.contains("placeholder images"), "{idle}");
    assert!(!idle.contains("fake"), "the summary is read by a person: {idle}");
    assert!(idle.contains("nothing in the gallery yet"), "{idle}");
    assert_eq!(idle.lines().count(), 1, "{idle}");

    // Trouble goes first, because it is the reason to look.
    m.notice = "the server could not be reached".into();
    let troubled = describe::summary(&m);
    assert!(troubled.starts_with("Studio \u{2014} the server could not be reached"), "{troubled}");
    m.notice.clear();
    assert!(!describe::summary(&m).contains("reached"));

    m.config = Config::unconfigured();
    assert!(describe::summary(&m).contains("no backend configured"));
}

#[test]
fn a_path_is_written_with_the_home_folder_as_a_tilde() {
    let home = gallery::home();
    assert_eq!(display(&home.join("Pictures/Studio")), "~/Pictures/Studio");
    assert_eq!(display(Path::new("/srv/pictures")), "/srv/pictures");
}

#[test]
fn a_label_in_the_queue_is_readable_and_short() {
    assert_eq!(short_prompt("a lighthouse in fog"), "\u{201c}a lighthouse in fog\u{201d}");
    assert_eq!(short_prompt("  a   lighthouse  "), "\u{201c}a lighthouse\u{201d}");
    let long = "a very long prompt that goes on and on about a lighthouse in the fog";
    let label = short_prompt(long);
    assert!(label.chars().count() <= 43, "{label}");
    assert!(label.ends_with("\u{2026}\u{201d}"), "{label}");
    assert!(label.starts_with("\u{201c}a very long prompt"), "{label}");
}

/// The window never sends a person to an action: where a sentence for a Mind names `set_backend`,
/// the sentence in the window tells the person about the file instead.
#[test]
fn nothing_the_window_says_names_an_action_the_person_cannot_call() {
    let (_world, mut m) = bare("window-words", r#"{"backend":{"kind":"fake"}}"#);
    m.config = Config::unconfigured();
    let facts = words::facts_block(&m);
    assert!(facts.contains("No backend is configured"), "{facts}");
    assert!(facts.contains(crate::config::FILE_NAME), "{facts}");
    assert!(!facts.contains("set_backend"), "{facts}");
    // The Mind's own note still names the action that fixes it.
    assert!(m.facts().note.contains("set_backend"));

    apply(&mut m, Command::ExplainBackend);
    assert_eq!(
        m.notice,
        "Pictures are made by this machine right now. Changing where Studio sends prompts is the person's choice; to change it, edit ~/.config/yantrik/studio.json."
    );
    for agent_only in ["set_backend", "yos act", "describe", "generate prompt="] {
        assert!(!m.notice.contains(agent_only), "the button says `{agent_only}`: {}", m.notice);
    }
}

#[test]
fn an_agent_names_only_files_in_the_home() {
    let dir = std::env::temp_dir().join(format!("studio-agent-rule-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let outside = dir.join("outside.png");
    std::fs::write(&outside, "SENTINEL").unwrap();
    let outside = outside.display().to_string();
    let home = std::fs::canonicalize(&dir).unwrap().join("home");
    std::fs::create_dir_all(home.join("Pictures")).unwrap();
    std::fs::write(home.join("Pictures/ok.png"), "x").unwrap();

    assert_eq!(agent_rule::admit_in(false, "upscale", &outside, &home), Ok(()), "the person is not asked");
    for action in ["upscale", "open", "variations"] {
        for asked in [outside.as_str(), "/etc/hostname"] {
            let e = agent_rule::admit_in(true, action, asked, &home).expect_err(asked);
            assert!(e.starts_with("an agent names only a file in the person's home"), "{action} {asked}: {e}");
        }
        // A bare name is a picture in the gallery, resolved there.
        assert_eq!(agent_rule::admit_in(true, action, "studio-1.png", &home), Ok(()), "{action}");
        assert_eq!(
            agent_rule::admit_in(true, action, &home.join("Pictures/ok.png").display().to_string(), &home),
            Ok(()),
            "{action}: a file in the home"
        );
    }
    assert_eq!(agent_rule::admit_in(true, "delete", &outside, &home), Ok(()), "delete keeps to the gallery itself");
    // `set_backend` is refused outright by the framework, so this rule has nothing to say of it.
    assert!(!agent_rule::names_a_file("set_backend"));
    assert!(!agent_rule::names_a_file("generate"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A long Rust string split across lines must end each line with a backslash. When the backslash
/// is lost, the next line's indentation lands inside the sentence (it happened twice on 27 October).
/// This reads the app's own source for a string with a run of three spaces in it.
#[test]
fn no_sentence_has_the_indentation_of_a_lost_line_continuation_in_it() {
    let sources = [
        ("intents.rs", include_str!("../intents.rs")),
        ("words.rs", include_str!("../words.rs")),
        ("describe.rs", include_str!("../describe.rs")),
        ("model.rs", include_str!("../model.rs")),
        ("plan.rs", include_str!("../plan.rs")),
        ("said.rs", include_str!("../said.rs")),
        ("jobs.rs", include_str!("../jobs.rs")),
        ("config.rs", include_str!("../config.rs")),
        ("gallery.rs", include_str!("../gallery.rs")),
        ("backend.rs", include_str!("../backend.rs")),
        ("trash.rs", include_str!("../trash.rs")),
        ("workflow.rs", include_str!("../workflow.rs")),
        ("update/mod.rs", include_str!("../update/mod.rs")),
        ("update/make.rs", include_str!("../update/make.rs")),
        ("update/files.rs", include_str!("../update/files.rs")),
        ("update/gallery.rs", include_str!("../update/gallery.rs")),
        ("update/backend.rs", include_str!("../update/backend.rs")),
    ];
    for (file, source) in sources {
        for literal in string_literals(source) {
            assert!(!literal.contains("   "), "{file}: a sentence has a run of spaces in it: {literal:?}");
        }
    }
}

/// The contents of the ordinary string literals in `source`, a backslash at the end of a line
/// joining it to the next as the compiler reads it. Comments, raw strings and characters are
/// skipped; it reads source, not a grammar, and is good enough to hold one rule.
fn string_literals(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut found = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        match chars[at] {
            '/' if chars.get(at + 1) == Some(&'/') => {
                while at < chars.len() && chars[at] != '\n' {
                    at += 1;
                }
            }
            '\'' if chars.get(at + 2) == Some(&'\'') || (chars.get(at + 1) == Some(&'\\') && chars.get(at + 3) == Some(&'\'')) => {
                at += if chars.get(at + 1) == Some(&'\\') { 4 } else { 3 };
            }
            'r' if matches!(chars.get(at + 1), Some('#') | Some('"')) && !chars.get(at.wrapping_sub(1)).is_some_and(|c| c.is_alphanumeric()) => {
                let mut hashes = 0;
                let mut end = at + 1;
                while chars.get(end) == Some(&'#') {
                    hashes += 1;
                    end += 1;
                }
                if chars.get(end) != Some(&'"') {
                    at += 1;
                    continue;
                }
                end += 1;
                let closing: String = std::iter::once('"').chain(std::iter::repeat_n('#', hashes)).collect();
                let rest: String = chars[end..].iter().collect();
                at = end + rest.find(&closing).map(|i| rest[..i].chars().count() + closing.chars().count()).unwrap_or(rest.chars().count());
            }
            '"' => {
                let mut literal = String::new();
                at += 1;
                while at < chars.len() && chars[at] != '"' {
                    if chars[at] == '\\' {
                        match chars.get(at + 1) {
                            Some('\n') => {
                                at += 2;
                                while chars.get(at).is_some_and(|c| c.is_whitespace()) {
                                    at += 1;
                                }
                                continue;
                            }
                            Some(next) => {
                                literal.push(*next);
                                at += 2;
                                continue;
                            }
                            None => break,
                        }
                    }
                    literal.push(chars[at]);
                    at += 1;
                }
                found.push(literal);
                at += 1;
            }
            _ => at += 1,
        }
    }
    found
}

#[test]
fn the_literal_reader_reads_a_continuation_and_a_lost_one() {
    let kept = string_literals("let a = \"one \\\n        two\";");
    assert_eq!(kept, ["one two"]);
    let lost = string_literals("let a = \"one\n        two\";");
    assert!(lost[0].contains("   "), "{lost:?}");
    assert_eq!(string_literals("// \"not a string\"\nlet c = '\"';"), Vec::<String>::new());
}
