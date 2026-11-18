//! An agent names only files in the person's home, outside its protected places (#443), is not
//! shown a tab whose file it may not read, and is told whose tabs are whose (VM 520, 4 October).
//!
//! The scratch folders are in the temporary directory, outside the home, which is exactly what an
//! agent is refused and the person is not.

use std::path::PathBuf;

use serde_json::{json, Value};
use yantrik_app::control::{AgentTokenScope, View};
use yantrik_testing::TestApp;

use super::{act, act_as_agent, file, none, opened, scratch, started, state, text};
use crate::TextEditor;

const SENTINEL: &str = "SENTINEL-7f3a-do-not-show";

/// `describe` as an agent with this token reads it.
fn described_as(app: &TestApp<TextEditor>, token: &str) -> View {
    let token = token.to_string();
    app.model(move |m| {
        let _agent = AgentTokenScope::enter(Some(token));
        crate::describe::view(m)
    })
}

fn outside_home(dir: &std::path::Path) -> bool {
    !dir.starts_with(PathBuf::from(std::env::var_os("HOME").unwrap_or_default()))
}

#[test]
fn an_agent_names_only_files_in_the_home_and_is_not_shown_the_persons_other_files() {
    let app = started();
    let dir = scratch("agent-home");
    if !outside_home(&dir) {
        return; // A temp folder inside the home would be allowed, and prove nothing here.
    }
    let tab = dir.join("secret.txt");
    opened(&app, &tab, &format!("omega {SENTINEL}\n"));
    assert!(state(&app)["content"].as_str().unwrap().contains(SENTINEL), "the person sees the text");
    act(&app, "find", json!({ "text": "omega" })).unwrap();

    let refused = |name: &str, args: Value, why: &str| {
        let err = act_as_agent(&app, "tok-editor", name, args.clone()).expect_err(&format!("{name} {args}"));
        assert!(err.ends_with(why), "{name} {args}: {err}");
        assert!(!err.contains(SENTINEL), "{err}");
    };
    refused("open", json!({ "path": "~/.ssh/id_ed25519" }), " is protected");
    // Yantrik's own data and state: the snippets store, an Email draft, a Notes draft (#940
    // review, High 1). Each app's data goes through its own door.
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
    refused("open", json!({ "path": tab.display().to_string() }), " is outside");
    refused("save", none(), " is outside");
    // The refusal of a save of the tab it is not shown names neither the file nor its folder.
    let err = act_as_agent(&app, "tok-editor", "save", none()).unwrap_err();
    assert!(!err.contains("secret.txt") && !err.contains(&dir.display().to_string()), "{err}");
    let hidden = "its text is left alone; `select_tab` another or `new` one.";
    refused("save_as", json!({ "path": "~/Documents/copied-out.txt" }), hidden);
    // `read` takes no path at all: the dispatch refuses one before the editor is asked.
    let err = act_as_agent(&app, "tok-editor", "read", json!({ "path": "~/.ssh/id_ed25519" })).unwrap_err();
    assert!(err.contains("has no argument `path`"), "{err}");
    let active = app.model(|m| m.active);
    refused("read", json!({ "tab": active }), "its text is left alone; read another tab or `new` one.");
    for (name, args) in [
        ("read", none()),
        ("find", json!({ "text": "omega" })),
        ("find-next", none()),
        ("find-prev", none()),
        ("replace_text", json!({ "text": "x" })),
        ("replace", none()),
        ("replace-all", none()),
        ("set_content", json!({ "text": "x" })),
        ("append", json!({ "text": "x" })),
        ("undo", none()),
        ("redo", none()),
    ] {
        refused(name, args, hidden);
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    assert!(!home.join("Documents/copied-out.txt").exists(), "nothing was copied out");
    // The person is not asked: their own open of Yantrik's own places is as it was.
    for own in ["~/.local/share/yantrik/snippets/snippets.json", "~/.local/state/yantrik/email/drafts/composing.json"] {
        let path = yantrik_ipc_contracts::home_paths::expand(own, &home).expect("a path");
        assert_eq!(crate::agent_rule::may_read(false, &path), Ok(()), "{own}");
    }
    assert!(text(&app).contains(SENTINEL), "the tab is untouched");
    assert_eq!(file(&tab), format!("omega {SENTINEL}\n"));

    // Nor is it read back through `describe`: not its text, its name, its path or a match count.
    let seen = described_as(&app, "tok-editor");
    let s = &seen.state;
    assert_eq!(s["content"], "", "{s}");
    assert!(s["content_hidden"].is_string(), "{s}");
    assert_eq!((s["find_count"].clone(), s["find_query"].clone()), (json!(0), json!("")), "{s}");
    for size in ["lines", "characters", "bytes"] {
        assert!(s[size].is_null(), "{size}: {s}");
    }
    assert!(seen.summary.contains("not shown to an agent") && !seen.summary.contains(" line"), "{}", seen.summary);
    let listed = s.to_string();
    assert!(!listed.contains("secret.txt") && !listed.contains(&dir.display().to_string()), "{listed}");
    assert!(!listed.contains(SENTINEL) && !seen.summary.contains(SENTINEL));

    // In a tab of its own, the write rule alone decides.
    act_as_agent(&app, "tok-editor", "new", json!({ "text": "echo pwned\n" })).unwrap();
    refused("save_as", json!({ "path": "~/.bashrc", "overwrite": true }), " is protected");
    refused("save_as", json!({ "path": "~/.config/autostart/x.desktop" }), " is protected");
    for place in ["~/.gitconfig", "~/.local/bin/x", "~/bin/x", "~/.tmux.conf"] {
        refused("save_as", json!({ "path": place }), " is hidden_place");
    }
    refused("save_as", json!({ "path": dir.join("agent.txt").display().to_string() }), " is outside");
    assert!(!dir.join("agent.txt").exists(), "nothing was written");
    // The notice every later `describe` reads names no hidden file either.
    assert!(!super::notice(&app).contains("secret.txt"));
}

#[test]
fn tabs_know_who_opened_them() {
    let app = started();
    let tabs = |token: Option<&str>| match token {
        Some(t) => described_as(&app, t).state["tabs"].clone(),
        None => state(&app)["tabs"].clone(),
    };
    act(&app, "new", none()).unwrap();
    let seen = tabs(None);
    assert_eq!((seen[1]["opened_by"].clone(), seen[1]["opened_by_you"].clone()), (json!("person"), json!(true)));

    let answer = act_as_agent(&app, "tok-owner-a", "new", json!({ "text": "the mind's notes\n" })).unwrap();
    assert_eq!((answer["opened_by"].clone(), answer["opened_by_you"].clone()), (json!("agent"), json!(true)));
    let seen = tabs(Some("tok-owner-a"));
    assert_eq!(seen[2]["opened_by_you"], true, "{seen}");
    assert_eq!(seen[1]["opened_by_you"], false, "the person's tab is not the agent's: {seen}");
    assert_eq!(tabs(Some("tok-owner-b"))[2]["opened_by_you"], false, "nor another agent's");
    assert_eq!(tabs(None)[2]["opened_by_you"], false, "nor the person's");
}

#[test]
fn an_agent_does_not_close_the_persons_unsaved_tab_or_what_undo_set_aside() {
    let app = started();
    super::typed(&app, "the person's next line\n");
    let err = act_as_agent(&app, "tok-a", "close", none()).unwrap_err();
    assert!(err.contains("opened by the person and has changes that are not saved"), "{err}");
    assert_eq!(state(&app)["dialog"], 0, "no question was put over the person's work");
    act_as_agent(&app, "tok-a", "undo", none()).expect("undo to the saved text");
    assert!(!super::modified(&app), "matches its file again");
    let err = act_as_agent(&app, "tok-a", "close", none()).unwrap_err();
    assert!(err.contains("that `undo` set aside"), "{err}");
    act(&app, "redo", none()).unwrap();
    assert_eq!(text(&app), "the person's next line\n");
}

#[test]
fn the_budget_holds_for_every_edit_and_the_refusal_says_whose_tab_to_close() {
    let app = started();
    let full = "z".repeat(crate::text::limits::MAX_BYTES);
    // The person's 63 unsaved tabs of 1 MiB, beside the empty one: one MiB of room is left.
    for _ in 0..63 {
        act(&app, "new", json!({ "text": full })).unwrap();
    }
    let err = act_as_agent(&app, "tok-a", "new", json!({ "text": format!("{full}z") })).unwrap_err();
    assert!(err.contains("Editing is limited to 1 MiB"), "{err}");
    act_as_agent(&app, "tok-a", "new", json!({ "text": full })).expect("the last MiB");
    let err = act_as_agent(&app, "tok-a", "new", json!({ "text": "one more" })).unwrap_err();
    assert!(err.contains("would hold more than the 64 MiB"), "{err}");
    assert!(err.contains("Every tab you opened has unsaved changes"), "{err}");
    assert_eq!(super::notice(&app), err, "said in the window too");
    let err = act_as_agent(&app, "tok-b", "new", json!({ "text": "one more" })).unwrap_err();
    // The person's empty first tab is the one that loses nothing.
    assert!(err.contains("None of the open tabs were opened by you; the person can close one (oldest empty: 0 Untitled)"), "{err}");
    // Edits are held to it too: a tab may not grow past the edge.
    act(&app, "select_tab", json!({ "index": 0 })).unwrap();
    let err = act(&app, "append", json!({ "text": "y" })).unwrap_err();
    assert!(err.contains("past the 64 MiB of text") && err.contains("the tab is unchanged"), "{err}");
    assert_eq!(text(&app), "");
}

#[test]
fn close_names_a_hidden_tab_as_hidden_and_save_as_keeps_a_tab_the_agents() {
    let app = started();
    let dir = scratch("agent-close");
    if !outside_home(&dir) {
        return;
    }
    opened(&app, &dir.join("hidden.txt"), "x\n");
    let closed = act_as_agent(&app, "tok-a", "close", none()).unwrap();
    assert_eq!(closed["was"], "(hidden)", "{closed}");
    assert!(!closed.to_string().contains("hidden.txt"), "{closed}");
}
