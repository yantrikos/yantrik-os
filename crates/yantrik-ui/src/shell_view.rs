//! The person's view is the person's: a Mind never moves it (#1033).
//!
//! A Mind asked `os_describe files` and was told "open it first"; it did, `open_app files`
//! switched the person's shell to Files, and the person lost the screen they were on. Everything
//! that changes `current-screen`, opens the launcher or an overlay over their work, or raises
//! the shell over a window they are using is the person's alone. A caller that `requester_now`
//! calls a Mind (an attached harness, an agent with a token, the built-in companion) is refused
//! with a plain sentence, before anything is applied, and uses the shell's actions instead
//! (`files_*` take a path and never touch the screen). The person's own calls are unchanged.
//!
//! One check, [`refuse_for_mind`], is called first in every handler that moves the view. The
//! `every_view_mover_asks_first` test reads the sources, so a new such action cannot skip it.

use crate::mind_view::{requester_now, Requester};

/// Refuse the call being dispatched on this thread when a Mind makes it. `what` is the thing it
/// would show: a screen name (`files`, `settings`), or a word for an overlay (`the launcher`).
pub(crate) fn refuse_for_mind(what: &str) -> Result<(), String> {
    refuse_for(&requester_now(), what)
}

/// [`refuse_for_mind`] once the requester is known.
pub(crate) fn refuse_for(who: &Requester, what: &str) -> Result<(), String> {
    match who {
        Requester::Person => Ok(()),
        Requester::Mind(_) => Err(sentence(what)),
    }
}

/// The refusal, in plain words. Files has actions of its own to point to; for the rest the shell's
/// `describe` is the way to look and its actions the way to act.
pub(crate) fn sentence(what: &str) -> String {
    if what.eq_ignore_ascii_case("files") {
        return "Files can't open in your view; use the shell's files_* actions.".to_string();
    }
    let mut name = what.to_string();
    if let Some(first) = name.get(..1) {
        name = first.to_uppercase() + &name[1..];
    }
    format!("{name} can't open in your view; use os_describe shell and the shell's actions instead.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mind() -> Requester {
        Requester::Mind("a mind".to_string())
    }

    #[test]
    fn a_mind_is_refused_and_the_person_is_not() {
        for what in ["files", "settings", "memory", "system", "the launcher", "desktop"] {
            assert_eq!(refuse_for(&mind(), what), Err(sentence(what)), "{what}");
            assert_eq!(refuse_for(&Requester::Person, what), Ok(()), "{what}");
        }
    }

    #[test]
    fn the_sentences_are_plain() {
        assert_eq!(sentence("files"), "Files can't open in your view; use the shell's files_* actions.");
        assert_eq!(
            sentence("settings"),
            "Settings can't open in your view; use os_describe shell and the shell's actions instead."
        );
        for what in ["files", "settings", "the launcher"] {
            let said = sentence(what);
            assert!(!said.contains("not running") && !said.to_lowercase().contains("open it first"), "{said}");
        }
    }

    /// Where a Mind is told nothing needs opening: no sentence of the shell's sends it to
    /// `open_app` for a screen.
    #[test]
    fn every_view_mover_asks_first() {
        // (file, a token that moves the person's view, the function it is allowed in or "" for the
        // file, and why). A new mover must either ask or be argued for here.
        const EXEMPT: &[(&str, &str, &str, &str)] = &[
            ("control_installer.rs", "set_current_screen(", "", "the first-run wizard is drivable by an agent by design, before any person has a view"),
            ("control_installer.rs", "invoke_navigate(", "", "the same wizard"),
            ("control_files.rs", "set_current_screen(", "", "ensure_files_screen itself; every handler intercepts first (files_actions_all_intercept)"),
            ("control_files.rs", "ensure_files_screen(", "", "the handlers intercept first (files_actions_all_intercept)"),
            ("control_approvals.rs", "raise_shell(", "", "the approval card is the person's own decision surface"),
            ("control_switcher.rs", "raise_shell(", "fn open_on_worker", "reached only once `open` has asked; the worker has no caller in scope"),
            ("control_switcher.rs", "raise_shell(", "fn activate_cell", "called by `commit`, which asks first (switcher_entry_points_ask_first)"),
            ("control_switcher.rs", "toplevel_watch::activate(", "fn activate_cell", "called by `commit`, which asks first"),
            ("control_switcher.rs", "toplevel_watch::activate(", "fn activate_origin", "called by `cancel`, which asks first"),
            ("control_switcher.rs", "windows::present(", "fn activate_cell", "called by `commit`, which asks first"),
            ("control_switcher.rs", "windows::present(", "fn activate_origin", "called by `cancel`, which asks first"),
        ];
        // Every `control*.rs`: `every_control_file_is_scanned` keeps this list equal to the directory.
        let files = [
            ("control.rs", include_str!("control.rs")),
            ("control_agent_terminal.rs", include_str!("control_agent_terminal.rs")),
            ("control_agents.rs", include_str!("control_agents.rs")),
            ("control_approvals.rs", include_str!("control_approvals.rs")),
            ("control_capture.rs", include_str!("control_capture.rs")),
            ("control_chat.rs", include_str!("control_chat.rs")),
            ("control_decide.rs", include_str!("control_decide.rs")),
            ("control_files.rs", include_str!("control_files.rs")),
            ("control_files_create.rs", include_str!("control_files_create.rs")),
            ("control_files_cursor.rs", include_str!("control_files_cursor.rs")),
            ("control_files_make.rs", include_str!("control_files_make.rs")),
            ("control_files_mind.rs", include_str!("control_files_mind.rs")),
            ("control_files_mind_tests.rs", include_str!("control_files_mind_tests.rs")),
            ("control_files_paste.rs", include_str!("control_files_paste.rs")),
            ("control_installer.rs", include_str!("control_installer.rs")),
            ("control_installer_consent.rs", include_str!("control_installer_consent.rs")),
            ("control_installer_target.rs", include_str!("control_installer_target.rs")),
            ("control_levels.rs", include_str!("control_levels.rs")),
            ("control_levels_actions.rs", include_str!("control_levels_actions.rs")),
            ("control_network.rs", include_str!("control_network.rs")),
            ("control_overlays.rs", include_str!("control_overlays.rs")),
            ("control_power.rs", include_str!("control_power.rs")),
            ("control_recipes.rs", include_str!("control_recipes.rs")),
            ("control_screen.rs", include_str!("control_screen.rs")),
            ("control_settings.rs", include_str!("control_settings.rs")),
            ("control_switcher.rs", include_str!("control_switcher.rs")),
            ("control_taint_scope.rs", include_str!("control_taint_scope.rs")),
            ("control_theme.rs", include_str!("control_theme.rs")),
            ("control_update.rs", include_str!("control_update.rs")),
            ("control_wifi_reconnect.rs", include_str!("control_wifi_reconnect.rs")),
            ("control_workroom.rs", include_str!("control_workroom.rs")),
        ];
        let movers = [
            "set_current_screen(",
            "raise_shell(",
            "ensure_files_screen(",
            "invoke_launch_app(",
            "show_desktop::press(",
            "invoke_switch_window(",
            "invoke_navigate(",
            "set_lens_open(true",
            "present_app(",
            "windows::present(",
            "toplevel_watch::activate(",
        ];
        let mut unguarded = Vec::new();
        for (file, src) in files {
            let src = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
            // A chunk is an action or a function: it starts at `Action::new(` or a line `fn `.
            let mut starts: Vec<usize> = src.match_indices("Action::new(").map(|(i, _)| i).collect();
            starts.extend(src.match_indices("\nfn ").map(|(i, _)| i));
            starts.extend(src.match_indices("\npub fn ").map(|(i, _)| i));
            starts.extend(src.match_indices("\npub(crate) fn ").map(|(i, _)| i));
            starts.sort_unstable();
            starts.push(src.len());
            for pair in starts.windows(2) {
                let chunk = &src[pair[0]..pair[1]];
                let code: String = chunk.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
                for mover in movers {
                    let Some(at) = code.find(mover) else { continue };
                    if code.contains("fn ensure_files_screen") || EXEMPT.iter().any(|(f, m, in_fn, _)| *f == file && *m == mover && chunk.trim_start().starts_with(in_fn)) {
                        continue;
                    }
                    let asked = ["refuse_for_mind(", "files_cursor::intercept(", "cursor::caller_key("]
                        .iter()
                        .filter_map(|ask| code.find(ask))
                        .min();
                    if !asked.is_some_and(|a| a < at) {
                        let head: String = chunk.trim_start().chars().take(70).collect();
                        unguarded.push(format!("{file}: `{mover}` in `{head}`"));
                    }
                }
            }
        }
        assert!(
            unguarded.is_empty(),
            "an action that moves the person's view must call `crate::shell_view::refuse_for_mind` \
             before it does (#1033):\n  {}",
            unguarded.join("\n  ")
        );
    }

    /// Every `files_*` action asks the cursor module before it looks at the screen, or is one that
    /// never does. A new `files_*` action is refused for a Mind until it is added to `act` there.
    #[test]
    fn files_actions_all_intercept() {
        let src = include_str!("control_files.rs");
        let src = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
        // Read-only or screen-free for everyone: they never touch the person's screen.
        const SCREEN_FREE: &[&str] = &["files_stat", "files_list", "files_new_folder", "files_new_file"];
        let mut seen = Vec::new();
        for (at, _) in src.match_indices("\"files_") {
            let rest = &src[at + 1..];
            let name = &rest[..rest.find('"').unwrap()];
            if !seen.contains(&name.to_string()) {
                seen.push(name.to_string());
            }
        }
        assert!(seen.len() >= 18, "the files_* actions were found: {seen:?}");
        for name in &seen {
            if SCREEN_FREE.contains(&name.as_str()) {
                continue;
            }
            let literal = format!("\"{name}\"");
            let by_macro = src.match_indices(&literal).any(|(i, _)| {
                src[i.saturating_sub(40)..i].split_whitespace().last() == Some("surface,")
            });
            let intercepted = src.contains(&format!("intercept(\"{name}\"")) || by_macro;
            assert!(intercepted, "{name} must ask files_cursor::intercept before it touches the screen");
        }
        // The macro that builds the screen-state actions asks too, before it reads the screen.
        let mac = &src[src.find("macro_rules! action").unwrap()..];
        assert!(mac.find("intercept($name").unwrap() < mac.find("mind::here(").unwrap());
    }

    /// The scan above reads a list of files; a new `control_*.rs` cannot be left out of it.
    #[test]
    fn every_control_file_is_scanned() {
        let listed: Vec<String> = include_str!("shell_view.rs")
            .lines()
            .filter(|l| l.contains("include_str!"))
            .filter_map(|l| l.trim().strip_prefix("(\"control"))
            .filter_map(|l| l.split('"').next())
            .filter(|n| n.ends_with(".rs"))
            .map(|n| format!("control{n}"))
            .collect();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut on_disk: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.starts_with("control") && n.ends_with(".rs"))
            .collect();
        on_disk.sort_unstable();
        let mut listed = listed;
        listed.sort_unstable();
        listed.dedup();
        assert_eq!(listed, on_disk, "every control*.rs is in `every_view_mover_asks_first`'s list");
    }

    /// The switcher's own actions move the person's selection or bring a window forward; the
    /// functions the person's keys call are not gated (their callers have no requester), so the
    /// actions a Mind reaches are: each asks before it does anything.
    #[test]
    fn switcher_entry_points_ask_first() {
        let src = include_str!("control_switcher.rs");
        let src = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
        for action in ["switcher_move", "switcher_commit", "switcher_cancel"] {
            let at = src.find(&format!("
                \"{action}\",
")).unwrap_or_else(|| panic!("{action} is published"));
            let rest = &src[at..];
            let body = &rest[rest.find("move |").unwrap()..];
            let body = &body[..body.find("let ui = ").unwrap()];
            assert!(body.contains("refuse_for_mind("), "{action} must ask before it takes the shell or acts");
        }
        let open = &src[src.find("pub fn open(").unwrap()..];
        assert!(open.find("refuse_for_mind(").unwrap() < open.find("hold_windows(").unwrap());
    }

    /// `describe` of any of the shell's own screens is answered by `yos` as "part of the shell",
    /// never "not running": the list it keeps is the shell's `SCREENS` and the launcher.
    #[test]
    fn describe_names_every_shell_screen_as_part_of_the_shell() {
        let control = include_str!("control.rs");
        let table = &control[control.find("const SCREENS: &[(&str, i32)]").unwrap()..];
        let table = &table[..table.find("];").unwrap()];
        let mut screens: Vec<&str> = table
            .lines()
            .filter_map(|l| l.trim().strip_prefix("(\""))
            .filter_map(|l| l.split('"').next())
            .collect();
        screens.push("launchpad");
        let yos = include_str!("../../../deploy/yantrik-os/yos");
        let listed = &yos[yos.find("SHELL_SCREENS = (").unwrap()..];
        let listed = &listed[..listed.find(')').unwrap()];
        let mut kept: Vec<&str> = listed.split('"').skip(1).step_by(2).collect();
        screens.sort_unstable();
        kept.sort_unstable();
        assert_eq!(kept, screens, "yos's SHELL_SCREENS must be the shell's screens (control.rs SCREENS) and launchpad");
        // The branch that answers it comes before the one that says a declared app is closed.
        assert!(yos.find("raise Unreachable(in_shell)").unwrap() < yos.find("raise Unreachable(closed_text(name, surface").unwrap());
    }
}
