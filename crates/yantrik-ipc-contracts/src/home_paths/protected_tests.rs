use std::os::unix::fs::symlink;
use std::path::Path;

use serde_json::json;

use super::tests::{exists_and_reason, home};
use super::*;

/// The Mind's folders from before `yantrik-update migrate-minds`: its database, egress key and
/// tokens, and the environment file its service starts from. Under the home and where the session
/// moved its data and configuration homes (security review of #942, finding 4).
#[test]
fn the_minds_own_folders_are_protected() {
    for p in [
        "/home/ann/.local/share/yantrik-mind/mind.db",
        "/home/ann/.local/share/yantrik-mind/egress.key",
        "/home/ann/.local/share/yantrik-mind/yantrik-memory.token",
        "/home/ann/.config/yantrik-mind/console.token",
        "/home/ann/.config/yantrik-mind.env",
    ] {
        assert!(is_protected(Path::new(p)), "{p} must be protected");
    }
    let moved = Places { data_home: Some(Path::new("/srv/ann-data")), config_home: Some(Path::new("/srv/ann-config")), ..Places::default() };
    assert!(is_protected_with(Path::new("/srv/ann-data/yantrik-mind/console.token"), &moved));
    assert!(is_protected_with(Path::new("/srv/ann-config/yantrik-mind.env"), &moved));
    assert!(is_protected_with(Path::new("/srv/ann-config/yantrik-mind/x"), &moved));
    assert!(!is_protected_with(Path::new("/srv/ann-config/yantrik-mind.env.example"), &moved));
    let (_d, home) = home();
    std::fs::create_dir_all(home.join(".local/share/yantrik-mind")).unwrap();
    std::fs::write(home.join(".local/share/yantrik-mind/yantrik-memory.token"), "SENTINEL").unwrap();
    std::fs::create_dir_all(home.join(".config")).unwrap();
    std::fs::write(home.join(".config/yantrik-mind.env"), "SENTINEL").unwrap();
    for asked in ["~/.local/share/yantrik-mind/yantrik-memory.token", "~/.config/yantrik-mind.env"] {
        assert!(may_read_file(asked, &home).unwrap_err().ends_with(" is protected"), "{asked}");
    }
}

/// A dotfile manager's links: `~/.local`, `~/.config` and `~/.ssh` each a link to a folder
/// elsewhere in the home. A protected place is protected where it really is, not only by the name
/// the link gives it, and so is one not made yet (security review of #942, finding 3).
#[test]
fn a_protected_place_moved_behind_a_link_is_protected_where_it_really_is() {
    let (_d, home) = home();
    let dot = home.join("dotfiles");
    for dir in ["local/share/yantrik/snippets", "local/share/fonts", "config/autostart", "config/gtk-3.0", "ssh"] {
        std::fs::create_dir_all(dot.join(dir)).unwrap();
    }
    for file in ["local/share/yantrik/snippets/snippets.json", "local/share/fonts/a.ttf", "config/gtk-3.0/settings.ini", "ssh/id_ed25519"] {
        std::fs::write(dot.join(file), "SENTINEL").unwrap();
    }
    symlink(dot.join("local"), home.join(".local")).unwrap();
    symlink(dot.join("config"), home.join(".config")).unwrap();
    symlink(dot.join("ssh"), home.join(".ssh")).unwrap();

    for asked in [
        "~/dotfiles/local/share/yantrik/snippets/snippets.json",
        "~/.local/share/yantrik/snippets/snippets.json",
        "~/dotfiles/ssh/id_ed25519",
    ] {
        assert!(may_read_file(asked, &home).unwrap_err().ends_with(" is protected"), "{asked}");
        assert!(may_write_file(asked, &home).unwrap_err().ends_with(" is protected"), "{asked}");
    }
    for (asked, real) in [
        ("~/dotfiles/config/autostart/x.desktop", "local autostart"),
        ("~/dotfiles/local/state/yantrik/snippets/draft.json", "a state tree not made yet"),
        ("~/dotfiles/local/share/yantrik/new.json", "a new file in the data tree"),
    ] {
        assert_eq!(exists_and_reason(asked, &home), (json!("unknown"), json!("protected")), "{real}: {asked}");
    }
    assert!(may_create("~/dotfiles/local/share/yantrik/newdir", &home).unwrap_err().ends_with(" is protected"));
    // The rest of the folders behind the links are the person's own, as they were.
    assert_eq!(may_read_file("~/dotfiles/local/share/fonts/a.ttf", &home), Ok(()));
    assert_eq!(may_read_file("~/dotfiles/config/gtk-3.0/settings.ini", &home), Ok(()));
}

/// #1120: a Mind's workshops are held by the one check, so every door that asks "may an agent
/// look here" (the Files tools, the editor, the companion's file tools) refuses them, by their own
/// name, through a link, and through a `..`.
#[test]
fn a_minds_workshops_are_out_of_reach_of_every_door() {
    let (_d, home) = home();
    workshops::roots_under_home_for_tests();
    let (shop, r) = (workshops::root(&home).join("hermes/shop"), workshops::root(&home).display().to_string());
    let back = r.strip_prefix(&format!("{}/", home.display())).unwrap().to_string();
    std::fs::create_dir_all(&shop).unwrap();
    std::fs::write(shop.join("notes.md"), "calendar").unwrap();
    symlink(&shop, home.join("notes/alias")).unwrap();
    symlink(workshops::root(&home), home.join("to-root")).unwrap();
    for asked in [
        format!("{r}/hermes/shop/notes.md"),
        format!("{r}/hermes/shop/not-yet.md"),
        "~/notes/alias/notes.md".to_string(),
        "~/to-root/hermes/shop/notes.md".to_string(),
        format!("~/notes/../{back}/hermes/shop/notes.md"),
    ] {
        let asked_for = if asked.contains("..") { home.join(&asked[2..]).display().to_string() } else { asked.clone() };
        assert!(may_read_file(&asked_for, &home).is_err(), "read {asked}");
        assert!(may_write_file(&asked_for, &home).is_err(), "write {asked}");
        assert!(may_create(&asked_for, &home).is_err(), "create {asked}");
        assert!(!stat(&asked_for, &home)["exists"].is_boolean(), "stat {asked}");
    }
    let held = |p: &Path| is_protected_in(p, Some(&home));
    assert!(held(&shop.join("notes.md")) && held(&home.join("notes/alias/notes.md")) && held(&workshops::root(&home)));
    assert!(!held(&home.join("notes/today.txt")));
    // A moved data home keeps its workshops held too.
    let moved = home.join("elsewhere");
    std::fs::create_dir_all(moved.join("yantrik/workshops/pi")).unwrap();
    let places = Places { home: Some(&home), data_home: Some(&moved), ..Default::default() };
    assert!(is_protected_with(&moved.join("yantrik/workshops/pi/x"), &places));
}
