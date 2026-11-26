//! Small pure rules of the window list that do not belong in `windows.rs` (#1419).

/// Whether a declared app id is the browser the dock's Browser entry opens: Debian's `chromium`
/// or Ubuntu's `chromium-browser`. Without this the window listed under its own id, and the dock
/// drew a second generic "Chromium Web Browser" icon beside the pinned Browser, which stayed dark.
/// wlrctl is still handed the declared id (`WindowEntry::wayland_app_id`); only the shell's id for
/// the window changes.
pub(crate) fn is_browser_id(declared_id: &str) -> bool {
    matches!(declared_id.to_lowercase().as_str(), "chromium" | "chromium-browser")
}

/// What is left to do after asking for focus on a window being restored.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RestoreStep {
    Done,
    Maximize,
}

/// A window the compositor still lists as minimized after the focus request needs the fallback:
/// `maximize` is the one verb wlrctl has that un-minimizes, and it costs the window its size.
pub(crate) fn restore_plan(still_minimized: bool) -> RestoreStep {
    if still_minimized { RestoreStep::Maximize } else { RestoreStep::Done }
}

/// `wlrctl toplevel find <spec> state:minimized`: succeeds when that window is minimized.
pub(crate) fn minimized_probe(matchspec: &str) -> Vec<String> {
    ["toplevel", "find", matchspec, "state:minimized"].map(String::from).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows::shell_app_id;

    #[test]
    fn both_debian_and_ubuntu_chromium_are_the_browser_entry() {
        for declared in ["chromium", "chromium-browser", "Chromium"] {
            assert_eq!(shell_app_id(declared, "Ask | Hacker News - Chromium"), "browser", "{declared}");
        }
        assert_ne!(shell_app_id("chrome", "x"), "browser", "only the browser the dock opens");
    }

    #[test]
    fn restore_asks_for_focus_first_and_resizes_only_when_that_left_it_minimized() {
        assert_eq!(restore_plan(false), RestoreStep::Done, "focus un-minimizes and keeps the size");
        assert_eq!(restore_plan(true), RestoreStep::Maximize, "the old way, only as the fallback");
    }

    #[test]
    fn the_probe_asks_for_minimized_windows_only() {
        assert_eq!(minimized_probe("title:Notes"), ["toplevel", "find", "title:Notes", "state:minimized"]);
    }
}
