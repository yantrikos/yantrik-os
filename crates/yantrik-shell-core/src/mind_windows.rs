//! Which apps give a Mind a window of its own beside the person's (#963).
//!
//! An app built on the app framework (`frameworks/yantrik-app`) is one instance that owns its
//! data, with a window per display: a Mind's launch from Mind View opens a window of the Mind's
//! there, masked, and the person's window on their desktop is not touched. So the shell may start
//! such an app for a Mind while the person has it open: the launch becomes the Mind's window, not
//! a second window on the person's desk.
//!
//! Every other program (a catalogue app, a terminal, a browser) opens one more window wherever it
//! is started, so a Mind's launch of one already open on the person's desktop is still not made.

use std::path::Path;

/// The framework apps' programs, as installed in `/opt/yantrik/bin`.
pub const OWN_WINDOW_FOR_MINDS: &[&str] = &[
    "yantrik-calendar",
    "yantrik-container-manager",
    "yantrik-download-manager",
    "yantrik-email",
    "yantrik-image-viewer",
    "yantrik-music-player",
    "yantrik-network-manager",
    "yantrik-notes",
    "yantrik-snippet-manager",
    "yantrik-system-monitor",
    "yantrik-text-editor",
    "yantrik-weather",
];

/// Whether `program` (a name or a path to it) gives a Mind a window of its own.
pub fn gives_minds_their_own_window(program: &str) -> bool {
    let name = Path::new(program.trim()).file_name().and_then(|n| n.to_str()).unwrap_or_default();
    OWN_WINDOW_FOR_MINDS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_framework_app_gives_a_mind_its_own_window_and_other_programs_do_not() {
        assert!(gives_minds_their_own_window("yantrik-notes"));
        assert!(gives_minds_their_own_window("/opt/yantrik/bin/yantrik-text-editor"));
        assert!(!gives_minds_their_own_window("foot"));
        assert!(!gives_minds_their_own_window("chromium"));
        assert!(!gives_minds_their_own_window("yantrik-ui"), "the shell itself is not an app of the framework");
        assert!(!gives_minds_their_own_window(""));
        assert!(!gives_minds_their_own_window("/tmp/evil/notes"), "by its program's name, not a word in it");
    }

    /// Every app the workspace builds on the framework is listed, and nothing else: an app moved
    /// onto the framework must be added here, or a Mind is refused its window while the person's
    /// is open.
    #[test]
    fn the_list_is_the_apps_that_run_on_the_framework() {
        let apps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps");
        let mut on_framework: Vec<String> = std::fs::read_dir(&apps)
            .unwrap()
            .flatten()
            .filter(|dir| {
                std::fs::read_to_string(dir.path().join("src/main.rs")).is_ok_and(|main| main.contains("yantrik_app::run::<"))
            })
            .filter_map(|dir| {
                let manifest = std::fs::read_to_string(dir.path().join("Cargo.toml")).ok()?;
                let name = manifest.lines().find_map(|l| l.strip_prefix("name = \""))?;
                Some(name.trim_end_matches('"').to_string())
            })
            .collect();
        on_framework.sort();
        let mut listed: Vec<String> = OWN_WINDOW_FOR_MINDS.iter().map(|s| s.to_string()).collect();
        listed.sort();
        assert_eq!(listed, on_framework);
    }
}
