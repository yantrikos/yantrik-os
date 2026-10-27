//! The apps built on the app framework (`frameworks/yantrik-app`): the one list the shell keeps
//! of them, and what each declares of itself (#942, #963, #972).
//!
//! An app of the framework applies `home_paths` and Private mode itself, so it is the only kind of
//! app an agent may open a file in (`agent_open`). It is one instance with a window per display, so
//! a Mind's launch can open a window of the Mind's beside the person's. It may do that while the
//! person has the app open only if it **masks for a Mind**: it hides the person's content in a
//! Mind's window. That is the app's own declaration, `App::MASKS_FOR_MIND`, and the framework
//! enforces it (`Core::present_for_mind`). `masks_for_minds` here carries the same declaration to
//! the shell, and the guard test below holds the two together, so no sentence or route of the
//! shell can say an app masks when the app does not.

use std::path::Path;

/// One app of the framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameworkApp {
    /// Its program and desktop entry id, as installed in `/opt/yantrik/bin`.
    pub program: &'static str,
    /// The id it publishes (`App::ID`): its surface and its instance's name.
    pub id: &'static str,
    /// It masks the person's content in a Mind's window (`App::MASKS_FOR_MIND`).
    pub masks_for_minds: bool,
}

const fn app(program: &'static str, id: &'static str, masks_for_minds: bool) -> FrameworkApp {
    FrameworkApp { program, id, masks_for_minds }
}

/// Every app of the framework.
pub const FRAMEWORK_APPS: &[FrameworkApp] = &[
    app("yantrik-calendar", "calendar", false),
    app("yantrik-container-manager", "containers", false),
    app("yantrik-download-manager", "download-manager", false),
    app("yantrik-email", "email", true),
    app("yantrik-image-viewer", "image-viewer", false),
    app("yantrik-music-player", "music", false),
    app("yantrik-network-manager", "network", false),
    app("yantrik-notes", "notes", true),
    app("yantrik-snippet-manager", "snippets", true),
    app("yantrik-system-monitor", "system-monitor", false),
    app("yantrik-text-editor", "editor", true),
    app("yantrik-weather", "weather", false),
];

/// The app of the framework that `program` (a name, or a path to it) is.
pub fn by_program(program: &str) -> Option<&'static FrameworkApp> {
    let name = Path::new(program.trim()).file_name().and_then(|n| n.to_str())?;
    FRAMEWORK_APPS.iter().find(|a| a.program == name)
}

/// The app of the framework whose id is `id`.
pub fn by_id(id: &str) -> Option<&'static FrameworkApp> {
    FRAMEWORK_APPS.iter().find(|a| a.id == id)
}

/// Whether `program` gives a Mind a window of its own while the person has it open.
pub fn gives_minds_their_own_window(program: &str) -> bool {
    by_program(program).is_some_and(|a| a.masks_for_minds)
}

/// The apps that mask for a Mind.
pub fn masking() -> impl Iterator<Item = &'static FrameworkApp> {
    FRAMEWORK_APPS.iter().filter(|a| a.masks_for_minds)
}

/// "A, B, C and D".
fn listed(names: &[String]) -> String {
    match names {
        [] => "No app".to_string(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// `describe shell`'s `mind_view.apps_the_person_has_open`, made from the apps' own declarations:
/// which apps give a Mind a window beside the person's, by `name` (the shell's display name of an
/// app id), and what every other app does instead.
pub fn beside_the_person(name: impl Fn(&str) -> String) -> String {
    let names: Vec<String> = masking().map(|a| name(a.id)).collect();
    format!(
        "{} open a window of yours in Mind View even while the person has the app open: one app, \
         with the person's window on their desktop and yours, masked, in Mind View; its calls \
         reach the person's data either way. Every other app cannot be opened in your view while \
         the person has it open: use its actions and os_describe instead.",
        listed(&names)
    )
}

/// What a Mind's `open_app` is told of an app the person has open that cannot also open in the
/// Mind's view: the app's display name `name`, and its surface `surface` to describe instead.
pub fn refused_beside_the_person(name: &str, surface: &str) -> String {
    format!(
        "{name} is open on the person's screen and cannot also open in your view; use its actions and \
         os_describe {surface} instead."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What each app under `apps/` built on the framework declares in its `impl App`: its program
    /// (its manifest's name), its id, and whether it masks for a Mind.
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
                let masks = text.contains("const MASKS_FOR_MIND: bool = true;");
                found.push((program.trim_end_matches('"').to_string(), id.to_string(), masks));
            }
        }
        found.sort();
        found
    }

    /// The one guard: the list is every app of the framework, with the id and the masking each
    /// declares. An app moved onto the framework, or one that learns to mask, changes here or the
    /// build fails.
    #[test]
    fn the_list_is_what_the_apps_declare() {
        let declared = declared();
        assert!(declared.len() >= 10, "the scan found {declared:?}");
        let mut listed: Vec<(String, String, bool)> = FRAMEWORK_APPS
            .iter()
            .map(|a| (a.program.to_string(), a.id.to_string(), a.masks_for_minds))
            .collect();
        listed.sort();
        assert_eq!(listed, declared);
        assert!(FRAMEWORK_APPS.iter().all(|a| a.program.starts_with("yantrik-") && !a.program.contains("mpv")));
    }

    /// The sentence a Mind reads and the route the shell takes say the same thing.
    #[test]
    fn the_describe_sentence_names_exactly_the_apps_that_mask() {
        let name = |id: &str| format!("<{id}>");
        let said = beside_the_person(name);
        for a in FRAMEWORK_APPS {
            assert_eq!(said.contains(&name(a.id)), a.masks_for_minds, "{}: {said}", a.id);
            assert_eq!(gives_minds_their_own_window(a.program), a.masks_for_minds, "{}", a.program);
        }
        assert!(said.starts_with("<email>, <notes>, <snippets> and <editor> open a window of yours"), "{said}");
        assert!(said.contains("Every other app cannot be opened in your view while the person has it open"));
    }

    /// The golden dump of `describe shell` `mind_view.apps_the_person_has_open` at this head, with
    /// the shell's display names (`yantrik-ui-kit` `APP_NAMES`) for the four apps that mask.
    #[test]
    fn the_sentence_as_describe_shell_says_it() {
        let shell_name = |id: &str| match id {
            "email" => "Email".to_string(),
            "notes" => "Notes".to_string(),
            "snippets" => "Snippets".to_string(),
            "editor" => "Editor".to_string(),
            other => panic!("{other} does not mask, so it is not named"),
        };
        assert_eq!(
            beside_the_person(shell_name),
            "Email, Notes, Snippets and Editor open a window of yours in Mind View even while the person has the app open: one app, with the person's window on their desktop and yours, masked, in Mind View; its calls reach the person's data either way. Every other app cannot be opened in your view while the person has it open: use its actions and os_describe instead."
        );
    }

    #[test]
    fn the_refusal_gives_the_next_step_word_for_word() {
        assert_eq!(
            refused_beside_the_person("Calendar", "calendar"),
            "Calendar is open on the person's screen and cannot also open in your view; use its actions and os_describe calendar instead."
        );
    }

    #[test]
    fn an_app_is_found_by_its_program_and_by_nothing_else() {
        assert_eq!(by_program("/opt/yantrik/bin/yantrik-notes").map(|a| a.id), Some("notes"));
        assert_eq!(by_program("/tmp/evil/notes"), None);
        assert_eq!(by_program("foot"), None);
        assert_eq!(by_id("editor").map(|a| a.program), Some("yantrik-text-editor"));
        assert!(!gives_minds_their_own_window("yantrik-calendar"), "Calendar does not mask yet");
        assert!(!gives_minds_their_own_window("yantrik-ui"));
    }
}
