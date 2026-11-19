//! The reviewed list: the (surface id, action) pairs the built-in companion may run through
//! `app_action` although the app grades them above `safe` (#1313, round 6).
//!
//! Everything not on this list and not graded `safe` by the app is refused at run time
//! (`app_guard::admit`). The id is the one the app's socket is bound under, from its `.desktop`
//! `X-Yantrik-Surface` or its `const ID` (`editor`, `documents`, `snippets`, `containers`, `music`,
//! `network`), never the folder name. A pair stays only if its handler changes nothing the person
//! keeps, starts nothing and sends nothing, in every mode; when in doubt it comes off, and the card
//! the companion's own tools raise can ask for it. Each group says why. `tests/app_guard_surface.rs` fails when a
//! pair here stops existing under its id.

/// A group of pairs and the one reason they stay.
pub struct Reviewed {
    pub because: &'static str,
    /// `id.action`.
    pub actions: &'static [&'static str],
}

/// The pairs, by reason.
pub const REVIEWED: &[Reviewed] = &[
    Reviewed {
        because: "moves a view, a day, a folder or a slide and changes nothing the app stores; no buffer is replaced",
        actions: &[
            "calendar.select_day", "calendar.show_date", "calendar.show_month", "calendar.go_to_today", "calendar.set_view",
            "containers.refresh", "containers.show_tab", "containers.show_logs",
            "network.refresh", "network.wifi_scan",
            "notes.search", "notes.set_folder", "notes.preview", "notes.focus", "notes.open_in_new_window",
            "presentation.show", "presentation.go_to", "presentation.next", "presentation.previous", "presentation.presenter_view",
            "system-monitor.sort_processes", "system-monitor.filter_processes",
            "weather.refresh",
            "spreadsheet.select", "email.select_folder", "email.search",
        ],
    },
    Reviewed {
        because: "opens a picture and shows it; the picture is turned or mirrored on screen only, nothing is written",
        actions: &[
            "image-viewer.open", "image-viewer.show", "image-viewer.next", "image-viewer.previous",
            "image-viewer.rotate", "image-viewer.flip", "image-viewer.fit", "image-viewer.toggle_info",
        ],
    },
    Reviewed {
        because: "works on tabs and the find bar; a tab with unsaved changes is not closed (it asks), and nothing is written to a file",
        actions: &[
            "editor.close", "editor.cancel", "editor.new", "editor.open", "editor.show",
            "editor.find", "editor.find-next", "editor.find-prev", "editor.select_tab",
        ],
    },
    Reviewed {
        because: "playback of the person's library: starts, stops, skips or seeks a track, changes no file",
        actions: &[
            "music.play", "music.pause", "music.next", "music.previous", "music.seek", "music.play_file",
        ],
    },
    Reviewed {
        because: "holds a transfer already in the list; it only stops one, starts no transfer, fetches nothing, removes nothing",
        actions: &["download-manager.pause", "download-manager.pause_all"],
    },
    Reviewed {
        because: "brings, shows, hides or switches a window or screen of this desktop, or lists what is installed; starts no program, runs no line it is given, and what shows up on screen is read (sensitive, gated by the shell)",
        actions: &[
            "shell.refresh_apps", "shell.show_screen", "shell.show_app", "shell.focus_window",
            "shell.minimise_window", "shell.maximise_window", "shell.switcher_commit", "shell.read_screen",
        ],
    },
    Reviewed {
        because: "moves around in the Files view of the shell: a listing, a folder, a selection, the view; nothing is opened, created, renamed or removed",
        actions: &["shell.files_go", "shell.files_enter", "shell.files_up", "shell.files_select", "shell.files_view"],
    },
    Reviewed {
        because: "a level of this machine that the same call puts back: volume, mute, brightness; the microphone only ever muted, never made live (this machine's sound server may keep the level across a restart)",
        actions: &["shell.set_volume", "shell.set_mute", "shell.set_brightness", "shell.mute_mic"],
    },
    Reviewed {
        because: "pauses a recipe that is running; it stops work and starts, resumes, sends and deletes nothing, and the person lets it go on",
        actions: &["shell.pause_recipe"],
    },
    Reviewed {
        because: "writes the background-research setting, but only in the safe direction: it turns background research off and cannot turn it on, ask for it or run a search",
        actions: &["shell.turn_off_background_research"],
    },
    Reviewed {
        because: "moves through the Mind's browser history or tabs without typing, clicking or sending anything and without an address of the caller's",
        actions: &["browser.back", "browser.forward", "browser.switch_tab"],
    },
    Reviewed {
        because: "stops a job that is running (a generation, in Studio); starts, creates, opens and sends nothing",
        actions: &["studio.cancel", "studio.cancel_all"],
    },
];

/// Whether `id.action` is on the list.
pub fn is_reviewed(id: &str, action: &str) -> bool {
    REVIEWED.iter().flat_map(|group| group.actions).any(|pair| pair.split_once('.').is_some_and(|(i, a)| i == id && a == action))
}
