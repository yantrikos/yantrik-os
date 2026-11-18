//! The reviewed list: the (surface id, action) pairs the built-in companion may run through
//! `app_action` although the app grades them above `safe` (#1313, round 6).
//!
//! Everything not on this list and not graded `safe` by the app is refused at run time
//! (`app_guard::admit`). The id is the one the app's socket is bound under, from its `.desktop`
//! `X-Yantrik-Surface` or its `const ID` (`editor`, `documents`, `snippets`, `containers`, `music`,
//! `network`), never the folder name. A pair stays only if its handler does not write, delete, move,
//! send, schedule or run a command; each group says why. `tests/app_guard_surface.rs` fails when a
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
            "weather.refresh", "weather.show_location",
            "spreadsheet.select", "email.select_folder", "email.search",
        ],
    },
    Reviewed {
        because: "opens or reads something and shows it; the picture is turned or mirrored on screen only, a message is only opened",
        actions: &[
            "image-viewer.open", "image-viewer.show", "image-viewer.next", "image-viewer.previous",
            "image-viewer.rotate", "image-viewer.flip", "image-viewer.fit", "image-viewer.toggle_info",
            "email.open_message", "email.open_message_id", "download-manager.open_folder",
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
            "music.play", "music.pause", "music.next", "music.previous", "music.seek", "music.set_volume", "music.play_file",
        ],
    },
    Reviewed {
        because: "holds or lets go of a transfer already in the list, or checks a finished file against a checksum; fetches nothing, removes nothing",
        actions: &[
            "download-manager.pause", "download-manager.resume", "download-manager.pause_all", "download-manager.resume_all",
            "download-manager.verify",
        ],
    },
    Reviewed {
        because: "starts a catalogued app, or brings, shows, hides or switches a window or screen of this desktop, or lists what is installed; runs no line it is given, and what shows up on screen is read (sensitive, gated by the shell)",
        actions: &[
            "shell.open_app", "shell.refresh_apps", "shell.show_screen", "shell.show_app", "shell.focus_window",
            "shell.minimise_window", "shell.maximise_window", "shell.switcher_commit", "shell.read_screen",
            "shell.open_capture", "shell.new_chat",
        ],
    },
    Reviewed {
        because: "moves around and opens in the Files view of the shell: a listing, a folder, a file in its viewer; nothing is created, renamed or removed",
        actions: &["shell.files_go", "shell.files_enter", "shell.files_open", "shell.files_up", "shell.files_select", "shell.files_view"],
    },
    Reviewed {
        because: "a level of this machine that is gone at the next restart or reversed by the same call: volume, mute, brightness; the microphone only ever muted, never made live",
        actions: &["shell.set_volume", "shell.set_mute", "shell.set_brightness", "shell.mute_mic"],
    },
    Reviewed {
        because: "holds, lets go of or stops work the person already allowed or that the companion started (a recipe, an agent, the Mind desk); starts nothing and hands nothing over",
        actions: &["shell.pause_recipe", "shell.resume_recipe", "shell.stop_agent", "shell.mind_desk_stop"],
    },
    Reviewed {
        because: "raises the shell's own card, or lowers what the companion may do, and changes nothing alone; a commitment on a web page goes through `commit`, which asks the person in every mode",
        actions: &[
            "shell.turn_off_background_research", "shell.ask_about_background_research_again", "browser.commit",
        ],
    },
    Reviewed {
        because: "moves through the Mind's browser history or tabs without typing, clicking or sending anything and without an address of the caller's",
        actions: &["browser.back", "browser.forward", "browser.switch_tab"],
    },
    Reviewed {
        because: "stops a job the companion or the person started (a generation, in Studio) and opens a finished picture; creates and sends nothing",
        actions: &["studio.open", "studio.cancel", "studio.cancel_all"],
    },
];

/// Whether `id.action` is on the list.
pub fn is_reviewed(id: &str, action: &str) -> bool {
    REVIEWED.iter().flat_map(|group| group.actions).any(|pair| pair.split_once('.').is_some_and(|(i, a)| i == id && a == action))
}
