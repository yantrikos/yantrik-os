//! An agent names only files in the person's home, outside its protected places (#443, #942), and
//! is not shown a document whose file it may not read.
//!
//! The scratch folders are in the temporary directory, outside the home, which is exactly what an
//! agent is refused and the person is not.

use std::path::PathBuf;

use serde_json::{json, Value};
use yantrik_app::control::{AgentTokenScope, View};
use yantrik_testing::TestApp;

use super::{act, act_as_agent, file, none, opened, scratch, started, state, text};
use crate::agent_rule::{HIDDEN, HIDDEN_REFUSAL};
use crate::YDoc;

const SENTINEL: &str = "SENTINEL-7f3a-do-not-show";

/// `describe` as an agent with this token reads it.
fn described_as(app: &TestApp<YDoc>, token: &str) -> View {
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
    let secret = dir.join("secret.md");
    opened(&app, &secret, &format!("# omega\n\n{SENTINEL}\n"));
    assert!(state(&app)["content"].as_str().unwrap().contains(SENTINEL), "the person sees the text");
    act(&app, "find", json!({ "query": "omega" })).unwrap();

    let refused = |name: &str, args: Value, why: &str| {
        let err = act_as_agent(&app, "tok-ydoc", name, args.clone()).expect_err(&format!("{name} {args}"));
        assert!(err.ends_with(why), "{name} {args}: {err}");
        assert!(!err.contains(SENTINEL), "{err}");
    };
    refused("open", json!({ "path": "~/.ssh/id_ed25519" }), " is protected");
    // Yantrik's own data and state, each through its own door.
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
    refused("open", json!({ "path": secret.display().to_string() }), " is outside");
    refused("save", none(), " is outside");
    // The refusal of a save of the document it is not shown names neither the file nor its folder.
    let err = act_as_agent(&app, "tok-ydoc", "save", none()).unwrap_err();
    assert!(!err.contains("secret.md") && !err.contains(&dir.display().to_string()), "{err}");
    for (name, args) in [
        ("find", json!({ "query": "omega" })),
        ("replace_all", json!({ "find": "omega", "with": "x" })),
        ("set_content", json!({ "text": "x" })),
        ("append", json!({ "text": "x" })),
        ("save_as", json!({ "path": "~/Documents/copied-out.md" })),
        ("export_markdown", json!({ "path": "~/Documents/copied-out.md" })),
    ] {
        refused(name, args, HIDDEN_REFUSAL);
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    assert!(!home.join("Documents/copied-out.md").exists(), "nothing was copied out");
    // The person is not asked: their own open of Yantrik's own places is as it was.
    for own in ["~/.local/share/yantrik/snippets/snippets.json", "~/.local/state/yantrik/email/drafts/composing.json"] {
        let path = yantrik_ipc_contracts::home_paths::expand(own, &home).expect("a path");
        assert_eq!(crate::agent_rule::may_read(false, &path), Ok(()), "{own}");
    }
    assert!(text(&app).contains(SENTINEL), "the document is untouched");
    assert_eq!(file(&secret), format!("# omega\n\n{SENTINEL}\n"));

    // Nor is it read back through `describe`: not its text, its name, its path, its outline or a
    // match count.
    let seen = described_as(&app, "tok-ydoc");
    let s = &seen.state;
    assert_eq!(s["content"], "", "{s}");
    assert!(s["content_hidden"].is_string(), "{s}");
    assert_eq!((s["find_matches"].clone(), s["find_query"].clone()), (json!(0), json!("")), "{s}");
    assert_eq!((s["title"].clone(), s["path"].clone()), (json!(HIDDEN), json!(HIDDEN)));
    assert_eq!((s["words"].clone(), s["chars"].clone(), s["outline"].clone()), (json!(0), json!(0), json!([])));
    let listed = s.to_string();
    assert!(!listed.contains("secret.md") && !listed.contains(&dir.display().to_string()), "{listed}");
    assert!(!listed.contains(SENTINEL) && !seen.summary.contains(SENTINEL) && !seen.summary.contains("omega"));
    assert!(seen.summary.contains(HIDDEN), "{}", seen.summary);
}

#[test]
fn an_agents_own_document_is_written_only_where_the_write_rule_allows() {
    let app = started();
    let dir = scratch("agent-write");
    if !outside_home(&dir) {
        return;
    }
    act_as_agent(&app, "tok-ydoc", "set_content", json!({ "text": "echo pwned\n" })).unwrap();
    let refused = |name: &str, path: &str, why: &str| {
        // `export_markdown` takes no `overwrite`: a copy never replaces a file.
        let args = if name == "save_as" { json!({ "path": path, "overwrite": true }) } else { json!({ "path": path }) };
        let err = act_as_agent(&app, "tok-ydoc", name, args).expect_err(path);
        assert!(err.ends_with(why), "{name} {path}: {err}");
    };
    for name in ["save_as", "export_markdown"] {
        refused(name, "~/.bashrc", " is protected");
        refused(name, "~/.config/autostart/x.desktop", " is protected");
        refused(name, &dir.join("agent.md").display().to_string(), " is outside");
    }
    assert!(!dir.join("agent.md").exists(), "nothing was written");
    // The person writes where they choose.
    let path = dir.join("person.md");
    act(&app, "save_as", json!({ "path": path.display().to_string() })).unwrap();
    assert_eq!(file(&path), "echo pwned\n");
}

#[test]
fn what_an_agent_reads_in_the_notice_about_a_hidden_document_names_no_file() {
    let app = started();
    let dir = scratch("agent-notice");
    if !outside_home(&dir) {
        return;
    }
    let secret = dir.join("private-plan.md");
    opened(&app, &secret, &format!("# Plan\n{SENTINEL}\n"));
    super::typed(&app, &format!("# Plan\n{SENTINEL}\nand an unsaved line\n"));
    // An open over the unsaved text is refused in words about the document in front.
    let other = dir.join("other.md");
    std::fs::write(&other, "x\n").unwrap();
    let err = act_as_agent(&app, "tok-ydoc", "open", json!({ "path": "~/Documents/x.md" })).unwrap_err();
    assert!(!err.contains("private-plan") && !err.contains(SENTINEL), "{err}");
    let seen = described_as(&app, "tok-ydoc");
    let notice = seen.state["notice"].as_str().unwrap().to_string();
    assert!(!notice.contains("private-plan") && !notice.contains("Plan"), "{notice}");
    // The person's own read of the same notice may name the file.
    let person = app.model(|m| m.notice.window().to_string());
    assert!(!person.contains(SENTINEL), "{person}");
}

#[test]
fn a_path_with_spaces_round_the_name_opens_and_writes_what_was_checked() {
    let app = started();
    let dir = scratch("trimmed");
    let note = dir.join("note.md");
    std::fs::write(&note, "# Note\n").unwrap();
    act(&app, "open", json!({ "path": format!("  {}  ", note.display()) })).expect("the trimmed name is the one opened");
    assert_eq!(super::path_of(&app), Some(note.clone()));
    let copy = dir.join("copy.md");
    act(&app, "export_markdown", json!({ "path": format!("{} ", copy.display()) })).unwrap();
    assert!(copy.exists() && !dir.join("copy.md ").exists(), "written where it was checked");
}
