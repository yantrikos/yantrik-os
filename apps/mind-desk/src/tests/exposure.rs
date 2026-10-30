//! What a Mind acting in Mind View can read off the desk, and what it can press there.
//!
//! Both are held here, because a Mind in Mind View can see this window (a screenshot, the text
//! of every line) and click it: nothing on it may be an argument, a path, a prompt or a token, and
//! nothing it offers may decide anything that is the person's.

use yantrik_ipc_contracts::mind_desk::{self as wire, Counts, DeskTool, Evidence, MindState, Narration, Outcome};
use yantrik_services::mind_desk::FakeMindDesk;

use super::{hermes, snapshot, start, top_words};
use crate::{chrome, Command};

const SENTINELS: [&str; 10] = ["/home", "~/", "ssh", "secret", "token", "sk-live", ".gguf", "/usr/", "hunter2", "--"];

/// A snapshot as a shell with a bug, or a harness with bad manners, might send it: every string
/// field carrying something that is not for the chrome.
fn hostile() -> yantrik_ipc_contracts::mind_desk::DeskSnapshot {
    let mut mind = hermes(MindState::RunningTool);
    mind.narration = Some(Narration { text: "Reading /home/pranab/.ssh/secret --token=sk-live-abc".into(), heard: "14:32".into() });
    mind.tool = Some(DeskTool { name: "cat ~/.ssh/id_rsa hunter2".into(), evidence: Evidence::Verified, outcome: Outcome::Running });
    mind.model = Some("/opt/models/secret.gguf".into());
    mind.name = "Hermes".into();
    let mut snap = snapshot(1, Some(mind));
    snap.apps = vec!["/usr/bin/secret-tool".into(), "Blender".into()];
    snap
}

#[test]
fn nothing_a_hostile_snapshot_carries_reaches_either_window_or_the_minds_describe() {
    let (app, _) = start(FakeMindDesk::new());
    app.run(Command::Heard(Ok(hostile())));
    let shown = [
        top_words(&app),
        format!("{:?}", app.model(chrome::bottom)),
        app.describe().to_string(),
        format!("{:?}", app.model(chrome::top)),
    ]
    .join("\n");
    for sentinel in SENTINELS {
        assert!(!shown.contains(sentinel), "`{sentinel}` is on the desk: {shown}");
    }
    // What was fit to show still is: the Mind, its state, its app.
    assert!(shown.contains("HERMES · RUNNING A TOOL") && shown.contains("Blender"), "{shown}");
}

/// Across every state a Mind can be in, and with the fixture's own fields, no string the desk
/// draws is something the contract's cleaning would refuse, or longer than the line it is for.
#[test]
fn every_string_the_desk_draws_is_one_short_clean_line() {
    let (app, _) = start(FakeMindDesk::new());
    for (n, state) in [
        MindState::Thinking,
        MindState::RunningTool,
        MindState::WaitingForYou,
        MindState::Idle,
        MindState::Done,
        MindState::Failed,
        MindState::HarnessGone,
    ]
    .into_iter()
    .enumerate()
    {
        let mut mind = hermes(state);
        mind.counts = Counts { calls: 12, failed: 3, interrupted: 2 };
        app.run(Command::Heard(Ok(snapshot(n as u64 + 1, Some(mind)))));
        let top = app.model(chrome::top);
        let bottom = app.model(chrome::bottom);
        let lines = [
            &top.label, &top.headline, &top.reported, &top.confirm_question, &top.stop_note, &top.strip_heading,
            &top.strip_tool, &top.strip_mark, &top.strip_stats, &bottom.apps, &bottom.safety, &bottom.model, &bottom.model_mark,
        ];
        for line in lines {
            assert!(!line.contains('\n') && !line.chars().any(char::is_control), "{state:?}: {line:?}");
            assert!(line.chars().count() <= 200, "{state:?}: {line:?}");
            // The Mind's own words are quoted and checked; nothing else needs the words a path has.
            let unquoted = line.trim_matches(|c| c == '“' || c == '”');
            assert!(!wire::looks_private(unquoted), "{state:?}: `{line}` looks like an argument, a path or a secret");
        }
    }
}

#[test]
fn the_windows_source_has_no_tooltip_and_every_label_says_what_is_drawn() {
    let ui = format!("{}{}", include_str!("../../ui/app.slint"), include_str!("../../ui/desk.slint"));
    assert!(!ui.contains("tooltip"), "hover text is a place for a string nobody cleaned");
    // Every accessible label is a property the Rust side sets from the same chrome, never a
    // literal and never a different string from the one drawn.
    for line in ui.lines().filter(|l| l.trim_start().starts_with("accessible-label:")) {
        let value = line.trim().trim_start_matches("accessible-label:").trim().trim_end_matches(';');
        assert!(value.starts_with("root."), "an accessible label that is not what is drawn: {line}");
    }
}

/// The desk offers a person two things and a Mind the same two: it can stop (a Mind stopping
/// itself is harmless) and put up the person's Agents screen. It cannot decide anything.
#[test]
fn the_desk_offers_stop_and_the_agents_screen_and_no_decision() {
    let ui = format!("{}{}", include_str!("../../ui/app.slint"), include_str!("../../ui/desk.slint"));
    let mut callbacks: Vec<&str> = ui
        .lines()
        .filter_map(|l| l.trim().strip_prefix("callback "))
        .filter_map(|l| l.split('(').next())
        .collect();
    callbacks.sort_unstable();
    callbacks.dedup();
    assert_eq!(callbacks, ["agents-pressed", "stop-cancelled", "stop-confirmed", "stop-pressed"]);

    // The labels of what can be pressed.
    let labels: Vec<&str> = ui.lines().filter_map(|l| l.trim().strip_prefix("label: \"")).filter_map(|l| l.split('"').next()).collect();
    assert_eq!(labels, ["Stop", "Keep going", "Open in Agents", "Stop…"]);

    // No wording of a decision, a grant or a handover, in the screen or in what it is bound to.
    let rust = [include_str!("../model.rs"), include_str!("../update.rs"), include_str!("../chrome.rs"), include_str!("../view.rs"), include_str!("../words.rs")]
        .join("\n");
    for source in [ui.as_str(), rust.as_str()] {
        let source = source.split("#[cfg(test)]").next().unwrap().to_lowercase();
        for forbidden in ["approve", "deny", "allow", "grant", "take over", "take_over", "takeover", "hand back", "hand_back", "handback"] {
            // The words may appear only to say what is absent: a line of comment.
            let said_in_code = source
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .any(|l| l.contains(forbidden));
            assert!(!said_in_code, "`{forbidden}` is in the desk's code");
        }
    }
}

/// The framework gives every app one action of its own, `launch`, which hands a launch to the
/// running copy and shows its window. The desk adds none: it declares no intent, so a Mind that
/// reads its surface finds nothing to take but what every app has, and that takes nothing in.
#[test]
fn a_mind_that_reads_the_desks_surface_finds_no_action_of_the_desks() {
    let (app, _) = start(FakeMindDesk::new());
    let described = app.describe();
    let names: Vec<&str> = described["actions"]
        .as_array()
        .map(|a| a.iter().filter_map(|a| a["name"].as_str()).collect())
        .unwrap_or_default();
    assert_eq!(names, ["launch"], "{described}");
    // Launching hands it nothing to open: it has no file to take and no intent to run.
    let table = {
        let mut table = yantrik_app::intents::Intents::<crate::MindDesk>::new();
        <crate::MindDesk as yantrik_app::app::App>::intents(&mut table);
        table.specs().len()
    };
    assert_eq!(table, 0, "no intent: nothing a Mind can ask of the desk by name");
}
