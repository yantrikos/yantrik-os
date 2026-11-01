//! A Mind closes the apps it opened in Mind View, and only those (#1033).
//!
//! `close_window` resolves a title against the person's desktop first, so a Mind that closed
//! "its" browser could close theirs. For a Mind it now reads only Mind View's own compositor, by
//! what the Mind can know: the app's registry name (`browser`), the app id the window declared
//! (`chromium`), or the window's title as it is now (`New Tab - Chromium`, which the app changes
//! as it runs). The close is sent to Mind View's display, so a window of the person's cannot be
//! reached, whatever it is titled.

use std::process::Command;
use std::time::Duration;

use serde_json::{json, Value};

use crate::windows::toplevel_entry;

/// Work out which window of Mind View `want` names, and how to say so to its compositor
/// (`title:…` or `app_id:…`), with the window's title. `lines` are `wlrctl toplevel list` of Mind
/// View's display; `registry` is the id and program of the app of that name in Mind View's own list, if any (a window
/// of the program answers to the id: Chromium's declares `chromium`, the app is `browser`).
pub(crate) fn resolve(lines: &[String], want: &str, registry: Option<(&str, &str)>) -> Result<(String, String), String> {
    let want_l = want.trim().to_lowercase();
    if want_l.is_empty() {
        return Err("`title` is empty".into());
    }
    let windows: Vec<_> = lines.iter().map(|l| toplevel_entry(l)).collect();
    let by_title: Vec<_> = windows.iter().filter(|w| w.title.to_lowercase() == want_l).collect();
    let exact = !by_title.is_empty();
    let found = if exact {
        by_title
    } else {
        windows
            .iter()
            .filter(|w| {
                w.app_id == want_l
                    || (!w.wayland_app_id.is_empty() && w.wayland_app_id.to_lowercase() == want_l)
                    || registry.is_some_and(|(id, binary)| {
                        w.app_id == id.to_lowercase() || crate::windows::same_program(binary, &w.wayland_app_id)
                    })
            })
            .collect()
    };
    match found.as_slice() {
        [] => {
            let shown = windows.iter().map(|w| w.title.clone()).collect::<Vec<_>>().join(", ");
            Err(format!(
                "`{}` is not an app in Mind View. Windows on the person's desktop can't be closed by a Mind; \
                 Mind View shows: {}",
                want.trim(),
                if shown.is_empty() { "nothing" } else { &shown }
            ))
        }
        [w] => {
            // wlrctl closes every window its matchspec matches, and it can name a window only by
            // title or app id. Use the one of those that is this window's alone.
            let mut candidates = Vec::new();
            if !exact && !w.wayland_app_id.is_empty() {
                candidates.push(format!("app_id:{}", w.wayland_app_id));
            }
            candidates.push(format!("title:{}", w.title));
            let alone = |selector: &str| windows.iter().filter(|o| selects(selector, o)).count() == 1;
            let selector = candidates.into_iter().find(|s| alone(s));
            selector.map(|s| (s, w.title.clone())).ok_or_else(|| {
                format!(
                    "`{}` is one window, but another in Mind View has the same title and app id, and a close \
                     cannot tell them apart; close it from Mind View itself",
                    want.trim()
                )
            })
        }
        many => Err(format!(
            "`{}` matches {} windows in Mind View and closing the wrong one cannot be undone; say which: {}",
            want.trim(),
            many.len(),
            many.iter().map(|w| w.title.as_str()).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// Whether `wlrctl toplevel close <selector>` would close `window`: its matchspec is compared
/// exactly and case-sensitively.
fn selects(selector: &str, window: &crate::windows::WindowEntry) -> bool {
    match selector.split_once(':') {
        Some(("title", t)) => window.title == t,
        Some(("app_id", id)) => window.wayland_app_id == id,
        _ => false,
    }
}

/// `wlrctl` aimed at Mind View's display and nowhere else: `WAYLAND_SOCKET`, which libwayland
/// prefers over `WAYLAND_DISPLAY`, is removed so the person's compositor cannot be reached through
/// an inherited connection.
pub(crate) fn wlrctl(display: &str, args: &[String]) -> Command {
    let mut cmd = Command::new("wlrctl");
    cmd.args(args).env("WAYLAND_DISPLAY", display).env_remove("WAYLAND_SOCKET");
    cmd
}

/// Ask the window of Mind View that `want` names to close, as its × does.
pub(crate) fn close(want: &str) -> Result<Value, String> {
    let lines = crate::mind_view::nested_window_lines()
        .ok_or("Mind View is not running, so nothing is drawn there to close")?;
    let app = crate::mind_view::app_named(want);
    let binary = app
        .as_ref()
        .and_then(|id| crate::running::running().into_iter().find(|a| &a.app_id == id))
        .map(|a| a.binary)
        .unwrap_or_default();
    let (selector, title) = resolve(&lines, want, app.as_deref().map(|id| (id, binary.as_str())))?;
    let name = close_matching(selector, title)?;
    Ok(json!({
        "closing": name,
        "where": "Mind View",
        "note": "the app was drawn in Mind View, where the apps a mind opens go, and was asked to close \
                 there as pressing × does; one with unsaved work may put up its own dialog and stay. \
                 Read `mind_view` in `describe shell` to see whether it went.",
    }))
}

/// Ask an app drawing in Mind View to close, as its × does: on Mind View's own display, so an app
/// with unsaved work can still put up its dialog. Returns the name it was asked by.
pub fn close_app(app_id: &str) -> Result<String, String> {
    let title = crate::windows::app_display_name(app_id);
    close_matching(format!("title:{title}"), title)
}

/// Ask the window of Mind View's compositor that `selector` (`title:…`, `app_id:…`) names to close.
/// Returns `title`, what the caller calls it.
pub(crate) fn close_matching(selector: String, title: String) -> Result<String, String> {
    let display = crate::mind_view::display_now()
        .ok_or_else(|| "Mind View is not running, so nothing is drawn there to close".to_string())?;
    let args = ["toplevel".to_string(), "close".to_string(), selector];
    let (answer, wait) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("yos-wlrctl-mind-view".to_string())
        .spawn(move || {
            let _ = answer.send(wlrctl(&display, &args).status());
        })
        .map_err(|_| "could not start a thread to talk to Mind View's compositor".to_string())?;
    match wait.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(status)) if status.success() => Ok(title),
        Ok(Ok(_)) => Err(format!("Mind View's compositor has no window called `{title}` to close")),
        Ok(Err(e)) => Err(format!("could not run wlrctl: {e}")),
        Err(_) => Err("Mind View's compositor did not answer within 2 s".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines() -> Vec<String> {
        vec!["chromium: New Tab - Chromium".to_string(), "foot: build log".to_string()]
    }

    #[test]
    fn a_mind_closes_its_app_by_the_registry_name() {
        // `app_named("browser")` answered `browser`; the window declared `chromium`.
        let (selector, title) = resolve(&lines(), "Browser", Some(("browser", "/usr/bin/chromium"))).unwrap();
        assert_eq!((selector.as_str(), title.as_str()), ("app_id:chromium", "New Tab - Chromium"));
    }

    #[test]
    fn a_mind_closes_its_app_by_its_title_as_it_is_now() {
        let (selector, title) = resolve(&lines(), "new tab - chromium", None).unwrap();
        assert_eq!((selector.as_str(), title.as_str()), ("title:New Tab - Chromium", "New Tab - Chromium"));
        // By the declared app id too.
        assert_eq!(resolve(&lines(), "chromium", None).unwrap().1, "New Tab - Chromium");
    }

    /// Only Mind View's list is read, so a window of the person's with the same title is not a
    /// candidate, and what is sent goes to Mind View's own display.
    #[test]
    fn a_same_titled_person_window_is_never_reached() {
        let src = include_str!("mind_view_close.rs");
        let src = &src[..src.find("#[cfg(test)]").unwrap()];
        for token in ["addressable_titles", "window_to_close", "windows::close("] {
            assert!(!src.contains(token), "the Mind's close must not reach the person's desktop: `{token}`");
        }
        assert!(src.contains("nested_window_lines()") && src.contains("close_matching("));
        assert!(src.contains("display_now()") && src.contains("wlrctl(&display, &args)"), "sent to Mind View's display");
        // A Mind's close_window returns here before it reads the person's windows.
        let ctl = include_str!("control.rs");
        let close = &ctl[ctl.find("\"close_window\",").unwrap()..];
        let close = &close[..close.find("windows::close(").unwrap()];
        let mind = close.find("mind_view_close::close(").unwrap();
        assert!(close.find("asked_by_mind()").unwrap() < mind);
        assert!(mind < close.find("addressable_titles()").unwrap());
        // With the person's window absent from Mind View's list, nothing matches its title.
        assert!(resolve(&["foot: build log".to_string()], "New Tab - Chromium", Some(("browser", "/usr/bin/chromium"))).is_err());
    }

    /// wlrctl closes every window its matchspec matches, so two identical lines, by title or by app
    /// id, are refused with the list and nothing is sent.
    #[test]
    fn two_identical_windows_are_refused_not_both_closed() {
        let twice = vec!["chromium: New Tab - Chromium".to_string(), "chromium: New Tab - Chromium".to_string()];
        for want in ["New Tab - Chromium", "chromium", "browser"] {
            let said = resolve(&twice, want, Some(("browser", "/usr/bin/chromium"))).unwrap_err();
            assert!(said.contains("matches 2 windows") && said.contains("New Tab - Chromium"), "{want}: {said}");
        }
        // One window by title, but its app id is shared with a window of another title: the title
        // is the selector that is its alone; a window with neither alone is refused.
        let mixed = vec!["chromium: A".to_string(), "chromium: B".to_string()];
        assert_eq!(resolve(&mixed, "A", None).unwrap().0, "title:A");
        let same_app_by_registry = vec!["chromium: A".to_string(), "chromium: B".to_string()];
        assert!(resolve(&same_app_by_registry, "chromium", None).unwrap_err().contains("matches 2 windows"));
    }

    /// The command is aimed at Mind View's display only: `WAYLAND_SOCKET` is removed (libwayland
    /// prefers it) for the close and for the list, which share this builder.
    #[test]
    fn wlrctl_is_aimed_at_mind_views_display_and_cannot_inherit_a_socket() {
        let cmd = wlrctl("wayland-9", &["toplevel".to_string(), "list".to_string()]);
        let env: Vec<_> = cmd.get_envs().map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned()))).collect();
        assert!(env.contains(&("WAYLAND_DISPLAY".into(), Some("wayland-9".into()))), "{env:?}");
        assert!(env.contains(&("WAYLAND_SOCKET".into(), None)), "removed, not left to inherit: {env:?}");
        let list = include_str!("mind_view.rs");
        let list = &list[list.find("pub fn nested_window_lines").unwrap()..];
        let list = &list[..list.find("\n}\n").unwrap()];
        assert!(list.contains("mind_view_close::wlrctl(&display"), "the lister uses the same builder");
    }

    #[test]
    fn an_unknown_title_is_refused_and_two_matches_are_not_guessed() {
        let said = resolve(&lines(), "Notes", None).unwrap_err();
        assert!(said.contains("not an app in Mind View") && said.contains("New Tab - Chromium"), "{said}");
        assert!(resolve(&lines(), "  ", None).is_err());
        let two = vec!["chromium: A".to_string(), "chromium: B".to_string()];
        assert!(resolve(&two, "chromium", None).unwrap_err().contains("matches 2 windows"));
        assert!(resolve(&[], "chromium", None).is_err());
    }
}
