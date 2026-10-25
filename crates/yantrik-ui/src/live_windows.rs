//! The compositor's account of what is on screen, read from its live window stream.
//!
//! `windows::shell_windows` merges the launch registry with what the compositor says is open. That
//! account used to be only a `wlrctl toplevel list` reading, kept for nine seconds
//! (`COMPOSITOR_TTL`) and read by a poll that turns every three. The registry knows processes, and
//! a multi-window app is one process, so a window an app opens or closes by itself — Notes'
//! second and third windows, from Ctrl+Shift+N or its header button — was only seen at the next
//! reading, up to twelve seconds later. The dock's count trailed one window behind: "2" over three
//! Notes windows, "3" after one of them closed, "2" over the last one (VM 520, the #918 run).
//!
//! `toplevel_watch` already follows the compositor's foreign-toplevel stream for Alt+Tab. While it
//! does, its list is the reading: no process is run, and it is as fresh as the compositor's last
//! event. When it does not (no protocol, or not on Wayland), the `wlrctl` reading stays the answer.

use crate::toplevel_watch::TopWindow;
use crate::windows::{self, WindowEntry};

/// The stream's account now, as the window list reads it: the windows, and the app window that
/// was in front most recently. `None` when the stream is not being followed.
pub(crate) fn now() -> Option<(Vec<WindowEntry>, Option<String>)> {
    let listed = crate::toplevel_watch::windows()?;
    Some(from_stream(&listed, &crate::toplevel_watch::recency()))
}

/// `listed` (the stream's windows, in the compositor's order) as the window list's entries, read
/// the way a `wlrctl` line is, so a window has the same id and title whichever source named it.
///
/// The front is the most recently focused window still on the list that is not the shell's own:
/// the `wlrctl` reading kept that window when the person clicked back to the desktop, and so does
/// this, because the taskbar's promise is "most recently focused".
pub(crate) fn from_stream(listed: &[TopWindow], recency: &[String]) -> (Vec<WindowEntry>, Option<String>) {
    let entries: Vec<WindowEntry> = listed
        .iter()
        .filter(|w| !windows::is_shell_surface(w.title.trim()))
        .map(|w| windows::window_entry(w.app_id.trim(), w.title.trim()))
        .collect();
    let front = recency
        .iter()
        .map(|t| t.trim())
        .find(|t| entries.iter().any(|e| e.title == *t))
        .map(str::to_string);
    (entries, front)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock_model::{self, LaunchOrder, Pin, Win};
    use crate::running::RunningApp;
    use crate::windows::{OSD_WINDOW_TITLE, SHELL_WINDOW_TITLE};

    fn top(id: u64, app_id: &str, title: &str) -> TopWindow {
        TopWindow { id, title: title.into(), app_id: app_id.into() }
    }

    /// One Notes process, which is one registry entry whatever number of windows it has.
    fn notes_process() -> Vec<RunningApp> {
        vec![RunningApp { app_id: "notes".into(), pid: 7, binary: "yantrik-notes".into(), since_unix: 0 }]
    }

    /// The dock's count for `app`, from the stream's list as the shell merges and groups it.
    fn badge(listed: &[TopWindow], app: &str) -> usize {
        let (discovered, front) = from_stream(listed, &[]);
        let merged = windows::merge_windows(&notes_process(), discovered, front.as_deref());
        let wins: Vec<Win> = merged
            .into_iter()
            .map(|w| Win { title: w.title, app_id: w.app_id, subtitle: w.subtitle })
            .collect();
        let pins = [Pin { app_id: "notes".into(), label: "Notes".into() }];
        let entries = dock_model::entries(&pins, &wins, None, &mut LaunchOrder::default(), |id, _| id.to_string());
        entries.iter().find(|e| e.app_id == app).map_or(0, |e| e.windows.len())
    }

    /// Three windows of one process are three on the badge, and each one closed is one fewer at
    /// once: the count follows the compositor's list, not the registry's one entry per process.
    #[test]
    fn every_window_of_a_multi_window_app_is_counted_as_it_opens_and_closes() {
        let shell = top(1, "", SHELL_WINDOW_TITLE);
        let notes = |id| top(id, "", "Notes");
        let three = [shell.clone(), notes(2), notes(3), notes(4)];
        assert_eq!(badge(&three, "notes"), 3, "three Notes windows, three on the badge");
        assert_eq!(badge(&[shell.clone(), notes(2), notes(4)], "notes"), 2, "one closed");
        assert_eq!(badge(&[shell, notes(4)], "notes"), 1, "the last one");
    }

    /// The shell's own surfaces are never windows, and the stream's titles read as `wlrctl` lines
    /// read: a window that declares no id is named by its title, one that does keeps the id.
    #[test]
    fn the_stream_is_read_as_the_wlrctl_list_is() {
        let listed = [
            top(1, "", SHELL_WINDOW_TITLE),
            top(2, "", OSD_WINDOW_TITLE),
            top(3, "", " Notes "),
            top(4, "Blender", "(Unsaved) - Blender 4.3.2"),
        ];
        let (entries, _) = from_stream(&listed, &[]);
        let wlrctl: Vec<WindowEntry> =
            [": Notes", "Blender: (Unsaved) - Blender 4.3.2"].iter().map(|l| windows::toplevel_entry(l)).collect();
        assert_eq!(entries, wlrctl);
    }

    /// The front is the app window most recently in front, and not the shell's, nor a window that
    /// has gone since it had focus.
    #[test]
    fn the_front_is_the_most_recently_focused_app_window_still_open() {
        let listed = [top(1, "", SHELL_WINDOW_TITLE), top(2, "", "Notes"), top(3, "", "Weather")];
        let front = |recency: &[&str]| {
            let recency: Vec<String> = recency.iter().map(|t| t.to_string()).collect();
            from_stream(&listed, &recency).1
        };
        assert_eq!(front(&[SHELL_WINDOW_TITLE, "Weather", "Notes"]), Some("Weather".into()), "back on the desktop");
        assert_eq!(front(&["Terminal", "Notes"]), Some("Notes".into()), "a closed window is no one's front");
        assert_eq!(front(&[SHELL_WINDOW_TITLE]), None, "nothing else has had focus");
    }

    /// The window list asks the stream before the `wlrctl` reading, so every reader of it — the
    /// dock, the taskbar, `describe shell` — sees a window as soon as the compositor announces it.
    #[test]
    fn the_window_list_reads_the_stream_first() {
        let src = include_str!("windows.rs");
        let f = &src[src.find("fn compositor_snapshot").expect("the snapshot")..];
        let f = &f[..f.find("\n}\n").expect("its end")];
        let live = f.find("live_windows::now()").expect("the stream is asked");
        let cached = f.find("COMPOSITOR").expect("the wlrctl reading is the fallback");
        assert!(live < cached, "the stream first, the cached reading after:\n{f}");
    }
}
