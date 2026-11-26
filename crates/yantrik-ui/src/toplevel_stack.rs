//! Which windows to put back in front after the desktop has been in front of them (#1419).
//!
//! The shell is one ordinary fullscreen toplevel, so anything that raises it (a card, the Lens, a
//! click on its dock, or labwc picking the next window after a close) lifts it over every app.
//! Handing the screen back used to activate ONE window; every other window stayed behind the
//! desktop, open and not minimized, and the person saw an empty desktop with apps "running".
//!
//! The compositor does not say how windows are stacked, but a window comes forward when it has
//! focus, so the focus history (`toplevel_watch::recency_ids`) is the stacking order of the
//! windows that have been used. Activating them oldest first, the front one last, rebuilds it.

/// The windows to activate, in the order to do it: oldest first, so the one that was in front is
/// activated last and ends up on top.
///
/// `recent` is the focus history, most recent first. The desktop itself and minimized windows are
/// left out: the first is what is being moved away from, and activating the second would
/// un-minimize it.
pub fn restack<K: Clone>(recent: &[(K, String)], shell: &str, minimized: impl Fn(&K) -> bool) -> Vec<K> {
    recent
        .iter()
        .filter(|(k, title)| title != shell && !minimized(k))
        .map(|(k, _)| k.clone())
        .rev()
        .collect()
}

/// The one window to give focus to when the window in front has closed: the most recently used
/// one that is still on screen. `None` when there is nothing but the desktop.
pub fn after_close<K: Clone>(recent: &[(K, String)], shell: &str, minimized: impl Fn(&K) -> bool) -> Option<K> {
    recent.iter().find(|(k, title)| title != shell && !minimized(k)).map(|(k, _)| k.clone())
}

use std::sync::Mutex;

/// The windows the compositor reports as minimized, by id, and the apps (the dock's ids) whose
/// every window is minimized; the dock reads the apps so a minimized app is not drawn, or
/// described, as the one in front. By app and not by title: a title is whatever the window says
/// today, and the dock's rows are keyed by app.
static MINIMIZED: Mutex<(Vec<u64>, Vec<String>)> = Mutex::new((Vec::new(), Vec::new()));

/// The apps with windows, every one of which is minimized. `windows` is (dock id, minimized).
pub fn apps_put_away(windows: &[(String, bool)]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (app, _) in windows {
        if !out.contains(app) && windows.iter().filter(|(a, _)| a == app).all(|(_, down)| *down) {
            out.push(app.clone());
        }
    }
    out
}

pub fn set_minimized(ids: Vec<u64>, titles: Vec<String>) {
    if let Ok(mut m) = MINIMIZED.lock() {
        *m = (ids, titles);
    }
}

pub fn is_minimized(id: &u64) -> bool {
    MINIMIZED.lock().is_ok_and(|m| m.0.contains(id))
}

/// Dock ids of the apps that are entirely minimized; empty while the stream is not followed.
pub fn minimized_apps() -> Vec<String> {
    MINIMIZED.lock().map(|m| m.1.clone()).unwrap_or_default()
}

/// [`after_close`] over the live history: `ids` and `titles` are `recency_ids` and `recency`.
pub fn next_after_close(ids: &[u64], titles: &[String], shell: &str) -> Option<u64> {
    let pairs: Vec<(u64, String)> = ids.iter().copied().zip(titles.iter().cloned()).collect();
    after_close(&pairs, shell, is_minimized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history() -> Vec<(u32, String)> {
        vec![(3, "Notes".into()), (9, "Yantrik OS".into()), (2, "Weather".into()), (1, "Files".into())]
    }

    #[test]
    fn an_app_is_put_away_only_when_every_window_of_it_is() {
        let w = |a: &str, d| (a.to_string(), d);
        let got = apps_put_away(&[w("notes", true), w("terminal", true), w("terminal", false), w("files", false)]);
        assert_eq!(got, ["notes"]);
        assert!(apps_put_away(&[]).is_empty());
    }

    #[test]
    fn the_stack_is_rebuilt_oldest_first_so_the_front_window_ends_on_top() {
        assert_eq!(restack(&history(), "Yantrik OS", |_| false), [1, 2, 3]);
    }

    #[test]
    fn the_desktop_and_minimized_windows_are_never_activated() {
        assert_eq!(restack(&history(), "Yantrik OS", |k| *k == 2), [1, 3]);
        assert!(restack(&history(), "Yantrik OS", |_| true).is_empty());
    }

    #[test]
    fn closing_the_front_window_brings_the_next_used_one_forward_not_the_desktop() {
        let after = &history()[1..];
        assert_eq!(after_close(after, "Yantrik OS", |_| false), Some(2));
        assert_eq!(after_close(after, "Yantrik OS", |k| *k == 2), Some(1));
        assert_eq!(after_close(&[(9, "Yantrik OS".to_string())], "Yantrik OS", |_| false), None);
    }
}
