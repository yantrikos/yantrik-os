//! The apps built on the app framework (`frameworks/yantrik-app`): the one list of them, shared by
//! the shell (which decides what a Mind may open) and by the framework (which says no in the same
//! words when a Mind's launch reaches an app). A guard test in `yantrik-shell-core` reads every
//! app under `apps/` and its `.desktop` file and fails if this list drifts from what they declare.

/// One app of the framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameworkApp {
    /// Its program and desktop entry id, as installed in `/opt/yantrik/bin`.
    pub program: &'static str,
    /// The id it publishes (`App::ID`): its surface and its instance's name.
    pub id: &'static str,
    /// What the person calls it (the `Name=` of its `.desktop` file), as every refusal says it.
    pub name: &'static str,
    /// A Mind may be given a window of it (`App::VIEWABLE`).
    pub viewable: bool,
}

const fn app(program: &'static str, id: &'static str, name: &'static str) -> FrameworkApp {
    FrameworkApp { program, id, name, viewable: false }
}

/// Every app of the framework. None is viewable under step 1 of design #978.
pub const FRAMEWORK_APPS: &[FrameworkApp] = &[
    app("yantrik-arcade", "arcade", "Arcade"),
    app("yantrik-calendar", "calendar", "Calendar"),
    app("yantrik-container-manager", "containers", "Containers"),
    app("yantrik-document-editor", "documents", "yDoc"),
    app("yantrik-download-manager", "download-manager", "Downloads"),
    app("yantrik-email", "email", "Email"),
    app("yantrik-image-viewer", "image-viewer", "Images"),
    app("yantrik-music-player", "music", "Music"),
    app("yantrik-network-manager", "network", "Network"),
    app("yantrik-notes", "notes", "Notes"),
    app("yantrik-presentation", "presentation", "yPresent"),
    app("yantrik-snippet-manager", "snippets", "Snippets"),
    app("yantrik-spreadsheet", "spreadsheet", "ySheets"),
    app("yantrik-studio", "studio", "Studio"),
    app("yantrik-system-monitor", "system-monitor", "System Monitor"),
    app("yantrik-terminal", "terminal", "Terminal"),
    app("yantrik-text-editor", "editor", "Editor"),
    app("yantrik-weather", "weather", "Weather"),
];

/// The app of the framework that `program` (a name, or a path to it) is.
pub fn by_program(program: &str) -> Option<&'static FrameworkApp> {
    let name = std::path::Path::new(program.trim()).file_name().and_then(|n| n.to_str())?;
    FRAMEWORK_APPS.iter().find(|a| a.program == name)
}

/// The app of the framework whose id is `id`, as its surface and its socket name it.
pub fn by_id(id: &str) -> Option<&'static FrameworkApp> {
    FRAMEWORK_APPS.iter().find(|a| a.id == id)
}

/// What a Mind's describe is told while the app is starting. Word for word; the apps that say it
/// themselves (Email's first describe) build it from here.
pub fn describe_while_starting(name: &str) -> String {
    format!("{name} is starting; ask again in a moment.")
}

/// What the person calls the app with surface id `id`; the id itself for one not in the list.
pub fn name_of(id: &str) -> &str {
    by_id(id).map_or(id, |a| a.name)
}
