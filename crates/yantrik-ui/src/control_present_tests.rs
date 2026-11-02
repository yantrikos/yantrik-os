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
    std::fs::write(dir.join(".ssh/id_ed25519"), "key").unwrap();
    let home = dir.canonicalize().unwrap();
    (Home(home.clone()), home)
}

fn ask(home: &Path, path: &str) -> Result<Offer, String> {
    plan(&json!({ "path": home.join(path).to_str().unwrap() }), home)
}

#[test]
fn a_file_in_the_home_is_offered_and_the_card_names_maker_file_and_full_path() {
    let (_d, home) = home("card");
    let offer = ask(&home, "Sites/index.html").unwrap();
    assert_eq!(offer.real, home.join("Sites/index.html"));
    let card = card("local:present-file:x", &Requester::Mind("Yantrik Mind".into()), &offer, "");
    assert_eq!(card.summary.as_str(), "Yantrik Mind made index.html");
    assert_eq!(card.body.as_str(), home.join("Sites/index.html").to_str().unwrap(), "the whole path, uncut");
    let actions: Vec<_> = card.actions.iter().map(|a| a.label.to_string()).collect();
    assert_eq!(actions, ["Open"]);
    let titled = super::card("local:present-file:x", &Requester::Mind("Mind".into()), &offer, "your website");
    assert_eq!(titled.summary.as_str(), "Mind made your website");
    // A name with a line break in it cannot start a line of its own on the card.
    let sly = super::card("i", &Requester::Mind("A\nB".into()), &offer, "");
    assert!(!sly.summary.contains('\n'));
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
fn a_press_opens_the_file_that_was_offered_and_html_goes_to_the_browser() {
    let (_d, home) = home("press");
    let offer = ask(&home, "Sites/index.html").unwrap();
    remember("local:present-file:press", &offer.real);
    assert_eq!(what_a_press_opens("local:present-file:press").unwrap(), offer.real);
    assert!(what_a_press_opens("local:present-file:never-offered").is_err());
    assert_eq!(crate::mime_dispatch::classify("index.html"), crate::mime_dispatch::FileAction::Browser);
    // The press goes through the one launcher a double-click uses.
    let body = include_str!("control_present.rs").split("#[cfg(test)]").next().unwrap();
    assert!(body.contains("wire::open_with::launch(&crate::mime_dispatch::classify("));
    assert!(include_str!("wire/notifications.rs").contains("crate::control_present::press(&ui, &id, &action_id)"));
}

#[test]
fn a_file_swapped_for_a_link_since_the_offer_is_not_opened() {
    let (_d, home) = home("swap");
    std::fs::write(home.join("Sites/a.html"), "a").unwrap();
    let offer = ask(&home, "Sites/a.html").unwrap();
    remember("local:present-file:swap", &offer.real);
    std::fs::remove_file(home.join("Sites/a.html")).unwrap();
    symlink("/etc/hostname", home.join("Sites/a.html")).unwrap();
    let err = what_a_press_opens("local:present-file:swap").unwrap_err();
    assert!(err.contains("nothing was opened"), "{err}");
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
    // Toasts are the shell's own: no second card system.
    assert!(!code.contains("Timer") && !code.contains("VecModel::from(vec![ToastData"));
}

#[test]
fn a_repeat_replaces_its_card_and_only_the_last_few_stay_openable() {
    let (_d, home) = home("many");
    let offer = ask(&home, "Sites/index.html").unwrap();
    assert_eq!(id_for(&offer.real), id_for(&offer.real));
    for i in 0..(KEPT + 3) {
        remember(&format!("local:present-file:many{i}"), &offer.real);
    }
    assert!(what_a_press_opens("local:present-file:many0").is_err());
    assert!(what_a_press_opens(&format!("local:present-file:many{}", KEPT + 2)).is_ok());
}
