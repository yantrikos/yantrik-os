//! An agent names only files in the person's home, outside its protected places (#443), never
//! reaches one through a link that leads out of it, and is not shown a sheet whose file it may not
//! read (the security reviews of #727).
//!
//! The scratch folders are in the temporary directory, outside the home, which is exactly what an
//! agent is refused and the person is not.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use yantrik_app::control::{AgentTokenScope, View};
use yantrik_testing::TestApp;

use super::{act, act_as_agent, scratch, settle, started};
use crate::agent_rule::HIDDEN_REFUSAL;
use crate::files::open::open_sheet_in;
use crate::YSheets;

const SENTINEL: &str = "topsecret-ysheets-7f3a";

fn outside_home(dir: &Path) -> bool {
    !dir.starts_with(PathBuf::from(std::env::var_os("HOME").unwrap_or_default()))
}

/// `describe` as an agent with this token reads it.
fn described_as(app: &TestApp<YSheets>, token: &str) -> View {
    let token = token.to_string();
    app.model(move |m| {
        let _agent = AgentTokenScope::enter(Some(token));
        crate::describe::view(m)
    })
}

/// The person opens a file outside the home, which no agent may read.
fn person_opens_a_secret(name: &str) -> (TestApp<YSheets>, PathBuf) {
    let app = started();
    let dir = scratch(name);
    let secret = dir.join("secret.csv");
    std::fs::write(&secret, format!("{SENTINEL},42\n")).unwrap();
    act(&app, "open", json!({ "path": secret.display().to_string() })).unwrap();
    (app, dir)
}

#[test]
fn an_agent_reads_nothing_of_a_hidden_sheet_through_any_action() {
    let (app, dir) = person_opens_a_secret("hidden");
    if !outside_home(&dir) {
        return;
    }
    let copy = dir.join("x.csv");
    for (name, args) in [
        ("select", json!({ "cell": "A1" })),
        ("set_cell", json!({ "cell": "C1", "text": "=A1&B1" })),
        ("set_cell", json!({ "cell": "C1", "text": format!("=VLOOKUP(\"{SENTINEL}\",A1:B1,2)") })),
        ("undo", json!({})),
        ("redo", json!({})),
        ("get_range", json!({ "range": "A1:C1" })),
        ("save", json!({ "path": "~/Documents/x.csv" })),
        ("save", json!({ "path": copy.display().to_string() })),
        ("save_over", json!({ "path": copy.display().to_string() })),
        ("save_over", json!({})),
    ] {
        let e = act_as_agent(&app, "tok-sheets-review", name, args).unwrap_err();
        assert!(!e.contains(SENTINEL), "{name}: {e}");
        assert_eq!(e, HIDDEN_REFUSAL, "{name}");
        assert!(e.contains("not read or saved for an agent"));
    }
    assert!(!copy.exists(), "nothing of the hidden sheet was written");
    assert_eq!(super::raw(&app, "C1"), "", "set_cell did not run");

    let view = serde_json::to_string(&app.describe()).unwrap();
    assert!(view.contains(SENTINEL), "the person's own describe shows it");
    let seen = serde_json::to_string(&described_as(&app, "tok-sheets-review").state).unwrap();
    assert!(!seen.contains(SENTINEL), "{seen}");
    // The person still sees and edits their own sheet.
    let got = act(&app, "select", json!({ "cell": "A1" })).unwrap();
    assert_eq!(got["typed"], SENTINEL);
}

#[test]
fn a_hidden_sheet_quoted_by_the_status_line_or_the_notice_is_not_read_out() {
    let (app, dir) = person_opens_a_secret("hidden-status");
    if !outside_home(&dir) {
        return;
    }
    // The person's own formula quotes the cell's text in its error, on the status line.
    act(&app, "set_cell", json!({ "cell": "C1", "text": "=A1+1" })).unwrap();
    assert!(super::status(&app).contains(SENTINEL));
    assert!(super::notice(&app).contains(SENTINEL));
    let seen = described_as(&app, "tok-sheets-review");
    let text = format!("{} {}", seen.summary, seen.state);
    assert!(!text.contains(SENTINEL), "{text}");
    assert_eq!(seen.state["notice"], "");
    assert_eq!(seen.state["status"], "");
}

/// The security delta review's INFO: `describe` still gave a hidden sheet's path, used range,
/// counts, first error and selected cell. Now it gives its name and nothing of its shape.
#[test]
fn describe_gives_an_agent_a_hidden_sheets_name_and_nothing_of_its_shape() {
    let (app, dir) = person_opens_a_secret("hidden-describe");
    if !outside_home(&dir) {
        return;
    }
    act(&app, "set_cell", json!({ "cell": "D3", "text": "=1/0" })).unwrap();
    act(&app, "select", json!({ "cell": "C2" })).unwrap();
    let seen = described_as(&app, "tok-sheets-review");
    let st = &seen.state;
    assert_eq!(st["sheet"], "secret.csv");
    for key in ["path", "rows", "cols", "used", "selection", "can_undo", "can_redo", "filled_cells", "error_cells", "first_error"] {
        assert!(st[key].is_null(), "{key}: {}", st[key]);
    }
    assert!(st["hidden"].is_string(), "the agent is told why");
    assert_eq!(st.as_object().unwrap().len(), 16, "the same sixteen keys");
    let text = format!("{} {st}", seen.summary);
    for shape in ["A1:D3", "D3", "C2", SENTINEL, &dir.display().to_string()] {
        assert!(!text.contains(shape), "{shape} in {text}");
    }
    assert_eq!(seen.summary, "ySheets — secret.csv · not read for an agent · unsaved changes");
}

#[test]
fn an_agent_names_only_files_in_the_home_and_a_save_goes_only_where_it_may_write() {
    let app = started();
    let dir = scratch("agent-write");
    if !outside_home(&dir) {
        return;
    }
    let target = dir.join("agent.csv");
    let refused = |name: &str, args: Value, why: &str| {
        let err = act_as_agent(&app, "tok-sheets", name, args.clone()).expect_err(&format!("{name} {args}"));
        assert!(err.ends_with(why), "{name} {args}: {err}");
    };
    refused("open", json!({ "path": "~/.ssh/id_ed25519" }), " is protected");
    for own in [
        "~/.local/share/yantrik/snippets/snippets.json",
        "~/.local/state/yantrik/notes/drafts/note-today.json",
        "~/.local/share/yantrik-mind/yantrik-memory.token",
    ] {
        refused("open", json!({ "path": own }), " is protected");
    }
    refused("open", json!({ "path": "/etc/hostname" }), " is outside");
    refused("open", json!({ "path": target.display().to_string() }), " is outside");
    // Typed by an agent, in a sheet of its own: the paths it names are what is refused below.
    act_as_agent(&app, "tok-sheets", "set_cell", json!({ "cell": "A1", "text": "echo pwned" })).unwrap();
    for name in ["save", "save_over"] {
        refused(name, json!({ "path": "~/.bashrc" }), " is protected");
        refused(name, json!({ "path": "~/.config/autostart/x.desktop.csv" }), " is protected");
        refused(name, json!({ "path": target.display().to_string() }), " is outside");
    }
    assert!(!target.exists(), "nothing was written");
    // The person writes where they choose.
    act(&app, "save", json!({ "path": target.display().to_string() })).unwrap();
    assert_eq!(super::file(&target), "echo pwned\r\n");
}

#[test]
fn an_import_never_follows_a_link_out_of_the_home() {
    // The home is a folder of this test's own, passed in to the rule that decides.
    let root = scratch("agent-link");
    let home = root.join("home");
    std::fs::create_dir_all(home.join("Documents")).unwrap();
    let outside = root.join("outside.csv");
    std::fs::write(&outside, format!("{SENTINEL}\n")).unwrap();
    let inside = home.join("Documents/real.csv");
    std::fs::write(&inside, "fine\n").unwrap();
    let link = home.join("Documents/link.csv");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let folder_link = home.join("escape");
    std::os::unix::fs::symlink(&root, &folder_link).unwrap();

    assert!(open_sheet_in(&home, &inside, true).is_ok(), "a file in the home is read");
    for path in [link.clone(), folder_link.join("outside.csv")] {
        let e = open_sheet_in(&home, &path, true).expect_err(&path.display().to_string());
        assert!(!e.contains(SENTINEL) && e.ends_with(" is outside"), "{e}");
    }
    // The person is not asked: a link out of the home is theirs to follow.
    let theirs = open_sheet_in(&home, &link, false).expect("the person may");
    assert_eq!(theirs.sheet.raw(super::cell("A1")), SENTINEL);
}

#[test]
fn a_path_with_spaces_round_the_name_opens_what_was_checked() {
    let app = started();
    let dir = scratch("trimmed");
    let sheet = dir.join("sheet.csv");
    std::fs::write(&sheet, "one\n").unwrap();
    act(&app, "open", json!({ "path": format!("  {}  ", sheet.display()) })).expect("the trimmed name is the one opened");
    assert_eq!(super::path_of(&app), Some(sheet.clone()));
    let copy = dir.join("copy.csv");
    act(&app, "save", json!({ "path": format!("{} ", copy.display()) })).unwrap();
    assert!(copy.exists() && !dir.join("copy.csv ").exists(), "written where it was checked");
    settle(&app);
}

#[test]
fn a_path_the_rule_refuses_is_said_before_unsaved_changes_as_it_always_was() {
    let app = started();
    let dir = scratch("order");
    if !outside_home(&dir) {
        return;
    }
    let other = dir.join("other.csv");
    std::fs::write(&other, "1\r\n").unwrap();
    act(&app, "set_cell", json!({ "cell": "A1", "text": "mine" })).unwrap();
    // An agent naming a file it may not read is told so, whether or not there is anything unsaved.
    let e = act_as_agent(&app, "tok-sheets", "open", json!({ "path": "~/.ssh/id_ed25519" })).unwrap_err();
    assert!(e.ends_with(" is protected"), "{e}");
    let e = act_as_agent(&app, "tok-sheets", "open", json!({ "path": other.display().to_string() })).unwrap_err();
    assert!(e.ends_with(" is outside"), "{e}");
    // The person, for whom the path is fine, is told about the unsaved changes.
    let e = act(&app, "open", json!({ "path": other.display().to_string() })).unwrap_err();
    assert!(e.contains("has unsaved changes, so nothing was opened"), "{e}");
    assert_eq!(super::raw(&app, "A1"), "mine");
}
