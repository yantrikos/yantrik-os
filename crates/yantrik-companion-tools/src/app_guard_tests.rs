//! `app_action` runs an action only if the app grades it `safe` or it is on the reviewed list, keyed on the
//! id the socket is bound under (#1313, round 6).

use super::*;
use crate::fake_shell::{Answer, FakeShell, First};
use serde_json::json;
use yantrik_shell_core::apps::parse_desktop_text;

/// The shipped `.desktop` files of the apps whose id is not their folder's name, parsed as the shell
/// parses them: the table `X-Yantrik-Surface` and `X-Yantrik-Aliases` give.
fn shipped() -> Vec<DesktopEntry> {
    [
        ("yantrik-text-editor", include_str!("../../../apps/desktop-files/yantrik-text-editor.desktop")),
        ("yantrik-document-editor", include_str!("../../../apps/desktop-files/yantrik-document-editor.desktop")),
        ("yantrik-snippet-manager", include_str!("../../../apps/desktop-files/yantrik-snippet-manager.desktop")),
        ("yantrik-container-manager", include_str!("../../../apps/desktop-files/yantrik-container-manager.desktop")),
        ("yantrik-music-player", include_str!("../../../apps/desktop-files/yantrik-music-player.desktop")),
        ("yantrik-calendar", include_str!("../../../apps/desktop-files/yantrik-calendar.desktop")),
    ]
    .iter()
    .map(|(stem, text)| parse_desktop_text(stem, text).unwrap_or_else(|| panic!("{stem} does not parse")))
    .collect()
}

fn nowhere() -> std::path::PathBuf {
    std::env::temp_dir().join("yantrik-app-guard-no-such-directory")
}

/// A view in which the app grades `action` as `grade`.
fn graded(action: &str, grade: &str) -> Value {
    json!({ "actions": [{ "name": action, "permission": grade, "description": "x" }] })
}

fn admit_here(dir: &std::path::Path, app: &str, action: &str, view: Option<&Value>) -> Result<(), String> {
    admit_in(dir, &shipped, app, action, view)
}

#[test]
fn the_table_gives_an_id_and_an_alias_the_same_answer() {
    let entries = shipped();
    for (name, id) in [
        ("editor", "editor"),
        ("text-editor", "editor"),
        ("Text Editor", "editor"),
        ("documents", "documents"),
        ("document-editor", "documents"),
        ("snippets", "snippets"),
        ("snippet_manager", "snippets"),
        ("containers", "containers"),
        ("container-manager", "containers"),
        ("music-player", "music"),
        ("calendar", "calendar"),
        ("something-nobody-declared", "something-nobody-declared"),
    ] {
        assert_eq!(table_id(&entries, name), id, "{name}");
    }
}

/// A real socket in a directory, one per id, and each alias a symlink to it, the way the shell's
/// `link_aliases_in` lays them out: a call by the id, by the alias, or by either before the app is
/// started, meets the same list.
#[cfg(unix)]
#[test]
fn the_text_editor_the_document_editor_and_the_snippet_manager_are_read_by_their_ids() {
    let dir = std::env::temp_dir().join(format!("yantrik-app-guard-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut held = Vec::new();
    for (id, alias, action) in [("editor", "text-editor", "save"), ("documents", "document-editor", "export_markdown"), ("snippets", "snippet-manager", "delete")] {
        held.push(std::os::unix::net::UnixListener::bind(dir.join(format!("app-{id}.sock"))).expect("a real socket"));
        std::os::unix::fs::symlink(format!("app-{id}.sock"), dir.join(format!("app-{alias}.sock"))).unwrap();
        assert_eq!(bound_id_in(&dir, id).as_deref(), Some(id));
        assert_eq!(bound_id_in(&dir, alias).as_deref(), Some(id), "{alias} is a link to {id}");
        for name in [id, alias] {
            let said = admit_here(&dir, name, action, Some(&graded(action, "standard"))).expect_err(name);
            assert!(said.starts_with("Refused:") && said.contains(&format!("`{id}.{action}`")) && said.contains("Nothing was changed"), "{name}: {said}");
            // The app's own grade `safe` is the other way in.
            assert_eq!(admit_here(&dir, name, action, Some(&graded(action, "safe"))), Ok(()), "{name}");
        }
    }
    // The same calls before the app is started: no socket, so the table answers.
    let before = nowhere();
    for (name, action) in [("editor", "save"), ("text-editor", "save"), ("documents", "export_markdown"), ("document-editor", "save"), ("snippets", "delete"), ("snippet-manager", "delete")] {
        assert!(admit_here(&before, name, action, Some(&graded(action, "standard"))).is_err(), "{name}.{action}");
        assert!(admit_here(&before, name, action, None).is_err(), "{name}.{action} with no describe");
    }
    drop(held);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The key is where the socket is bound, not what the table says of the name: a real socket under
/// an alias's name belongs to whatever bound it, and is read as that id.
#[cfg(unix)]
#[test]
fn a_socket_bound_under_a_name_is_that_id_whatever_the_table_says() {
    let dir = std::env::temp_dir().join(format!("yantrik-app-guard-bound-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let _held = std::os::unix::net::UnixListener::bind(dir.join("app-text-editor.sock")).unwrap();
    assert_eq!(bound_id_in(&dir, "text-editor").as_deref(), Some("text-editor"));
    let view = graded("close", "standard");
    assert!(admit_here(&dir, "text-editor", "close", Some(&view)).is_err(), "`text-editor.close` is not on the list: only `editor.close` is");
    assert_eq!(admit_here(&nowhere(), "text-editor", "close", Some(&view)), Ok(()), "with no socket the table says editor");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_action_nobody_reviewed_and_the_app_does_not_call_safe_is_refused() {
    let dir = nowhere();
    // Not published at all, published above safe, graded with no such level, and no describe.
    assert!(admit_here(&dir, "notes", "purge", Some(&json!({ "actions": [] }))).is_err());
    assert!(admit_here(&dir, "notes", "purge", Some(&graded("purge", "dangerous"))).is_err());
    assert!(admit_here(&dir, "notes", "purge", Some(&graded("purge", "spicy"))).is_err());
    assert!(admit_here(&dir, "notes", "purge", Some(&graded("purge", "standard"))).is_err());
    assert!(admit_here(&dir, "notes", "purge", None).is_err());
    assert!(admit_here(&dir, "an-app-nobody-reviewed", "anything", Some(&graded("anything", "standard"))).is_err());
    // Graded safe by the app: in, without a review.
    assert_eq!(admit_here(&dir, "an-app-nobody-reviewed", "look", Some(&graded("look", "safe"))), Ok(()));
    // Reviewed: in, even when the app did not answer.
    assert_eq!(admit_here(&dir, "calendar", "show_month", None), Ok(()));
    assert_eq!(admit_here(&dir, "Calendar", "show_month", None), Ok(()));
    assert_eq!(admit_here(&dir, "container-manager", "refresh", None), Ok(()));
    assert_eq!(admit_here(&dir, "music-player", "pause", None), Ok(()));
}

/// What the review took off the reviewed list, each refused under its own id and by its alias.
#[test]
fn what_came_off_the_reviewed_list_is_refused() {
    for (id, action) in [
        ("terminal", "run"), ("terminal", "send_input"), ("shell", "agent_run"), ("shell", "agent_input"), ("blender", "run_python"),
        ("calendar", "delete_own_event"), ("calendar", "update_own_event"),
        ("notes", "undo"), ("notes", "tags"), ("notes", "notebook"),
    ] {
        assert!(!is_reviewed(id, action), "{id}.{action} is still reviewed");
        let said = admit_here(&nowhere(), id, action, Some(&graded(action, "standard"))).expect_err(action);
        assert!(said.contains("no tool that asks") || said.contains("Use `"), "{said}");
    }
}

#[test]
fn a_pair_with_an_asking_tool_names_it_and_is_refused_before_anything_starts() {
    for i in INSTEAD {
        assert!(!is_reviewed(i.id, i.action), "{}.{} is on the reviewed list", i.id, i.action);
        let said = refused_early_in(&nowhere(), &shipped, i.id, i.action).unwrap_or_else(|| panic!("{}.{} is not refused early", i.id, i.action));
        assert!(said.starts_with("Refused:") && said.contains(&format!("Use `{}`", i.tool)) && said.contains("Nothing was changed"), "{said}");
    }
    // By an alias, and by a spelling, through the shell's table.
    for (name, action) in [("text-editor", "save"), ("Text Editor", "set_content"), ("document-editor", "save_as"), ("Calendar", "delete_event")] {
        assert!(refused_early_in(&nowhere(), &shipped, name, action).is_some(), "{name}.{action}");
    }
    // An action of the same name in another app, and one nobody has a tool for, wait for `admit`.
    for (name, action) in [("notes", "delete_event"), ("weather", "save"), ("calendar", "add_event"), ("shell", "save"), ("notes", "set_content")] {
        assert_eq!(refused_early_in(&nowhere(), &shipped, name, action), None, "{name}.{action}");
    }
}

#[test]
fn the_reviewed_list_names_ids_not_folders_and_lists_each_pair_once() {
    let folders = ["text-editor", "document-editor", "snippet-manager", "container-manager", "music-player", "network-manager"];
    let mut seen = std::collections::HashSet::new();
    for group in REVIEWED {
        assert!(group.because.len() > 40, "a reason says something: {}", group.because);
        for pair in group.actions {
            let (id, _) = pair.split_once('.').unwrap_or_else(|| panic!("`{pair}` is not id.action"));
            assert!(!folders.contains(&id), "`{pair}` is keyed on a folder name, not the id the socket is bound under");
            assert!(seen.insert(*pair), "{pair} is listed twice");
        }
    }
    assert!(seen.len() < 120, "the list is small on purpose: {}", seen.len());
}

/// The road itself: `app_action` on a pair the companion has a tool for changes nothing, starts nothing and
/// raises no card, whatever the mode would have said.
#[test]
fn the_app_action_road_refuses_before_it_starts_or_asks_anything() {
    use crate::{PermissionLevel, ToolContext};
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    let ctx = ToolContext { db: &db, max_permission: PermissionLevel::Sensitive, registry_metadata: None, task_manager: None, incognito: false, agent_spawner: None };
    let shell = FakeShell::session(First::NotNeeded, Answer::Granted);
    for (app, action) in [("calendar", "delete_event"), ("calendar", "update_event"), ("Calendar", "delete_event"), ("editor", "set_content"), ("editor", "save"), ("documents", "save"), ("shell", "run_recipe")] {
        let said = crate::app_act::gated_act(&ctx, app, action, &json!({ "id": "x" }), "").expect_err("refused");
        assert!(said.starts_with("Refused:") && said.contains("Nothing was changed"), "{app}.{action}: {said}");
    }
    assert!(shell.calls().is_empty(), "no shell call, no card: {:?}", shell.calls());
}
