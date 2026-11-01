//! A Mind closes the apps it opened in Mind View, and only those (#1033).
//!
//! `close_window` resolves a title against the person's desktop first, so a Mind that closed
//! "its" browser could close theirs. For a Mind it now reads only Mind View's own compositor, by
//! what the Mind can know: the app's registry name (`browser`), the app id the window declared
//! (`chromium`), or the window's title as it is now (`New Tab - Chromium`, which the app changes
//! as it runs). The close is sent to Mind View's display, so a window of the person's cannot be
//! reached, whatever it is titled.

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
    let mut titles: Vec<&str> = found.iter().map(|w| w.title.as_str()).collect();
    titles.sort_unstable();
    titles.dedup();
    match (found.first(), titles.len()) {
        (None, _) => {
            let shown = windows.iter().map(|w| w.title.clone()).collect::<Vec<_>>().join(", ");
            Err(format!(
                "`{}` is not an app in Mind View. Windows on the person's desktop can't be closed by a Mind; \
                 Mind View shows: {}",
                want.trim(),
                if shown.is_empty() { "nothing" } else { &shown }
            ))
        }
        (Some(w), 1) => {
            // A title names the window as it is; for an app id, the id is what stays true if the
            // title changes between the list and the close.
            let selector = if !exact && !w.wayland_app_id.is_empty() {
                format!("app_id:{}", w.wayland_app_id)
            } else {
                format!("title:{}", w.title)
            };
            Ok((selector, w.title.clone()))
        }
        (_, n) => Err(format!(
            "`{}` matches {n} windows in Mind View and closing the wrong one cannot be undone; say which: {}",
            want.trim(),
            titles.join(", ")
        )),
    }
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
    let name = crate::mind_view::close_matching(selector, title)?;
    Ok(json!({
        "closing": name,
        "where": "Mind View",
        "note": "the app was drawn in Mind View, where the apps a mind opens go, and was asked to close \
                 there as pressing × does; one with unsaved work may put up its own dialog and stay. \
                 Read `mind_view` in `describe shell` to see whether it went.",
    }))
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
        for token in ["addressable_titles", "window_to_close", "windows::close(", "Command::new"] {
            assert!(!src.contains(token), "the Mind's close must not reach the person's desktop: `{token}`");
        }
        assert!(src.contains("nested_window_lines()") && src.contains("close_matching("));
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
