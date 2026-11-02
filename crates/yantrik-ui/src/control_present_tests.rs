use std::os::unix::fs::symlink;

use slint::Model;

use super::*;

/// A home of its own for each test, removed when the guard drops.
struct Home(PathBuf);
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn home(name: &str) -> (Home, PathBuf) {
    let dir = std::env::temp_dir().join(format!("yantrik-present-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for d in ["Sites", ".ssh", "Documents"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    std::fs::write(dir.join("Sites/index.html"), "<h1>hi</h1>").unwrap();
    std::fs::write(dir.join("Sites/notes.txt"), "n").unwrap();
    std::fs::write(dir.join(".ssh/id_ed25519"), "key").unwrap();
    let home = dir.canonicalize().unwrap();
    (Home(home.clone()), home)
}

fn ask(home: &Path, path: &str) -> Result<Offer, String> {
    plan(&json!({ "path": home.join(path).to_str().unwrap() }), home)
}

fn mind(name: &str) -> Requester {
    Requester::Mind(name.into())
}

#[test]
fn a_file_in_the_home_is_offered_and_the_card_names_maker_file_full_path_and_what_a_press_does() {
    let (_d, home) = home("card");
    let offer = ask(&home, "Sites/index.html").unwrap();
    assert_eq!(offer.real, home.join("Sites/index.html"));
    assert_eq!(offer.opens, Opens::Apart);
    let card = card("local:present-file:x", &mind("Yantrik Mind"), &offer, "");
    assert_eq!(card.summary.as_str(), "Yantrik Mind made index.html");
    let body = card.body.to_string();
    assert!(body.starts_with(home.join("Sites/index.html").to_str().unwrap()), "the whole path, uncut: {body}");
    assert!(body.contains("Made by Yantrik Mind."), "{body}");
    assert!(body.contains("separate browser: no network, none of your sign-ins"), "{body}");
    assert!(body.contains("will not show"), "what will not load is said: {body}");
    let actions: Vec<_> = card.actions.iter().map(|a| a.label.to_string()).collect();
    assert_eq!(actions, ["Open"]);
    let titled = super::card("local:present-file:x", &mind("Mind"), &offer, "your website");
    assert_eq!(titled.summary.as_str(), "Mind made your website");
    // Other types open as Files opens them, and say so.
    let txt = ask(&home, "Sites/notes.txt").unwrap();
    assert_eq!(txt.opens, Opens::App("editor".into()));
    assert!(super::card("i", &mind("Mind"), &txt, "").body.contains("Opens in editor."));
}

#[test]
fn a_caller_the_shell_cannot_name_is_never_shown_as_the_system() {
    let (_d, home) = home("maker");
    let offer = ask(&home, "Sites/index.html").unwrap();
    let c = card("i", &Requester::Person, &offer, "");
    assert!(c.summary.starts_with("Something on this computer made"), "{}", c.summary);
    assert!(!c.summary.contains("Yantrik made"));
    let c = card("i", &mind("  \u{202e}\u{7} "), &offer, "");
    assert!(c.summary.starts_with("Something on this computer made"), "{}", c.summary);
}

#[test]
fn what_is_outside_the_roots_or_protected_or_not_a_file_is_refused() {
    let (_d, home) = home("refuse");
    symlink("/etc/hostname", home.join("Sites/link.html")).unwrap();
    symlink("/etc", home.join("etc-link")).unwrap();
    let h = home.to_str().unwrap();
    for path in [
        "/etc/hostname".to_string(),
        format!("{h}/.ssh/id_ed25519"),
        format!("{h}/Sites/link.html"),
        format!("{h}/etc-link/hostname"),
        format!("{h}/Sites/../../etc/hostname"),
        format!("{h}/Sites/nope.html"),
        format!("{h}/Sites"),
        "Sites/index.html".to_string(),
        String::new(),
    ] {
        let err = plan(&json!({ "path": path }), &home).expect_err(&path);
        assert!(!err.is_empty(), "{path}");
    }
}

#[test]
fn a_name_cannot_lie_about_what_will_open() {
    let (_d, home) = home("names");
    // "holiday" + RIGHT-TO-LEFT OVERRIDE + "gpj.html" displays as "holidaylmth.jpg".
    for name in ["holiday\u{202e}gpj.html", "a\nsecond line.html", "zero\u{200b}width.html", "bell\u{7}.html"] {
        std::fs::write(home.join("Sites").join(name), "x").unwrap();
        let err = ask(&home, &format!("Sites/{name}")).expect_err(name);
        assert!(err.contains("control or direction characters"), "{name}: {err}");
    }
    // A title is cleaned, not trusted.
    let offer = ask(&home, "Sites/index.html").unwrap();
    let c = card("i", &mind("Mind"), &offer, "your\nwebsite\u{202e}!");
    assert_eq!(c.summary.as_str(), "Mind made yourwebsite!");
    // A long name is cut with its extension kept.
    let long = format!("{}.html", "x".repeat(200));
    let cut = shown(&long);
    assert!(cut.chars().count() <= SHOWN && cut.ends_with("….html"), "{cut}");
    assert_eq!(shown("short.html"), "short.html");
}

#[test]
fn the_card_names_the_real_file_not_the_name_of_a_link_to_it() {
    let (_d, home) = home("link");
    symlink(home.join("Sites/index.html"), home.join("Sites/photo.jpg")).unwrap();
    let offer = ask(&home, "Sites/photo.jpg").unwrap();
    assert_eq!(offer.name, "index.html");
    assert_eq!(offer.opens, Opens::Apart);
    assert_eq!(card("i", &mind("Mind"), &offer, "").summary.as_str(), "Mind made index.html");
}

#[test]
fn nothing_that_runs_or_an_app_the_person_chose_is_offered() {
    let (_d, home) = home("kinds");
    for name in ["run.sh", "app.desktop", "thing", "payload.bin"] {
        std::fs::write(home.join("Sites").join(name), "x").unwrap();
        // The Text Editor or nothing: never a launcher, never an exec.
        if let Ok(offer) = ask(&home, &format!("Sites/{name}")) {
            assert!(matches!(offer.opens, Opens::App(ref a) if a == "editor"), "{name}: {:?}", offer.opens);
        }
    }
    let body = include_str!("control_present.rs").split("#[cfg(test)]").next().unwrap();
    assert!(body.contains("FileAction::DesktopApp(id) =>"), "an app the person chose is refused");
    assert!(!body.contains("Command::new") && !body.contains("xdg-open"));
}

#[test]
fn a_press_opens_the_file_that_was_offered_in_the_way_the_card_said() {
    let (_d, home) = home("press");
    let offer = ask(&home, "Sites/index.html").unwrap();
    remember("local:present-file:press", &offer);
    let (path, opens) = what_a_press_opens("local:present-file:press").unwrap();
    assert_eq!((path, opens), (offer.real.clone(), Opens::Apart));
    assert!(what_a_press_opens("local:present-file:never-offered").is_err());
    // HTML, SVG and PDF open apart and offline, tainted or not: the press is consent to view.
    for name in ["a.html", "a.svg", "a.pdf"] {
        assert_eq!(crate::mime_dispatch::classify(name), crate::mime_dispatch::FileAction::Browser);
    }
    let body = include_str!("control_present.rs").split("#[cfg(test)]").next().unwrap();
    assert!(body.contains("Opens::Apart => crate::open_url::open_file_offline(&path)"));
    assert!(!body.contains("find_browser"), "never the person's own browser");
    assert!(include_str!("wire/notifications.rs").contains("crate::control_present::press(&ui, &id, &action_id)"));
}

#[test]
fn a_file_changed_or_swapped_since_the_offer_is_not_opened() {
    let (_d, home) = home("swap");
    std::fs::write(home.join("Sites/a.html"), "a").unwrap();
    let offer = ask(&home, "Sites/a.html").unwrap();
    remember("local:present-file:swap", &offer);
    assert!(what_a_press_opens("local:present-file:swap").is_ok());
    // New contents under the same name: not what was shown.
    std::fs::write(home.join("Sites/a.html"), "a different page, now with a script").unwrap();
    let err = what_a_press_opens("local:present-file:swap").unwrap_err();
    assert!(err.contains("nothing was opened"), "{err}");
    // A link in its place.
    std::fs::remove_file(home.join("Sites/a.html")).unwrap();
    symlink("/etc/hostname", home.join("Sites/a.html")).unwrap();
    assert!(what_a_press_opens("local:present-file:swap").unwrap_err().contains("nothing was opened"));
}

#[test]
fn it_only_asks_it_is_graded_standard_and_moves_no_screen() {
    let spec = action();
    assert_eq!(spec.permission, "standard");
    let body = include_str!("control_present.rs").split("#[cfg(test)]").next().unwrap();
    let code: String = body.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    // `run` raises a toast and nothing else; the only launch is in `press`.
    let run = &code[code.find("pub(crate) fn run").unwrap()..code.find("pub fn press").unwrap()];
    assert!(run.contains("wire::toast::push("));
    for moves in ["set_current_screen", "invoke_", "launch(", "spawn", "ensure_files_screen", "present_app", "windows::", "set_"] {
        assert!(!run.contains(moves), "present_file's handler reaches for `{moves}`");
    }
    // Toasts are the shell's own: no second card system. And no taint check closes the card.
    assert!(!code.contains("Timer") && !code.contains("VecModel::from(vec![ToastData"));
    assert!(!code.contains("taint"), "present_file only asks; a taint does not close it");
    // The press does nothing on the lock screen.
    assert!(code.contains("locked_screen"));
}

#[test]
fn only_a_few_cards_stay_up_and_the_oldest_is_taken_down() {
    let (_d, home) = home("many");
    let offer = ask(&home, "Sites/index.html").unwrap();
    assert_eq!(id_for(&offer.real), id_for(&offer.real));
    let mut list = Vec::new();
    let mut gone = Vec::new();
    for i in 0..(KEPT + 3) {
        gone.extend(remember_in(&mut list, &format!("local:present-file:many{i}"), &offer));
    }
    assert_eq!(gone, ["local:present-file:many0", "local:present-file:many1", "local:present-file:many2"]);
    assert_eq!(list.len(), KEPT);
    // The same file again replaces its own card and takes nobody else down.
    assert!(remember_in(&mut list, &format!("local:present-file:many{}", KEPT + 2), &offer).is_empty());
    assert_eq!(list.len(), KEPT);
}
