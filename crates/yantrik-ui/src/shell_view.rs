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
        // (file, a token that moves the person's view, how many handlers or helpers contain it
        // unguarded on purpose, and why). The count is exact, so a new mover must either ask or be
        // argued for here.
        const EXEMPT: &[(&str, &str, &str)] = &[
            ("control_installer.rs", "set_current_screen(", "the first-run wizard is drivable by an agent by design, before any person has a view"),
            ("control_files.rs", "set_current_screen(", "ensure_files_screen itself; every handler intercepts first (files_actions_all_intercept)"),
            ("control_files.rs", "ensure_files_screen(", "the handlers intercept first (files_actions_all_intercept)"),
            ("control_approvals.rs", "raise_shell(", "the approval card is the person's own decision surface"),
            ("control_switcher.rs", "raise_shell(", "reached only once `open` has asked; open_on_worker has no caller in scope"),
        ];
        let files = [
            ("control.rs", include_str!("control.rs")),
            ("control_agents.rs", include_str!("control_agents.rs")),
            ("control_workroom.rs", include_str!("control_workroom.rs")),
            ("control_overlays.rs", include_str!("control_overlays.rs")),
            ("control_switcher.rs", include_str!("control_switcher.rs")),
            ("control_files.rs", include_str!("control_files.rs")),
            ("control_files_make.rs", include_str!("control_files_make.rs")),
            ("control_installer.rs", include_str!("control_installer.rs")),
            ("control_approvals.rs", include_str!("control_approvals.rs")),
        ];
        let movers = ["set_current_screen(", "raise_shell(", "ensure_files_screen(", "invoke_launch_app(", "show_desktop::press("];
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
                    if code.contains("fn ensure_files_screen") || EXEMPT.iter().any(|(f, m, _)| *f == file && *m == mover) {
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
