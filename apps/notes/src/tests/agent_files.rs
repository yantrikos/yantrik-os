//! An agent's `import` and `export` name only a file in the person's home, outside its protected
//! places (`home_paths`, #443): read in as a note, any file the person can read was `describe`'s
//! first 4000 characters, the snippets store and every Notes draft included (security review of
//! #942, finding 1). The person names any file, as before.

use serde_json::json;
use yantrik_testing::Call;

use super::{note, settle, started, state, store::fixture};

const SENTINEL: &str = "SENTINEL-notes-agent-5c1e";
const RULE: &str = "an agent names only a file in the person's home, outside its protected places: ";

fn as_agent(app: &yantrik_testing::TestApp<crate::Notes>, name: &str, args: serde_json::Value) -> Result<serde_json::Value, String> {
    let pending = app.act_as(Call::new(name, args).as_agent("mind"))?;
    settle(app);
    pending.settle()
}

#[test]
fn an_agent_imports_and_exports_only_files_in_the_home() {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let dir = fixture("agent-files");
    assert!(!dir.starts_with(&home), "the test's folder is outside the home");
    let outside = dir.join("credentials.md");
    std::fs::write(&outside, format!("# Keys\n\n{SENTINEL}\n")).unwrap();
    let app = started();
    note(&app, "Plan", "the open note");

    for asked in [
        outside.display().to_string(),
        "~/.ssh/id_notes_probe".to_string(),
        "~/.local/share/yantrik/snippets/snippets.json".to_string(),
        "~/.local/state/yantrik/notes/drafts/note-probe.json".to_string(),
    ] {
        let e = as_agent(&app, "import", json!({ "path": asked })).unwrap_err();
        assert!(e.starts_with(RULE), "{asked}: {e}");
    }
    let shown = app.describe().to_string();
    assert!(!shown.contains(SENTINEL), "nothing was read in: {shown}");

    for asked in [dir.join("out.md").display().to_string(), "~/.bashrc-notes-probe".to_string(), "~/.config/autostart/notes-probe.desktop".to_string()] {
        let e = as_agent(&app, "export", json!({ "path": asked })).unwrap_err();
        assert!(e.starts_with(RULE), "{asked}: {e}");
    }
    assert!(!dir.join("out.md").exists(), "nothing was written");
    assert!(!home.join(".bashrc-notes-probe").exists() && !home.join(".config/autostart/notes-probe.desktop").exists());

    // The person names any file they can read or write, as before.
    let pending = app.act("import", json!({ "path": outside.display().to_string() })).unwrap();
    settle(&app);
    pending.settle().expect("the person's import");
    assert!(state(&app).to_string().contains("Keys"), "the person's import is in the library");
    let pending = app.act("export", json!({ "path": dir.join("mine.md").display().to_string() })).unwrap();
    settle(&app);
    pending.settle().expect("the person's export");
    assert!(dir.join("mine.md").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
