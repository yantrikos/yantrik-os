use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::*;

/// A home of its own for each test, removed when the guard drops.
struct Home(PathBuf);
impl Drop for Home {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        // A test that locked a folder must not leave it behind undeletable.
        if let Ok(walk) = std::fs::read_dir(&self.0) {
            for entry in walk.flatten() {
                let _ = std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o755));
            }
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn home() -> (Home, PathBuf) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "yantrik-home-paths-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(dir.join("notes")).unwrap();
    std::fs::write(dir.join("notes/today.txt"), "kept safe").unwrap();
    let home = dir.canonicalize().unwrap();
    (Home(home.clone()), home)
}

fn exists_and_reason(asked: &str, home: &Path) -> (serde_json::Value, serde_json::Value) {
    let v = stat(asked, home);
    (v["exists"].clone(), v["reason"].clone())
}

#[test]
fn what_is_there_is_described() {
    let (_d, home) = home();
    let v = stat("~/notes/today.txt", &home);
    assert_eq!(v["exists"], true);
    assert_eq!(v["kind"], "file");
    assert_eq!(v["size"], 9);
    assert!(v["modified"].as_u64().is_some());
    assert_eq!(stat("~/notes", &home)["kind"], "directory");
    assert_eq!(stat("~/notes/", &home)["kind"], "directory", "a trailing slash");
    assert_eq!(stat(home.join("notes").to_str().unwrap(), &home)["exists"], true, "absolute too");
}

#[test]
fn where_a_path_really_resolves_is_said_and_whether_a_link_took_it_there() {
    let (_d, home) = home();
    let v = stat("~/notes/today.txt", &home);
    assert_eq!(v["real"], home.join("notes/today.txt").to_str().unwrap());
    assert_eq!(v["via_link"], false);
    symlink(home.join("notes/today.txt"), home.join("alias.txt")).unwrap();
    symlink(home.join("notes"), home.join("shelf")).unwrap();
    for asked in ["~/alias.txt", "~/shelf/today.txt"] {
        let v = stat(asked, &home);
        assert_eq!(v["exists"], true, "{asked}");
        assert_eq!(v["real"], home.join("notes/today.txt").to_str().unwrap(), "{asked}");
        assert_eq!(v["via_link"], true, "{asked}");
    }
}

#[test]
fn changed_is_the_ctime_and_a_backdated_mtime_does_not_move_it_back() {
    let (_d, home) = home();
    let file = home.join("notes/today.txt");
    let before = stat("~/notes/today.txt", &home)["changed"].as_i64().unwrap();
    // `touch -d` moves mtime into the past; ctime goes forward instead.
    let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
    std::fs::File::options().write(true).open(&file).unwrap().set_modified(old).unwrap();
    let v = stat("~/notes/today.txt", &home);
    assert_eq!(v["modified"], 1_000_000_000);
    assert!(v["changed"].as_i64().unwrap() >= before, "ctime never goes back");
    assert!(v["changed"].as_i64().unwrap() > 1_000_000_000);
}

#[test]
fn missing_is_false_and_says_so() {
    let (_d, home) = home();
    let v = stat("~/notes/tomorrow.txt", &home);
    assert_eq!(v["exists"], false);
    assert_eq!(v["reason"], "not_found");
    assert_eq!(stat("~/nowhere/at/all.txt", &home)["exists"], false);
    assert_eq!(stat("~/notes/today.txt/inside", &home)["reason"], "not_found", "ENOTDIR");
    assert_eq!(stat("~/notes/today.txt/", &home)["exists"], true, "a trailing slash on a file");
}

#[test]
fn outside_the_home_is_never_true_or_false() {
    let (_d, home) = home();
    for asked in ["/etc/passwd", "/", "/nonexistent/file", "~//etc/passwd"] {
        assert_eq!(stat(asked, &home)["exists"], "unknown", "{asked}");
    }
}

#[test]
fn a_link_out_of_the_home_answers_nothing_about_where_it_leads() {
    let (_d, home) = home();
    symlink("/etc", home.join("escape")).unwrap();
    // Whether /etc/ssh is there or /etc/no-such-dir is not, the answer is the same.
    for asked in [
        "~/escape/passwd",
        "~/escape/not-there",
        "~/escape/ssh/anything",
        "~/escape/no-such-dir/anything",
        "~/escape/no-such-dir/deeper/still",
    ] {
        assert_eq!(exists_and_reason(asked, &home), (json!("unknown"), json!("outside")), "{asked}");
    }
}

#[test]
fn a_dangling_link_is_not_known_to_be_anything() {
    let (_d, home) = home();
    symlink("/etc/yantrik-no-such-file", home.join("dangling")).unwrap();
    for asked in ["~/dangling", "~/dangling/below"] {
        assert_eq!(exists_and_reason(asked, &home), (json!("unknown"), json!("broken_link")), "{asked}");
    }
}

#[test]
fn a_link_that_leads_back_to_itself_is_not_known_to_be_anything() {
    let (_d, home) = home();
    symlink("loop", home.join("loop")).unwrap();
    symlink(home.join("ping"), home.join("pong")).unwrap();
    symlink(home.join("pong"), home.join("ping")).unwrap();
    for asked in ["~/loop", "~/loop/new.txt", "~/ping/deeper/new.txt"] {
        assert_eq!(exists_and_reason(asked, &home), (json!("unknown"), json!("broken_link")), "{asked}");
        assert!(may_write_file(asked, &home).is_err(), "{asked}");
    }
}

#[test]
fn protected_places_are_not_answered_for() {
    let (_d, home) = home();
    std::fs::create_dir(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh/id_ed25519"), "key").unwrap();
    std::fs::create_dir_all(home.join(".config/yantrik")).unwrap();
    for asked in [
        "~/.ssh",
        "~/.ssh/id_ed25519",
        "~/.ssh/not-there",
        "~/.config/yantrik/memory.db",
        "~/.bash_history",
        "~/.bash_profile",
        "~/.zshrc",
        "~/.config/autostart/evil.desktop",
        "~/.config/systemd/user/evil.service",
        "~/.local/share/applications/evil.desktop",
    ] {
        assert_eq!(exists_and_reason(asked, &home), (json!("unknown"), json!("protected")), "{asked}");
    }
    // Whole components only.
    std::fs::create_dir(home.join(".ssh-notes")).unwrap();
    assert_eq!(stat("~/.ssh-notes", &home)["exists"], true);
    // Nor through a link that leads into one.
    symlink(home.join(".ssh"), home.join("keys")).unwrap();
    assert_eq!(stat("~/keys/id_ed25519", &home)["reason"], "protected");
}

#[test]
fn a_protected_name_split_across_a_link_is_still_protected() {
    // `c -> ~/.config` is not protected, and neither is `labwc/autostart` on its own; written
    // through the link, the file lands in ~/.config/labwc, and the shell runs it at login.
    let (_d, home) = home();
    std::fs::create_dir_all(home.join(".config")).unwrap();
    std::fs::create_dir_all(home.join("x")).unwrap();
    symlink(home.join(".config"), home.join("x/c")).unwrap();
    for asked in ["~/x/c/labwc/autostart", "~/x/c/yantrik/config.yaml", "~/x/c/systemd/user/a.service"] {
        assert_eq!(exists_and_reason(asked, &home), (json!("unknown"), json!("protected")), "{asked}");
        assert!(may_write_file(asked, &home).unwrap_err().ends_with(" is protected"), "{asked}");
    }
    // The same link to somewhere ordinary under .config is fine.
    assert_eq!(stat("~/x/c/gtk-3.0/settings.ini", &home)["exists"], false);
}

#[test]
fn a_link_into_a_protected_folder_protects_what_is_not_there_yet() {
    let (_d, home) = home();
    std::fs::create_dir(home.join(".ssh")).unwrap();
    symlink(home.join(".ssh"), home.join("keys")).unwrap();
    for asked in ["~/keys/authorized_keys", "~/keys/new/deeper/file"] {
        assert_eq!(exists_and_reason(asked, &home), (json!("unknown"), json!("protected")), "{asked}");
    }
}

#[test]
fn a_relative_link_that_stays_in_the_home_is_followed_like_any_folder() {
    let (_d, home) = home();
    symlink("notes", home.join("jottings")).unwrap();
    assert_eq!(stat("~/jottings/today.txt", &home)["exists"], true);
    assert_eq!(stat("~/jottings/tomorrow.txt", &home)["exists"], false);
    assert!(may_read_file("~/jottings/today.txt", &home).is_ok());
    assert!(may_write_file("~/jottings/tomorrow.txt", &home).is_ok());
}

#[test]
fn what_is_not_a_path_is_said_to_be_not_a_path() {
    let (_d, home) = home();
    for asked in ["", "notes/today.txt", "~bob/x", "~/notes/../../etc", "~/no\0te"] {
        assert_eq!(stat(asked, &home)["reason"], "not_a_path", "{asked:?}");
    }
}

#[test]
fn a_home_that_is_the_root_answers_for_nothing() {
    for home in ["/", "", "relative"] {
        assert_eq!(stat("/etc/passwd", Path::new(home))["reason"], "outside", "{home:?}");
    }
}

/// Whether this process can enter a folder with no permissions at all - root can - which makes
/// a test about a locked folder prove nothing.
fn locks_are_ignored_here(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok()
}

#[test]
fn hidden_is_not_missing() {
    // A directory the person cannot enter: what is inside is not known, which is not the same
    // as not there.
    use std::os::unix::fs::PermissionsExt;
    let (_d, home) = home();
    let locked = home.join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::write(locked.join("inside.txt"), "x").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if locks_are_ignored_here(&locked) {
        return;
    }
    let v = exists_and_reason("~/locked/inside.txt", &home);
    let w = stat("~/locked/never-there.txt", &home);
    let read = may_read_file("~/locked/inside.txt", &home);
    let write = may_write_file("~/locked/new.txt", &home);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(v, (json!("unknown"), json!("not_allowed")));
    assert_eq!(w["exists"], "unknown", "what is not there is not known either, behind a door");
    assert!(read.unwrap_err().ends_with(" is not_allowed"));
    assert!(write.unwrap_err().ends_with(" is not_allowed"));
}

#[test]
fn an_agent_reads_only_files_that_are_there_in_the_home() {
    let (_d, home) = home();
    std::fs::create_dir(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh/id_ed25519"), "key").unwrap();
    symlink("/etc/hostname", home.join("host")).unwrap();
    assert!(may_read_file("~/notes/today.txt", &home).is_ok());
    for (refused, why) in [
        ("~/.ssh/id_ed25519", " is protected"),
        ("/etc/hostname", " is outside"),
        ("~/host", " is outside"),
        ("~/notes", " is not a file"),
        ("~/notes/nothing.txt", "there is no file at ~/notes/nothing.txt"),
        ("notes/today.txt", " is not_a_path"),
    ] {
        let err = may_read_file(refused, &home).unwrap_err();
        assert!(err.ends_with(why), "{refused}: {err}");
    }
}

#[test]
fn an_agent_writes_into_folders_that_are_there_and_never_through_a_link() {
    let (_d, home) = home();
    symlink(home.join("notes/today.txt"), home.join("notes/alias.txt")).unwrap();
    assert!(may_write_file("~/notes/new.txt", &home).is_ok(), "a new file in a folder that is there");
    assert!(may_write_file("~/notes/today.txt", &home).is_ok(), "saving over a file in the home");
    for (refused, why) in [
        ("~/.bashrc", " is protected"),
        ("~/.profile", " is protected"),
        ("~/.config/autostart/run.desktop", " is protected"),
        ("/etc/profile.d/x.sh", " is outside"),
        ("~/notes/alias.txt", " is a link; an agent writes a file, not through a link to one"),
        ("~/notes", " is not a file"),
        ("~/nowhere/new.txt", "does not exist"),
    ] {
        let err = may_write_file(refused, &home).unwrap_err();
        assert!(err.ends_with(why), "{refused}: {err}");
    }
}

/// A missing folder's refusal says how to get past it, in the path a caller can hand straight to
/// files_new_folder (VM 520, 4 October: a mind made the folder and never saved again).
#[test]
fn a_missing_folder_says_how_to_make_it_and_that_nothing_is_lost() {
    let (_d, home) = home();
    let err = may_write_file("~/longtask/recipes/index.html", &home).unwrap_err();
    assert!(err.contains("files_new_folder, name \"~/longtask/recipes\""), "{err}");
    assert!(err.contains("send the same save again") && err.contains("nothing written so far is lost"), "{err}");
    assert!(err.ends_with("does not exist"), "the reason still ends it: {err}");
}

/// Credential stores and the harnesses' key-bearing state are protected, for reading and writing
/// alike; names that merely start the same are not (security review of #617).
#[test]
fn credential_files_are_protected_and_lookalikes_are_not() {
    for p in [
        "/home/ann/.aws/credentials",
        "/home/ann/.git-credentials",
        "/home/ann/.netrc",
        "/home/ann/.config/gh/hosts.yml",
        "/home/ann/.docker/config.json",
        "/home/ann/.kube/config",
        "/home/ann/.npmrc",
        "/home/ann/.cargo/credentials.toml",
        "/home/ann/.local/share/keyrings/login.keyring",
        "/home/ann/.claude.json",
        "/home/ann/.hermes/config.yaml",
    ] {
        assert!(is_protected(Path::new(p)), "{p} must be protected");
    }
    for p in ["/home/ann/.aws-notes.txt", "/home/ann/.docker/other.json", "/home/ann/notes/.netrc-howto.md", "/home/ann/pictures/.pi-day.png"] {
        assert!(!is_protected(Path::new(p)), "{p} is not a credential store");
    }
}

#[test]
fn protected_names_match_whole_components_only() {
    assert!(is_protected(Path::new("/home/ann/.ssh/id_ed25519")));
    assert!(is_protected(Path::new("/home/ann/.config/labwc/rc.xml")));
    assert!(is_protected(Path::new("/home/ann/.local/share/applications/a.desktop")));
    assert!(!is_protected(Path::new("/home/ann/.ssh-notes/a")));
    assert!(!is_protected(Path::new("/home/ann/labwc/.config")));
    assert!(!is_protected(Path::new("/home/ann/.config/labwc-themes/a")));
    assert!(!is_protected(Path::new("/home/ann/.local/share/applications-old")));
}

#[test]
fn a_file_with_a_second_name_is_not_known_to_be_anything() {
    // The other name may be anywhere; a hard link to a key looks like any file in the home.
    let (_d, home) = home();
    std::fs::hard_link(home.join("notes/today.txt"), home.join("twin.txt")).unwrap();
    for asked in ["~/twin.txt", "~/notes/today.txt"] {
        assert_eq!(exists_and_reason(asked, &home), (json!("unknown"), json!("hard_link")), "{asked}");
        assert!(may_read_file(asked, &home).unwrap_err().ends_with(" is hard_link"), "{asked}");
        assert!(may_write_file(asked, &home).unwrap_err().ends_with(" is hard_link"), "{asked}");
    }
    std::fs::remove_file(home.join("twin.txt")).unwrap();
    assert_eq!(stat("~/notes/today.txt", &home)["exists"], true, "one name again, an ordinary file");
}

#[test]
fn which_program_opens_what_is_protected() {
    let (_d, home) = home();
    std::fs::create_dir_all(home.join(".config")).unwrap();
    std::fs::write(home.join(".config/mimeapps.list"), "[Default Applications]\n").unwrap();
    assert_eq!(exists_and_reason("~/.config/mimeapps.list", &home), (json!("unknown"), json!("protected")));
    assert!(may_write_file("~/.config/mimeapps.list", &home).unwrap_err().ends_with(" is protected"));
}

#[test]
fn desktop_entries_under_a_moved_data_home_are_protected() {
    let xdg = Path::new("/srv/ann-data");
    assert!(is_protected_with(Path::new("/srv/ann-data/applications/term.desktop"), &data(xdg)));
    assert!(is_protected_with(Path::new("/srv/ann-data/applications"), &data(xdg)));
    assert!(!is_protected_with(Path::new("/srv/ann-data/applications-old/a"), &data(xdg)));
    assert!(!is_protected_with(Path::new("/srv/ann-data/fonts/a.ttf"), &data(xdg)));
    assert!(!is_protected_with(Path::new("/srv/ann-data/applications/a"), &Places::default()));
}

#[test]
fn startup_places_under_a_moved_config_home_are_protected() {
    let xdg = Path::new("/srv/ann-config");
    for place in ["autostart/run.desktop", "systemd/user/a.service", "environment.d/x.conf", "labwc/rc.xml", "yantrik/config.yaml", "mimeapps.list"] {
        assert!(is_protected_with(&xdg.join(place), &Places { config_home: Some(xdg), ..Places::default() }), "{place}");
    }
    assert!(!is_protected_with(Path::new("/srv/ann-config/gtk-3.0/settings.ini"), &Places { config_home: Some(xdg), ..Places::default() }));
}

#[test]
fn protected_names_are_matched_whatever_their_case() {
    // A case-folding filesystem makes `.SSH` the folder `.ssh`.
    assert!(is_protected(Path::new("/home/ann/.SSH/id_ed25519")));
    assert!(is_protected(Path::new("/home/ann/.Config/Autostart/x.desktop")));
    assert!(is_protected(Path::new("/home/ann/Memory.DB")));
    assert!(!is_protected(Path::new("/home/ann/.SSH-notes")));
}

#[test]
fn nothing_is_made_where_it_would_become_a_protected_place() {
    // files_go ~/.local/share, new_folder x, save a .desktop into it, rename x to applications:
    // each step looked harmless, and the last one made a desktop entry the person's menu runs.
    let (_d, home) = home();
    std::fs::create_dir_all(home.join(".local/share/x")).unwrap();
    std::fs::create_dir_all(home.join(".config")).unwrap();
    assert!(may_create("~/notes/new-folder", &home).is_ok());
    assert!(may_create("~/.local/share/x", &home).unwrap_err().ends_with(" is hidden_place"), "a hidden folder");
    for asked in [
        "~/.local/share/applications",
        "~/.config/autostart",
        "~/.config/systemd",
        "~/.config/mimeapps.list",
        "~/.ssh",
    ] {
        assert!(may_create(asked, &home).unwrap_err().ends_with(" is protected"), "{asked}");
    }
    symlink("/etc", home.join("escape")).unwrap();
    assert!(may_create("~/escape/new", &home).unwrap_err().ends_with(" is outside"));
}

/// Security review of #828, finding 1: a saved screenshot or recording is a picture of the
/// person's desktop, and no agent reads one by its path, opens its folder, or carries it out in a
/// copy of the folder above. The person's other pictures and videos are untouched.
#[test]
fn the_persons_screenshots_and_recordings_are_no_agents_to_read() {
    let (_d, home) = home();
    std::fs::create_dir_all(home.join("Pictures/Screenshots")).unwrap();
    std::fs::create_dir_all(home.join("Videos/Recordings")).unwrap();
    let shot = "Screenshot 2026-10-15 at 14.32.07.png";
    std::fs::write(home.join("Pictures/Screenshots").join(shot), "png").unwrap();
    std::fs::write(home.join("Videos/Recordings/Recording 2026-10-15 at 14.32.07.mp4"), "mp4").unwrap();
    std::fs::write(home.join("Pictures/holiday.png"), "png").unwrap();
    for refused in [
        format!("~/Pictures/Screenshots/{shot}"),
        "~/Videos/Recordings/Recording 2026-10-15 at 14.32.07.mp4".to_string(),
        "~/Pictures/Screenshots".to_string(),
        "~/pictures/SCREENSHOTS".to_string(),
    ] {
        assert_eq!(exists_and_reason(&refused, &home), (json!("unknown"), json!("protected")), "{refused}");
        assert!(may_read_file(&refused, &home).is_err(), "{refused}");
    }
    // Through a link, too: the link is in the home, where it leads is not.
    symlink(home.join("Pictures/Screenshots"), home.join("shots")).unwrap();
    assert!(may_read_file(&format!("~/shots/{shot}"), &home).unwrap_err().ends_with(" is protected"));
    // Copying the folder above them would carry them somewhere readable.
    assert!(may_land(&home.join("Pictures"), &home.join("notes/Pictures"), &home).unwrap_err().contains("protected"));
    assert!(may_read_file("~/Pictures/holiday.png", &home).is_ok(), "the person's other pictures");
    for not in ["/home/ann/Pictures/Screenshots-old/a.png", "/home/ann/Videos/film.mp4", "/home/ann/Recordings/a.mp4"] {
        assert!(!is_protected(Path::new(not)), "{not}");
    }
}

fn data(home: &Path) -> Places<'_> {
    Places { data_home: Some(home), ..Places::default() }
}

/// Yantrik's own data and state are whole trees no agent reads as a file: the snippets library and
/// its exports, Email's draft and its unsent mail, Notes' drafts, and whatever else an app keeps
/// there. Each app's data goes through that app's own door. Names that merely start the same are
/// not protected (security review of #940, High 1).
#[test]
fn yantriks_own_data_and_state_are_protected_and_lookalikes_are_not() {
    for p in [
        "/home/ann/.local/share/yantrik/snippets/snippets.json",
        "/home/ann/.local/share/yantrik/snippets/export-1789862400.json",
        "/home/ann/.local/state/yantrik/snippets/drafts/snippet-3.json",
        "/home/ann/.local/share/yantrik/email/draft.json",
        "/home/ann/.local/state/yantrik/email/drafts/composing.json",
        "/home/ann/.local/state/yantrik/notes/drafts/note-today.json",
        "/home/ann/.local/share/yantrik",
        "/home/ann/.LOCAL/Share/Yantrik/x",
    ] {
        assert!(is_protected(Path::new(p)), "{p} must be protected");
    }
    for p in ["/home/ann/.local/share/yantrik-minds-old/x", "/home/ann/.local/share/yantrikos/x", "/home/ann/yantrik/notes.md", "/home/ann/.local/share/fonts/a.ttf"] {
        assert!(!is_protected(Path::new(p)), "{p} is not Yantrik's own");
    }
    // The same trees where the session moved its data and state homes.
    let moved = Places { data_home: Some(Path::new("/srv/ann-data")), state_home: Some(Path::new("/srv/ann-state")), config_home: None, home: None };
    assert!(is_protected_with(Path::new("/srv/ann-data/yantrik/snippets/snippets.json"), &moved));
    assert!(is_protected_with(Path::new("/srv/ann-state/yantrik/email/drafts/composing.json"), &moved));
    assert!(!is_protected_with(Path::new("/srv/ann-state/yantrik-old/x"), &moved));
    assert!(!is_protected_with(Path::new("/srv/ann-state/yantrik/x"), &Places::default()), "only where the session put it");
}

/// What an agent asking for one of them is told: protected, before anything is looked at, whether
/// or not the file is there.
#[test]
fn an_agent_reading_yantriks_own_files_is_refused_as_protected() {
    let (_d, home) = home();
    std::fs::create_dir_all(home.join(".local/share/yantrik/snippets")).unwrap();
    std::fs::write(home.join(".local/share/yantrik/snippets/snippets.json"), "{}").unwrap();
    for asked in [
        "~/.local/share/yantrik/snippets/snippets.json",
        "~/.local/state/yantrik/email/drafts/composing.json",
        "~/.local/state/yantrik/notes/drafts/note-a.json",
    ] {
        assert!(may_read_file(asked, &home).unwrap_err().ends_with(" is protected"), "{asked}");
        assert!(may_write_file(asked, &home).unwrap_err().ends_with(" is protected"), "{asked}");
    }
}

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
