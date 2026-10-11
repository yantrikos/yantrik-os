//! The folder open beside the tabs (#731), driven through the real window and the published
//! actions: the 520 check, as far as a test without the harnesses folder can take it.
use super::*;
use slint::Model;

/// Called from the window test, since only one Slint platform may exist per process.
pub(super) fn a_folder_opens_beside_the_tabs(
    ui: &TextEditorApp,
    s: &State,
    queue: &Queue,
    window: &MinimalSoftwareWindow,
    published: &[(Action, Handler)],
    dir: &Path,
) {
    let root = dir.join("harnesses");
    for d in ["deepseek", "hermes", "lib", "openclaw", "pi", "tests", ".git", "target"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    std::fs::write(root.join("hermes/README.md"), "# Hermes\n").unwrap();
    std::fs::write(root.join("hermes/adapter.py"), "def adapt():\n    pass\n").unwrap();

    // Something that is not a folder is refused with a sentence, and nothing opens.
    let refused = act_on(published, "open_folder", serde_json::json!({ "path": root.join("hermes/README.md") }))
        .expect_err("a file is not a folder");
    assert!(refused.contains("is not a folder"), "{refused}");
    assert!(!ui.get_folder_open());

    // 1. The tree shows the folders, the noise left out.
    let answer = act_on(published, "open_folder", serde_json::json!({ "path": root })).unwrap();
    assert_eq!(answer["rows"], serde_json::json!(["deepseek/", "hermes/", "lib/", "openclaw/", "pi/", "tests/"]));
    assert_eq!(answer["truncated"], false);
    assert!(ui.get_folder_open() && ui.get_show_tree());
    assert_eq!(ui.get_project_name(), "harnesses");

    // 2. Expand hermes/ and open README.md: it opens in a tab and is highlighted.
    let index = |name: &str| ui.get_tree_rows().iter().position(|r| r.name == name).unwrap() as i32;
    ui.invoke_tree_toggled(index("hermes"));
    assert!(ui.get_tree_rows().iter().any(|r| r.name == "README.md" && r.depth == 1));
    ui.invoke_tree_activated(index("README.md"));
    wait(queue, window, || !ui.get_busy());
    let readme = root.join("hermes/README.md");
    let tabs = s.borrow().docs.len();
    assert_eq!(s.borrow().docs[s.borrow().active].path.as_ref(), Some(&readme));
    let current = ui.get_tree_current();
    let row = ui.get_tree_rows().row_data(current as usize).unwrap();
    assert!(row.selected && row.name == "README.md", "{row:?}");
    // Clicked again, the open tab comes forward; the file is not opened twice.
    ui.invoke_action("new".into());
    ui.invoke_tree_activated(index("README.md"));
    wait(queue, window, || !ui.get_busy());
    assert_eq!(s.borrow().docs.len(), tabs + 1);
    assert_eq!(s.borrow().docs[s.borrow().active].path.as_ref(), Some(&readme));
    tick(queue, window);
    save(window, "editor-project.png");

    // 3. Filter `adapter`: hermes/adapter.py is listed.
    ui.invoke_tree_filtered("adapter".into());
    let names: Vec<_> = ui.get_tree_rows().iter().map(|r| r.name.to_string()).collect();
    assert_eq!(names, ["hermes/adapter.py"]);
    ui.invoke_tree_filtered("".into());

    // 4. `describe` shows folder.rows; the expanded folder is kept with the drafts.
    let folder = folder::describe(&s.borrow());
    assert!(folder["rows"].as_array().unwrap().contains(&serde_json::json!("hermes/README.md")), "{folder}");
    let saved = folder::saved(&s.borrow()).unwrap();
    assert!(saved.root == root && saved.expanded.contains(&root.join("hermes")));

    // Ctrl+B hides the panel without closing the folder; `close_folder` closes it.
    window.dispatch_event(WindowEvent::KeyPressed { text: slint::platform::Key::Control.into() });
    key(window, "b");
    window.dispatch_event(WindowEvent::KeyReleased { text: slint::platform::Key::Control.into() });
    assert!(!ui.get_show_tree() && ui.get_folder_open());
    ui.set_show_tree(true);
    act_on(published, "close_folder", serde_json::json!({})).unwrap();
    assert!(!ui.get_folder_open() && folder::describe(&s.borrow()).is_null());
    assert!(act_on(published, "close_folder", serde_json::json!({})).is_err());
}

#[test]
fn the_open_folder_is_kept_with_the_drafts_and_an_older_file_still_reads() {
    let dir = fixture("folder-recovery");
    let path = dir.join("drafts.json");
    // A file written before #731: the drafts alone.
    std::fs::write(&path, "[]").unwrap();
    assert_eq!(document::recover_all(&path).unwrap().1, None);
    let saved = project::Saved { root: dir.clone(), expanded: vec![dir.join("a")] };
    document::checkpoint_with(&path, &[], Some(&saved)).unwrap();
    let (docs, folder) = document::recover_all(&path).unwrap();
    assert!(docs.is_empty());
    assert_eq!(folder, Some(saved));
    assert!(document::recover(&path).unwrap().is_empty());
}
