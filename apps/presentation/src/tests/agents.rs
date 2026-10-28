//! An agent names only files in the person's home, outside its protected places (#443, #942), and
//! is not shown a deck whose file it may not read.
//!
//! The scratch folders are in the temporary directory, outside the home, which is exactly what an
//! agent is refused and the person is not.

use std::path::PathBuf;

use serde_json::{json, Value};
use yantrik_app::control::{AgentTokenScope, View};
use yantrik_testing::TestApp;

use super::{act, act_as_agent, notice, outline, scratch, started, type_title, with_file};
use crate::agent_rule::{HIDDEN, HIDDEN_REFUSAL};
use crate::YPresent;

const SENTINEL: &str = "SENTINEL-ypresent-7f3a-do-not-show";

/// `describe` as an agent with this token reads it.
fn described_as(app: &TestApp<YPresent>, token: &str) -> View {
    let token = token.to_string();
    app.model(move |m| {
        let _agent = AgentTokenScope::enter(Some(token));
        crate::describe::view(m)
    })
}

fn outside_home(dir: &std::path::Path) -> bool {
    !dir.starts_with(PathBuf::from(std::env::var_os("HOME").unwrap_or_default()))
}

/// A deck the person opened from outside the home, carrying the sentinel.
fn secret_deck(name: &str) -> (TestApp<YPresent>, PathBuf, PathBuf) {
    let (app, path) = with_file(name, &["omega"]);
    super::act(&app, "set_slide", json!({ "index": 0, "body": SENTINEL, "notes": SENTINEL })).unwrap();
    super::act(&app, "save", super::none()).unwrap();
    let dir = path.parent().unwrap().to_path_buf();
    (app, path, dir)
}

#[test]
fn an_agent_names_only_files_in_the_home_and_is_not_shown_the_persons_other_decks() {
    let (app, path, dir) = secret_deck("agent-home");
    if !outside_home(&dir) {
        return; // A temp folder inside the home would be allowed, and prove nothing here.
    }
    assert!(state_of(&app)["current"]["body"].as_str().unwrap().contains(SENTINEL), "the person sees the text");

    let refused = |name: &str, args: Value, why: &str| {
        let err = act_as_agent(&app, "tok-ypresent", name, args.clone()).expect_err(&format!("{name} {args}"));
        assert!(err.ends_with(why), "{name} {args}: {err}");
        assert!(!err.contains(SENTINEL), "{err}");
    };
    refused("open", json!({ "path": "~/.ssh/id_ed25519" }), " is protected");
    for own in [
        "~/.local/share/yantrik/snippets/snippets.json",
        "~/.local/share/yantrik/email/draft.json",
        "~/.local/state/yantrik/email/drafts/composing.json",
        "~/.local/state/yantrik/notes/drafts/note-today.json",
        "~/.local/share/yantrik-mind/yantrik-memory.token",
    ] {
        refused("open", json!({ "path": own }), " is protected");
    }
    refused("open", json!({ "path": "/etc/hostname" }), " is outside");
    refused("open", json!({ "path": path.display().to_string() }), " is outside");
    refused("save", super::none(), " is outside");
    // The refusal of a save of the deck it is not shown names neither the file nor its folder.
    let err = act_as_agent(&app, "tok-ypresent", "save", super::none()).unwrap_err();
    assert!(!err.contains("talk.ydeck") && !err.contains(&dir.display().to_string()), "{err}");
    for (name, args) in [
        ("go_to", json!({ "index": 0 })),
        ("next", json!({})),
        ("previous", json!({})),
        ("set_slide", json!({ "index": 0, "title": "x" })),
        ("add_slide", json!({ "title": "x" })),
        ("delete_slide", json!({ "index": 0 })),
        ("move_slide", json!({ "from": 0, "to": 0 })),
        ("presenter_view", json!({ "mode": "on" })),
        ("save_as", json!({ "path": "~/Documents/copied-out.ydeck" })),
        ("export_markdown", json!({ "path": "~/Documents/copied-out.md" })),
        ("export_markdown", json!({})),
    ] {
        refused(name, args, HIDDEN_REFUSAL);
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    assert!(!home.join("Documents/copied-out.md").exists() && !home.join("Documents/copied-out.ydeck").exists());
    assert!(super::file(&path).contains(SENTINEL), "the deck's file is untouched");
    assert_eq!(super::listing(&dir), vec!["talk.ydeck"], "and nothing was exported beside it");

    // Nor is it read back through `describe`: not its text, its name, its path, its outline or a
    // match count.
    let seen = described_as(&app, "tok-ypresent");
    let s = &seen.state;
    assert_eq!(s["file"], json!(HIDDEN), "{s}");
    assert_eq!(s["content_hidden"], json!("the deck is a file an agent is not shown"));
    let keys: Vec<&str> = s.as_object().unwrap().keys().map(String::as_str).collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, vec!["content_hidden", "dirty", "file", "notice", "presenting"]);
    let listed = s.to_string();
    assert!(!listed.contains("talk.ydeck") && !listed.contains(&dir.display().to_string()), "{listed}");
    assert!(!listed.contains(SENTINEL) && !seen.summary.contains("omega") && !seen.summary.contains("Talk"), "{}", seen.summary);
    assert_eq!(seen.summary, format!("yPresent \u{2014} {HIDDEN}"));
}

fn state_of(app: &TestApp<YPresent>) -> Value {
    super::state(app)
}

#[test]
fn the_person_is_shown_their_own_deck_and_may_do_everything_with_it() {
    let (app, _, dir) = secret_deck("person-sees");
    if !outside_home(&dir) {
        return;
    }
    let state = state_of(&app);
    assert!(state["file"].is_string() && state["file"] != json!(HIDDEN));
    assert_eq!(state["outline"], json!(["omega"]));
    act(&app, "export_markdown", json!({ "path": dir.join("copy.md").display().to_string() })).unwrap();
    act(&app, "go_to", json!({ "index": 0 })).unwrap();
}

#[test]
fn an_agents_own_deck_is_written_only_where_the_write_rule_allows() {
    let app = started();
    let dir = scratch("agent-write");
    if !outside_home(&dir) {
        return;
    }
    act_as_agent(&app, "tok-ypresent", "set_slide", json!({ "index": 0, "title": "echo pwned" })).unwrap();
    let refused = |name: &str, path: &str, why: &str| {
        let err = act_as_agent(&app, "tok-ypresent", name, json!({ "path": path })).expect_err(path);
        assert!(err.ends_with(why), "{name} {path}: {err}");
    };
    for name in ["save_as", "export_markdown"] {
        refused(name, "~/.bashrc", " is protected");
        refused(name, "~/.config/autostart/x.desktop", " is protected");
        refused(name, &dir.join("agent.ydeck").display().to_string(), " is outside");
    }
    assert!(!dir.join("agent.ydeck").exists(), "nothing was written");
    // A save with no file goes to the deck folder, which is the same rule: outside the home here.
    let err = act_as_agent(&app, "tok-ypresent", "save", super::none()).unwrap_err();
    assert!(err.ends_with(" is outside"), "{err}");
    // The person writes where they choose.
    let path = dir.join("person.ydeck");
    act(&app, "save_as", json!({ "path": path.display().to_string() })).unwrap();
    assert!(super::file(&path).contains("echo pwned"));
}

#[test]
fn a_deck_that_is_replaced_is_first_written_only_where_an_agent_may_write() {
    let (app, path, dir) = secret_deck("agent-replace");
    if !outside_home(&dir) {
        return;
    }
    type_title(&app, "unsaved edit");
    let before = super::file(&path);
    // `open` and `new_deck` first write the deck in front, by the write rule, and nothing is
    // replaced if that is refused.
    let err = act_as_agent(&app, "tok-ypresent", "new_deck", super::none()).unwrap_err();
    assert!(err.ends_with(" is outside. The deck was not replaced."), "{err}");
    assert!(!err.contains(SENTINEL), "{err}");
    assert_eq!(outline(&app), vec!["unsaved edit"]);
    assert_eq!(super::file(&path), before);
}

#[test]
fn what_an_agent_reads_in_the_notice_about_a_hidden_deck_names_no_file() {
    let (app, path, dir) = secret_deck("agent-notice");
    if !outside_home(&dir) {
        return;
    }
    type_title(&app, "an unsaved line");
    std::fs::write(&path, super::deck_text("Theirs", &["theirs"])).unwrap();
    app.look_at_files();
    super::settle(&app);
    let person = notice(&app);
    assert!(person.contains("talk.ydeck") || person.contains("omega") || person.contains("Talk"), "{person}");
    let seen = described_as(&app, "tok-ypresent");
    let for_agent = seen.state["notice"].as_str().unwrap().to_string();
    assert!(!for_agent.contains("Talk") && !for_agent.contains("talk.ydeck") && !for_agent.contains(&dir.display().to_string()), "{for_agent}");
    assert!(for_agent.contains(HIDDEN), "{for_agent}");
}

#[test]
fn a_path_with_spaces_round_the_name_opens_and_writes_what_was_checked() {
    let app = started();
    let dir = scratch("trimmed");
    let deck = dir.join("deck.ydeck");
    super::write_deck(&deck, "Deck", &["one"]);
    act(&app, "open", json!({ "path": format!("  {}  ", deck.display()) })).expect("the trimmed name is the one opened");
    assert_eq!(super::path_of(&app), Some(deck.clone()));
    let copy = dir.join("copy.md");
    act(&app, "export_markdown", json!({ "path": format!("{} ", copy.display()) })).unwrap();
    assert!(copy.exists() && !dir.join("copy.md ").exists(), "written where it was checked");
}

/// A folder of this test's own inside the real home, for the deck folder: the agent rule judges
/// where a file goes, so the copy must be able to land somewhere it allows.
fn folder_in_home(name: &str) -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let dir = home.join(format!("ypresent-test-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// A hidden deck with an unsaved edit carrying the sentinel, and the deck folder inside the home.
fn hidden_and_unsaved(name: &str) -> Option<(TestApp<YPresent>, PathBuf, PathBuf)> {
    let (app, path, dir) = secret_deck(name);
    if !outside_home(&dir) {
        return None;
    }
    let folder = folder_in_home(name)?;
    app.run(crate::command::Command::UseDeckDir(folder.clone()));
    type_title(&app, &format!("unsaved {SENTINEL}"));
    Some((app, path, folder))
}

/// An agent may not move a deck it is not shown into the home by replacing it: `new_deck` and
/// `open` write the deck being replaced to a file of its own first, and that copy is somewhere the
/// agent may read.
#[test]
fn an_agent_cannot_have_a_hidden_deck_copied_into_the_home_by_replacing_it() {
    for gone in ["deleted", "conflict"] {
        let Some((app, path, folder)) = hidden_and_unsaved(&format!("rescue-{gone}")) else { return };
        if gone == "deleted" {
            std::fs::remove_file(&path).unwrap();
            app.look_at_files();
        } else {
            super::outside_writes(&app, &path, &super::deck_text("Theirs", &["theirs"]));
        }
        super::settle(&app);
        assert!(super::modified(&app), "{gone}");
        // A deck an agent may read, so only the replacement is in question.
        let other = folder.join("o.ydeck");
        super::write_deck(&other, "O", &["o"]);
        for (name, args) in [("new_deck", json!({})), ("open", json!({ "path": other.display().to_string() }))] {
            let err = act_as_agent(&app, "tok-ypresent", name, args).unwrap_err();
            assert_eq!(err, HIDDEN_REFUSAL, "{gone} {name}");
        }
        assert_eq!(super::listing(&folder), vec!["o.ydeck"], "{gone}: nothing was copied into the home");
        assert_eq!(super::outline(&app).len(), 1, "{gone}: the deck stays in front");
        app.run_writes();
        assert!(super::draft_file(&app).exists(), "{gone}: and its draft is kept");
        // The person's own replacement is unchanged: the deck is kept in a file of its own.
        let answer = act(&app, "new_deck", super::none()).unwrap();
        let kept = answer["previous_deck_saved_to"].as_str().unwrap().to_string();
        assert!(kept.starts_with(&folder.display().to_string()), "{gone}: {kept}");
        assert!(super::file(std::path::Path::new(&kept)).contains(SENTINEL), "{gone}: the person's copy holds the deck");
        let _ = std::fs::remove_dir_all(&folder);
    }
}

#[test]
fn what_an_agent_reads_in_the_notice_names_no_file_after_an_export_or_a_replacement() {
    let (app, _, dir) = secret_deck("agent-notice-2");
    if !outside_home(&dir) {
        return;
    }
    super::press(&app, crate::command::WindowAction::ExportMarkdown);
    assert!(notice(&app).starts_with("Exported to "), "{}", notice(&app));
    let seen = described_as(&app, "tok-ypresent");
    let for_agent = seen.state["notice"].as_str().unwrap().to_string();
    assert!(!for_agent.contains("talk.md") && !for_agent.contains(&dir.display().to_string()), "{for_agent}");
    // An export that fails says why to the person, with the file; an agent reads no name.
    let md = dir.join("talk.md");
    let mut mode = std::fs::metadata(&md).unwrap().permissions();
    mode.set_readonly(true);
    std::fs::set_permissions(&md, mode).unwrap();
    super::press(&app, crate::command::WindowAction::ExportMarkdown);
    assert!(notice(&app).contains("read-only"), "{}", notice(&app));
    let for_agent = described_as(&app, "tok-ypresent").state["notice"].as_str().unwrap().to_string();
    assert!(!for_agent.contains("talk.md") && !for_agent.contains(&dir.display().to_string()), "{for_agent}");
    std::fs::set_permissions(&md, std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();
    // A deck replaced by the person's own Open says where it went; an agent reads no path.
    type_title(&app, "edited");
    let recent = scratch("agent-notice-recent");
    super::write_deck(&recent.join("only.ydeck"), "Only", &["only"]);
    app.run(crate::command::Command::UseDeckDir(recent));
    super::press(&app, crate::command::WindowAction::OpenRecent);
    assert!(notice(&app).starts_with("The deck you were editing was saved to "), "{}", notice(&app));
    let for_agent = described_as(&app, "tok-ypresent").state["notice"].as_str().unwrap().to_string();
    assert!(!for_agent.contains("talk.ydeck") && !for_agent.contains(&dir.display().to_string()), "{for_agent}");
}
