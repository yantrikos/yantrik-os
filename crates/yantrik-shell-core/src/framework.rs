//! The apps built on the app framework (`frameworks/yantrik-app`): the one list the shell keeps
//! of them, and what each declares of itself (#942, #978).
//!
//! An app of the framework applies `home_paths` and Private mode itself, so it is the only kind of
//! app an agent may ask to open a file (`agent_open`). Whether a Mind may be given a window of it
//! is the app's own declaration, `App::VIEWABLE`, carried here as `viewable`; the guard test
//! below holds the two together. Under step 1 of design #978 no app is viewable, so every
//! sentence a Mind reads about windows is made from this list and says so for all of them.

pub use yantrik_ipc_contracts::framework_apps::{by_id, by_program, name_of, FrameworkApp, FRAMEWORK_APPS};


/// Whether a Mind may be given a window of the app that runs `program`.
pub fn gives_minds_a_window(program: &str) -> bool {
    by_program(program).is_some_and(|a| a.viewable)
}

/// What a Mind's `open_app` (or a file opened for it) is told of an app of the framework: its
/// display name `name`, and the surface `surface` to describe instead. Word for word.
pub fn refused_for_minds(name: &str, surface: &str) -> String {
    yantrik_ipc_contracts::mind_view::refused_for_minds(name, surface)
}

/// What a Mind's act is told when the app it acts on is still starting: nothing was done, and the
/// act is not kept to run later.
pub fn act_while_starting(name: &str) -> String {
    format!("{name} is starting; nothing was done \u{2014} send it again in a moment.")
}

/// What a Mind's act is told when the app could not be started: nothing was done.
pub fn act_start_failed(name: &str, why: &str) -> String {
    format!("{name} could not be started ({why}); nothing was done.")
}

/// What a Mind's describe is told while the app is starting.
pub fn describe_while_starting(name: &str) -> String {
    format!("{name} is starting; ask again in a moment.")
}

/// What a Mind's describe is told when the app could not be started. The only place that says
/// "not running".
pub fn describe_start_failed(name: &str, why: &str) -> String {
    format!("{name} is not running: the desktop could not start it ({why}).")
}

/// "A, B, C and D".
fn listed(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// `describe shell`'s `mind_view.apps_the_person_has_open`, made from the apps' own declarations:
/// which apps may give a Mind a window (`names` is their display names), and what every other
/// app does instead.
pub fn apps_the_person_has_open(names_of_viewable: &[String]) -> String {
    let instead = "use an app's actions and os_describe: it starts on its own, with no window, \
                   when you ask it something, whether or not the person has it open";
    if names_of_viewable.is_empty() {
        format!("No Yantrik app opens in your view; {instead}.")
    } else {
        format!(
            "{} open in your view; no other Yantrik app does: {instead}.",
            listed(names_of_viewable)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// What each app under `apps/` built on the framework declares in its `impl App`: its program
    /// (its manifest's name), its id, and whether it is viewable.
    fn declared() -> Vec<(String, String, bool)> {
        let apps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps");
        let mut found = Vec::new();
        for dir in std::fs::read_dir(&apps).expect("apps/").flatten() {
            let Ok(files) = std::fs::read_dir(dir.path().join("src")) else { continue };
            for file in files.flatten().filter(|f| f.path().extension().is_some_and(|e| e == "rs")) {
                let text = std::fs::read_to_string(file.path()).unwrap_or_default();
                if !text.contains("impl App for") {
                    continue;
                }
                let id = text
                    .lines()
                    .find(|l| l.contains("const ID: &'static str"))
                    .and_then(|l| l.split('"').nth(1));
                let Some(id) = id else { continue };
                let manifest = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap_or_default();
                let program = manifest.lines().find_map(|l| l.strip_prefix("name = \"")).unwrap_or_default();
                let viewable = text.contains("const VIEWABLE: bool = true;");
                found.push((program.trim_end_matches('"').to_string(), id.to_string(), viewable));
            }
        }
        found.sort();
        found
    }

    /// The one guard: the list is every app of the framework, with the id and the property each
    /// declares. An app moved onto the framework, or one that becomes viewable, changes here or
    /// the build fails.
    #[test]
    fn the_list_is_what_the_apps_declare() {
        let declared = declared();
        assert!(declared.len() >= 10, "the scan found {declared:?}");
        let mut listed: Vec<(String, String, bool)> =
            FRAMEWORK_APPS.iter().map(|a| (a.program.to_string(), a.id.to_string(), a.viewable)).collect();
        listed.sort();
        assert_eq!(listed, declared);
        assert!(FRAMEWORK_APPS.iter().all(|a| a.program.starts_with("yantrik-") && !a.program.contains("mpv")));
    }

    /// Step 1 of #978: no app is viewable, so no Mind has a window of a framework app.
    #[test]
    fn no_app_is_viewable_yet() {
        assert!(FRAMEWORK_APPS.iter().all(|a| !a.viewable));
        assert!(!gives_minds_a_window("yantrik-notes"));
    }

    /// The `.desktop` file of each app names the surface the registry says, and its `Name=` is
    /// what the refusal calls the app.
    #[test]
    fn the_desktop_files_agree_with_the_list() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop-files");
        for a in FRAMEWORK_APPS {
            let text = std::fs::read_to_string(dir.join(format!("{}.desktop", a.program))).expect(a.program);
            let key = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)).unwrap_or_default().to_string();
            assert_eq!(key("X-Yantrik-Surface="), a.id, "{}", a.program);
            assert_eq!(key("Name="), a.name, "{}", a.program);
        }
    }

    #[test]
    fn the_refusal_is_the_sentence_word_for_word() {
        assert_eq!(
            refused_for_minds("Calendar", "calendar"),
            "Calendar cannot open in your view; use its actions and os_describe calendar instead."
        );
        assert_eq!(
            refused_for_minds("Notes", "notes"),
            "Notes cannot open in your view; use its actions and os_describe notes instead."
        );
        assert_eq!(
            refused_for_minds("Image Viewer", "image-viewer"),
            "Image Viewer cannot open in your view; use its actions and os_describe image-viewer instead."
        );
    }

    #[test]
    fn the_start_sentences_are_pinned_word_for_word() {
        assert_eq!(
            act_while_starting("Notes"),
            "Notes is starting; nothing was done \u{2014} send it again in a moment."
        );
        assert_eq!(describe_while_starting("Notes"), "Notes is starting; ask again in a moment.");
        assert!(act_start_failed("Notes", "it exited").contains("nothing was done"));
        assert!(describe_start_failed("Notes", "it exited").contains("not running"));
        assert!(!describe_while_starting("Notes").contains("not running"));
        assert!(!describe_while_starting("Notes").to_lowercase().contains("open it"));
    }

    #[test]
    fn the_describe_sentence_is_made_from_the_list() {
        assert_eq!(
            apps_the_person_has_open(&[]),
            "No Yantrik app opens in your view; use an app's actions and os_describe: it starts on its own, with no window, when you ask it something, whether or not the person has it open."
        );
        let some = apps_the_person_has_open(&["Notes".into(), "Email".into()]);
        assert!(some.starts_with("Notes and Email open in your view;"), "{some}");
    }

    #[test]
    fn an_app_is_found_by_its_program_and_by_nothing_else() {
        assert_eq!(by_program("/opt/yantrik/bin/yantrik-notes").map(|a| a.id), Some("notes"));
        assert_eq!(by_program("/tmp/evil/notes"), None);
        assert_eq!(by_program("foot"), None);
        assert_eq!(by_id("editor").map(|a| a.program), Some("yantrik-text-editor"));
    }
}
