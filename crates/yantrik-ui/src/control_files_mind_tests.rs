use std::os::unix::fs::symlink;
use std::path::PathBuf;

use super::*;

/// A home of its own for each test, removed when the guard drops.
struct Home(PathBuf);
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn home(name: &str) -> (Home, PathBuf) {
    let dir = std::env::temp_dir().join(format!("yantrik-files-mind-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Documents")).unwrap();
    std::fs::create_dir_all(dir.join(".ssh")).unwrap();
    std::fs::write(dir.join("notes.txt"), "x").unwrap();
    let home = dir.canonicalize().unwrap();
    (Home(home.clone()), home)
}

/// The refusal ends with the reason; the words in front name both rules and prove nothing.
fn refused_because(result: Result<(), String>, why: &str, asked: &str) {
    let err = result.expect_err(asked);
    assert!(err.ends_with(why), "{asked}: {err}");
}

#[test]
fn a_mind_opens_folders_in_the_home_and_nowhere_else() {
    let (_d, home) = home("open");
    symlink("/etc", home.join("escape")).unwrap();
    let open = |p: &str| open_verdict(p, &home);

    assert!(open("~").is_ok());
    assert!(open("~/Documents").is_ok());
    assert!(open(home.join("Documents").to_str().unwrap()).is_ok(), "absolute too");
    for (asked, why) in [
        ("/etc", " is outside"),
        ("/home", " is outside"),
        ("~/escape", " is outside"),
        ("~/.ssh", " is protected"),
        ("Trash", " is not_a_path"),
        ("~/notes.txt", " is not a folder"),
        // The refusal names the call that makes it: told only "no folder", a mind on VM 520 never made one.
        ("~/Nowhere", "call files_new_folder with name \"~/Nowhere\" (missing parent folders are made too)"),
    ] {
        refused_because(open(asked), why, asked);
    }
}

#[test]
fn a_protected_folder_split_across_a_link_stays_closed() {
    let (_d, home) = home("split");
    std::fs::create_dir_all(home.join(".config/labwc")).unwrap();
    std::fs::create_dir_all(home.join("x")).unwrap();
    symlink(home.join(".config"), home.join("x/c")).unwrap();
    assert!(open_verdict("~/x/c", &home).is_ok(), ".config itself is not protected");
    refused_because(open_verdict("~/x/c/labwc", &home), " is protected", "~/x/c/labwc");
}

#[test]
fn a_link_to_a_protected_folder_stays_closed_below_it_too() {
    let (_d, home) = home("keys");
    symlink(home.join(".ssh"), home.join("keys")).unwrap();
    refused_because(open_verdict("~/keys", &home), " is protected", "~/keys");
    refused_because(open_verdict("~/keys/not-there", &home), " is protected", "~/keys/not-there");
}

#[test]
fn a_link_loop_opens_nothing() {
    let (_d, home) = home("loop");
    symlink("loop", home.join("loop")).unwrap();
    refused_because(open_verdict("~/loop", &home), " is broken_link", "~/loop");
    refused_because(open_verdict("~/loop/deeper", &home), " is broken_link", "~/loop/deeper");
}

#[test]
fn a_folder_the_person_cannot_read_is_not_opened() {
    use std::os::unix::fs::PermissionsExt;
    let (_d, home) = home("locked");
    let locked = home.join("locked");
    std::fs::create_dir_all(locked.join("inner")).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    // Root enters anything, and then the lock proves nothing.
    let ignored = std::fs::read_dir(&locked).is_ok();
    let verdict = open_verdict("~/locked/inner", &home);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    if !ignored {
        refused_because(verdict, " is not_allowed", "~/locked/inner");
    }
}

#[test]
fn a_relative_link_that_stays_in_the_home_opens_like_the_folder_it_names() {
    let (_d, home) = home("relative");
    symlink("Documents", home.join("Docs")).unwrap();
    assert!(open_verdict("~/Docs", &home).is_ok());
}

#[test]
fn up_from_the_home_is_the_folder_above_it_and_is_refused() {
    // The label says `~`; checking `Path::new("~").parent()` asked about "", not about /home.
    let (_d, home) = home("up");
    let above = up_from("~");
    let real_home = PathBuf::from(std::env::var("HOME").unwrap());
    assert_eq!(PathBuf::from(&above), real_home.parent().unwrap(), "the loaded path, not \"\"");
    refused_because(open_verdict(&above, &home), " is outside", &above);
    // And from a folder in the home, up is the home.
    assert_eq!(up_from("~/Documents"), "~");
    assert!(open_verdict(&up_from("~/Documents"), &home).is_ok());
    assert_eq!(into("~", "Documents"), "~/Documents");
}

#[test]
fn a_mind_acts_only_on_a_folder_on_screen_it_may_see() {
    let (_d, home) = home("here");
    assert!(here_verdict("~", &home).is_ok());
    assert!(here_verdict("~/Documents", &home).is_ok());
    for label in ["/etc", "~/.ssh", "Trash", ""] {
        let err = here_verdict(label, &home).unwrap_err();
        assert!(label.is_empty() || !err.contains(label), "the folder is not named: {err}");
        assert!(err.starts_with("Files is showing a folder outside"), "{label}: {err}");
    }
}

#[test]
fn a_mind_opens_only_entries_that_are_there_and_not_protected() {
    let (_d, home) = home("entry");
    symlink("/etc/hostname", home.join("host")).unwrap();
    symlink(home.join(".ssh"), home.join("keys")).unwrap();
    let label = home.to_str().unwrap();
    assert!(entry_verdict(label, "notes.txt", &home).is_ok());
    assert!(entry_verdict(label, "Documents", &home).is_ok());
    refused_because(entry_verdict(label, "host", &home), " is outside", "host");
    refused_because(entry_verdict(label, "keys", &home), " is protected", "keys");
    refused_because(entry_verdict(label, ".ssh", &home), " is protected", ".ssh");
    assert!(entry_verdict(label, "gone.txt", &home).unwrap_err().starts_with("nothing is at"));
    assert!(entry_verdict("/etc", "hostname", &home).unwrap_err().starts_with("Files is showing"));
}

#[test]
fn a_hidden_folder_is_described_as_hidden_and_names_nothing() {
    let shown = serde_json::json!({
        "path": "~/.ssh",
        "entries": [{"name": "id_ed25519"}],
        "recent": [{"name": "id_ed25519"}],
        "shown": 1,
        "total": 1,
        "selected": "id_ed25519",
        "selection_count": 1,
        "preview_name": "id_ed25519",
        "notice": "Copied id_ed25519",
        "operation": "Copying id_ed25519",
        "view": "list",
        "places": [{"label": "Home", "path": "~"}],
    });
    let hidden = hide_folder(shown, HIDDEN);
    assert!(!hidden.to_string().contains("id_ed25519"), "{hidden}");
    assert!(!hidden.to_string().contains(".ssh"), "{hidden}");
    assert_eq!(hidden["hidden"], HIDDEN);
    assert_eq!(hidden["view"], "list", "what says nothing about the folder stays");
    assert_eq!(hidden["places"][0]["path"], "~");
}

#[test]
fn a_mind_does_not_name_anything_into_a_protected_place() {
    // files_go ~/.local/share, new_folder x, save a .desktop into x, files_rename x applications:
    // the rename makes it the menu's desktop entries.
    let (_d, home) = home("make");
    std::fs::create_dir_all(home.join(".local/share/x")).unwrap();
    std::fs::create_dir_all(home.join(".config")).unwrap();
    let share = home.join(".local/share");
    let share = share.to_str().unwrap();
    refused_because(make_verdict(share, "fonts", &home), " is hidden_place", "fonts under .local");
    assert!(make_verdict(home.join("Documents").to_str().unwrap(), "fonts", &home).is_ok());
    refused_because(make_verdict(share, "applications", &home), " is protected", "applications");
    let config = home.join(".config");
    let config = config.to_str().unwrap();
    for name in ["autostart", "systemd", "mimeapps.list", "labwc"] {
        refused_because(make_verdict(config, name, &home), " is protected", name);
    }
    refused_because(make_verdict(home.to_str().unwrap(), ".bashrc", &home), " is protected", ".bashrc");
    assert!(make_verdict(home.to_str().unwrap(), "a/b", &home).is_err(), "a name, not a path");
}

#[test]
fn renaming_the_keys_folder_away_is_refused() {
    // `files_rename .ssh keys` checks the entry it renames, not only the name it gets.
    let (_d, home) = home("rename");
    refused_because(entry_verdict(home.to_str().unwrap(), ".ssh", &home), " is protected", ".ssh");
    assert!(make_verdict(home.to_str().unwrap(), "keys", &home).is_ok(), "the new name alone is harmless");
    refused_because(rename_verdict(home.to_str().unwrap(), ".ssh", "keys", &home), " is protected", ".ssh -> keys");
}

#[test]
fn a_folder_renamed_so_that_what_is_below_becomes_a_startup_place_is_refused() {
    // `cfg` holding autostart/evil.desktop, renamed: to `.config` it is hidden; to a name whose
    // tree lands as a protected place it is caught below the top.
    let (_d, home) = home("rename-tree");
    std::fs::create_dir_all(home.join("cfg/autostart")).unwrap();
    std::fs::write(home.join("cfg/autostart/evil.desktop"), "[Desktop Entry]").unwrap();
    let label = home.to_str().unwrap();
    refused_because(rename_verdict(label, "cfg", ".config", &home), " is hidden_place", "cfg -> .config");
    std::fs::create_dir_all(home.join("stuff/.config")).unwrap();
    std::fs::rename(home.join("cfg"), home.join("stuff/cfg")).unwrap();
    // Under a folder that is itself hidden only below the home: the walk sees what lands.
    assert!(home_paths::may_land(&home.join("stuff/cfg"), &home.join("stuff/.config"), &home).is_err());
    std::fs::create_dir_all(home.join("photos/2026")).unwrap();
    assert!(rename_verdict(label, "photos", "pictures", &home).is_ok(), "an ordinary rename");
}

#[test]
fn an_unreadable_clipboard_refuses_a_paste_rather_than_passing_it() {
    let (_d, home) = home("clipboard");
    let label = home.to_str().unwrap();
    let err = paste_now_verdict(Err("the Files clipboard cannot be read right now".into()), label, &home).unwrap_err();
    assert!(err.contains("cannot be read"), "{err}");
    assert!(paste_now_verdict(Ok(None), label, &home).is_ok(), "an empty clipboard pastes nothing");
    // And the shell's own reader, with no Files browser wired in this process, says it cannot.
    assert!(crate::wire::files::clipboard_now().is_err());
}

/// Run `check` as a call from an agent that presented a token: `requester_now` says a mind.
fn as_a_mind<T>(check: impl FnOnce() -> T) -> T {
    use yantrik_app_runtime::control::{AgentTokenScope, Caller, CallerScope};
    let _held = crate::control_agent_terminal::RESOLVER_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let me = unsafe { libc::getuid() };
    let _caller = CallerScope::enter(Some(Caller { pid: 4242, uid: me, gid: me }));
    let _token = AgentTokenScope::enter(Some("tok-files-mind-test".into()));
    assert_ne!(requester_now(), Requester::Person, "the scopes make this a mind's call");
    check()
}

#[test]
fn a_mind_does_not_undo_a_trash_or_open_the_trash() {
    assert!(may_undo_trash().is_ok(), "the person undoes");
    assert!(may_show_trash().is_ok());
    as_a_mind(|| {
        assert!(may_undo_trash().unwrap_err().contains("a mind does not"));
        assert!(may_show_trash().is_err());
        // With no browser to read, a mind's paste is refused, not waved through.
        assert!(may_paste("~").is_err());
    });
}
