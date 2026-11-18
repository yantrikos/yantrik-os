//! Which effects the companion may not reach through `app_action` (#1311, round 5 of #1313).
//!
//! "Approve the effect, not the route to it": the four questions (`asks.rs`) guard what the
//! companion's own tools do to the person's mail, calendar, files, memory and plans, and a card the
//! person pressed is the only thing that lets one go ahead. `app_action` is one tool name for every
//! app's every action, so an app that publishes the same effect under its own action name
//! (`calendar.delete_event`) was a second road to it, with no card in full bypass. This is the
//! guard on that road: [`refusal`] is read by `app_act::gated_act` before the app is started and
//! again once its socket exists (an alias is a symlink to the real one, so a second name for an app
//! is read as the app's own).
//!
//! There is one way to each effect, and it asks. A call this refuses raises no card: the sentence
//! names the tool that asks instead, or, where the companion has none (an overwritten note, a
//! deleted snippet, a mail marked read), says the person can do it in the app.
//!
//! Two lists, and nothing between them: [`REFUSED`] holds the (app, action) pairs whose effect
//! belongs to a question, and [`REVIEWED`] holds every other action graded above `safe`, with why it
//! is not a guarded effect. `tests/app_guard_surface.rs` reads the source of every `apps/*` app and
//! of the shell and fails when an action graded above `safe` is on neither, so a new action cannot
//! arrive unnoticed, and when a listed pair no longer exists.

use crate::asks::Question;

/// An effect refused on the `app_action` road.
pub struct Refused {
    pub app: &'static str,
    pub action: &'static str,
    /// The question that guards the same effect when the companion makes it with its own tool.
    pub question: Question,
    /// The companion tool that asks for this effect, where there is one.
    pub instead: Option<&'static str>,
}

const fn refused(app: &'static str, action: &'static str, question: Question, instead: Option<&'static str>) -> Refused {
    Refused { app, action, question, instead }
}

/// Every pair whose effect is a question's, each mapped to it.
pub const REFUSED: &[Refused] = &[
    refused("calendar", "delete_event", Question::DeleteForGood, Some("calendar_delete_event")),
    refused("calendar", "update_event", Question::ChangeItem, Some("calendar_update_event")),
    refused("document-editor", "save", Question::DeleteForGood, Some("write_file")),
    refused("document-editor", "save_as", Question::DeleteForGood, Some("write_file")),
    refused("document-editor", "set_content", Question::DeleteForGood, Some("write_file")),
    refused("document-editor", "export_markdown", Question::DeleteForGood, Some("write_file")),
    refused("presentation", "save", Question::DeleteForGood, Some("write_file")),
    refused("presentation", "save_as", Question::DeleteForGood, Some("write_file")),
    refused("presentation", "export_markdown", Question::DeleteForGood, Some("write_file")),
    refused("spreadsheet", "save_over", Question::DeleteForGood, Some("write_file")),
    refused("text-editor", "save", Question::DeleteForGood, Some("write_file")),
    refused("text-editor", "save_as", Question::DeleteForGood, Some("write_file")),
    refused("text-editor", "set_content", Question::DeleteForGood, Some("write_file")),
    refused("text-editor", "discard", Question::DeleteForGood, None),
    refused("notes", "set_content", Question::DeleteForGood, None),
    refused("notes", "append", Question::DeleteForGood, None),
    refused("notes", "set_title", Question::DeleteForGood, None),
    refused("notes", "trash", Question::DeleteForGood, None),
    refused("notes", "discard_draft", Question::DeleteForGood, None),
    refused("snippet-manager", "save", Question::ChangeItem, None),
    refused("snippet-manager", "delete", Question::DeleteForGood, None),
    refused("email", "mark_read", Question::ChangeItem, None),
    refused("email", "flag", Question::ChangeItem, None),
    refused("shell", "run_recipe", Question::PlanWork, Some("run_recipe")),
    refused("blender", "save", Question::DeleteForGood, None),
    refused("shell", "files_delete", Question::DeleteForGood, None),
    refused("shell", "files_rename", Question::ChangeItem, None),
    refused("shell", "desktop_archive", Question::ChangeItem, None),
];

/// Actions graded above `safe` that are not a guarded effect, grouped by why.
pub struct Reviewed {
    pub because: &'static str,
    /// `app.action`.
    pub actions: &'static [&'static str],
}

/// The reviewed list. Every action of every app and of the shell graded above `safe` is here or in
/// [`REFUSED`].
pub const REVIEWED: &[Reviewed] = &[
    Reviewed {
        because: "changes only what the window shows or plays; no mail, event, file, memory or plan is changed",
        actions: &["arcade.play", "arcade.verify", "calendar.select_day", "calendar.show_date", "calendar.show_month", "calendar.go_to_today", "calendar.set_view", "container-manager.refresh", "container-manager.show_tab", "container-manager.show_logs", "download-manager.open_folder", "image-viewer.open", "image-viewer.show", "image-viewer.next", "image-viewer.previous", "image-viewer.rotate", "image-viewer.flip", "image-viewer.fit", "image-viewer.toggle_info", "network-manager.refresh", "network-manager.wifi_scan", "notes.search", "notes.set_folder", "notes.notebook", "notes.tags", "notes.copy", "notes.reload", "notes.preview", "notes.focus", "notes.open_in_new_window", "notes.open_note", "presentation.open", "presentation.show", "presentation.new_deck", "presentation.go_to", "presentation.next", "presentation.previous", "presentation.presenter_view", "snippet-manager.copy", "snippet-manager.toggle_favorite", "system-monitor.sort_processes", "system-monitor.filter_processes", "text-editor.close", "weather.refresh", "shell.files_go", "shell.files_enter", "shell.files_open", "shell.files_up", "shell.files_select", "shell.files_view"],
    },
    Reviewed {
        because: "starts or arranges things on this desktop (an app, a service, a window, a pin, a self-check): not a mail, event, file, memory or plan",
        actions: &["shell.open_app", "shell.start_service", "shell.refresh_apps", "shell.pin_app", "shell.show_screen", "shell.show_app", "shell.focus_window", "shell.close_window", "shell.minimise_window", "shell.maximise_window", "shell.desktop_place", "shell.prove", "shell.switcher_commit"],
    },
    Reviewed {
        because: "reads or opens something and changes nothing of the person's",
        actions: &["document-editor.open", "document-editor.new", "email.open_message", "email.select_folder", "email.search", "notes.import", "spreadsheet.select", "spreadsheet.open", "text-editor.cancel", "text-editor.new", "text-editor.open", "text-editor.show", "text-editor.find", "text-editor.find-next", "text-editor.find-prev", "text-editor.select_tab", "shell.read_screen"],
    },
    Reviewed {
        because: "edits the open document in the window only; nothing reaches a file until save, save_as or export, which this road refuses",
        actions: &["document-editor.append", "document-editor.replace_all", "presentation.add_slide", "presentation.set_slide", "presentation.delete_slide", "presentation.move_slide", "spreadsheet.set_cell", "spreadsheet.undo", "spreadsheet.redo", "text-editor.replace_text", "text-editor.replace", "text-editor.replace-all", "text-editor.append", "text-editor.undo", "text-editor.redo"],
    },
    Reviewed {
        because: "writes or reverts only what the window already holds; every way to put new text into it is refused above",
        actions: &["notes.save", "notes.undo", "notes.redo"],
    },
    Reviewed {
        because: "creates something new and replaces nothing that exists; the companion's own tool for it asks nothing either (adding is not one of the four)",
        actions: &["calendar.add_event", "notes.keep_mine_as_copy", "notes.new_note", "snippet-manager.new", "snippet-manager.import", "studio.generate", "studio.variations", "studio.upscale", "studio.open", "studio.cancel", "studio.cancel_all"],
    },
    Reviewed {
        because: "creates an empty file or folder where nothing is, and refuses to replace one",
        actions: &["spreadsheet.save", "shell.files_new_folder", "shell.files_new_file"],
    },
    Reviewed {
        because: "keeps or replaces a game or character spec in the arcade's own library, work the companion itself makes there; the person's files are not touched",
        actions: &["arcade.new_character", "arcade.new_game", "arcade.update_game", "arcade.update_character", "arcade.build"],
    },
    Reviewed {
        because: "moves to a Trash that can bring it back, inside the app's own folder of what the companion made; not the person's files",
        actions: &["arcade.delete", "studio.delete"],
    },
    Reviewed {
        because: "reaches only what this caller created itself (the machine checks the ownership, not the request), none of the person's items",
        actions: &["calendar.delete_own_event", "calendar.update_own_event"],
    },
    Reviewed {
        because: "a copy leaving a file where it is, refusing to replace one that exists",
        actions: &["notes.export"],
    },
    Reviewed {
        because: "puts back what the person wrote or kept; replaces nothing without a conflict check",
        actions: &["notes.restore", "notes.take_theirs", "notes.restore_draft"],
    },
    Reviewed {
        because: "opens the composer with a draft and does NOT send: the person reviews and sends it",
        actions: &["email.compose"],
    },
    Reviewed {
        because: "opens a sign-in page: reach-refused unattended and changes nothing",
        actions: &["email.begin_google_sign_in"],
    },
    Reviewed {
        because: "playback or library rescan; changes no file of the person's",
        actions: &["music-player.play", "music-player.pause", "music-player.next", "music-player.previous", "music-player.seek", "music-player.set_volume", "music-player.play_file", "music-player.rescan"],
    },
    Reviewed {
        because: "sends nothing from the person's accounts; network or download work refused unattended when it fetches an address (reach), and not one of the four",
        actions: &["download-manager.add", "download-manager.pause", "download-manager.resume", "download-manager.cancel", "download-manager.retry", "download-manager.verify", "download-manager.pause_all", "download-manager.resume_all", "download-manager.clear_completed"],
    },
    Reviewed {
        because: "stops or starts a process or container: dangerous and gated by the app's grade in every mode; not one of the four",
        actions: &["container-manager.start", "container-manager.stop", "container-manager.restart", "container-manager.remove", "system-monitor.kill_process"],
    },
    Reviewed {
        because: "runs a program or a command line: refused unattended (open-ended) and not one of the four; the terminal and the sealed workshop have their own consent",
        actions: &["terminal.run", "terminal.send_input", "terminal.new_tab", "terminal.open_directory", "shell.workshop_create", "shell.workshop_revoke", "shell.agent_run_sealed", "shell.workshop_present", "shell.workshop_export", "shell.agent_run", "shell.agent_input", "shell.agent_job", "shell.agent_kill", "blender.run_python"],
    },
    Reviewed {
        because: "hands work to or steers an agent on this desktop: the shell's own gate asks for hand-offs after a private read in every mode, and an agent is not one of the four",
        actions: &["shell.new_agent", "shell.send_to_agent", "shell.stop_agent", "shell.hand_off", "shell.new_chat", "shell.mind_desk_stop"],
    },
    Reviewed {
        because: "steers a recipe the person already allowed to start (pause, resume, answer, cancel); starting one is refused above",
        actions: &["shell.answer_recipe", "shell.pause_recipe", "shell.resume_recipe", "shell.cancel_recipe"],
    },
    Reviewed {
        because: "changes a setting or a device of this desktop (volume, theme, network, power, harness choice): not a mail, event, file, memory or plan, and graded and gated in every mode",
        actions: &["network-manager.wifi_connect", "network-manager.wifi_disconnect", "network-manager.wifi_forget", "network-manager.wifi_radio", "studio.set_backend", "weather.show_location", "weather.add_location", "weather.set_units", "shell.use_harness", "shell.install_harness", "shell.assign_provider", "shell.revert_provider", "shell.configure_harness", "shell.start_harness", "shell.set_do_not_disturb", "shell.set_auto_lock", "shell.set_power_profile", "shell.set_volume", "shell.set_mute", "shell.set_brightness", "shell.set_mic_mute", "shell.mute_mic", "shell.set_wifi", "shell.disconnect_network", "shell.connect_wifi", "shell.power_sleep", "shell.power_off", "shell.set_theme", "shell.set_dark_style", "shell.set_update_channel", "shell.apply_update", "shell.reconnect_wifi", "shell.choose_wifi_network"],
    },
    Reviewed {
        because: "writes a new capture file and overwrites none; not one of the four",
        actions: &["arcade.screenshot", "shell.screenshot_screen", "shell.record_start", "shell.open_capture", "shell.record_stop"],
    },
    Reviewed {
        because: "the installer's own consented flow, available only on the first-run desktop; not the companion's changes to the person's data",
        actions: &["shell.installer_set", "shell.installer_go_to", "shell.installer_install", "shell.installer_reboot", "shell.installer_erase_macos", "shell.installer_choose_target"],
    },
    Reviewed {
        because: "the Lens's own send, which refuses an agent (turn_source) and is open-ended, so refused unattended; not a chat message from the person's account",
        actions: &["shell.send_message"],
    },
    Reviewed {
        because: "is itself a question, a consent or a commitment card: it asks the person and changes nothing alone",
        actions: &["shell.report_problem", "shell.send_mail", "shell.allow_location_lookup", "shell.allow_background_research", "shell.turn_off_background_research", "shell.ask_about_background_research_again", "browser.commit"],
    },
    Reviewed {
        because: "edits or renders the open Blender scene in memory or writes a new picture; the .blend file changes only through save, which this road refuses",
        actions: &["blender.new_scene", "blender.add_primitive", "blender.delete_object", "blender.transform", "blender.set_material", "blender.set_camera", "blender.set_light", "blender.import_model", "blender.set_render", "blender.render", "blender.open", "blender.screenshot"],
    },
    Reviewed {
        because: "a hand on a web page in the Mind View browser; what reads as a commitment (buy, pay, send, delete) must go through commit, which asks the person in every mode, and webmail through a browser is not the mail tools' effect",
        actions: &["browser.go", "browser.back", "browser.forward", "browser.reload", "browser.click", "browser.context_click", "browser.drag", "browser.key_down", "browser.key_up", "browser.type", "browser.press", "browser.select", "browser.dialog", "browser.switch_tab", "browser.close_tab"],
    },
];

/// One spelling of an app's name, as the framework folds it: so a caller's capitals, spaces and
/// underscores are not part of the question.
fn fold(name: &str) -> String {
    name.trim().to_lowercase().replace([' ', '_'], "-")
}

/// The id a name is linked to, when its socket is a symlink to another's (`app-<alias>.sock` to
/// `app-<id>.sock`), in `dir`.
pub(crate) fn linked_id_in(dir: &std::path::Path, name: &str) -> Option<String> {
    let target = std::fs::read_link(dir.join(format!("app-{name}.sock"))).ok()?;
    let file = target.file_name()?.to_str()?;
    file.strip_prefix("app-")?.strip_suffix(".sock").map(str::to_string)
}

/// The names this call could be about: what was sent, folded, and the id its socket is linked to.
fn names_for(dir: &std::path::Path, app: &str) -> Vec<String> {
    let mut names = vec![app.trim().to_string(), fold(app)];
    for name in names.clone() {
        if let Some(id) = linked_id_in(dir, &name) {
            names.push(fold(&id));
        }
    }
    names
}

/// The guarded pair this call is, if it is one, with the sentence to refuse it in. `dir` is the
/// socket directory, read for aliases.
pub(crate) fn refusal_in(dir: &std::path::Path, app: &str, action: &str) -> Option<String> {
    let names = names_for(dir, app);
    let hit = REFUSED.iter().find(|r| r.action == action && names.iter().any(|n| n == r.app))?;
    let way = match hit.instead {
        Some(tool) => format!("Use `{tool}`, which asks the person on a card first."),
        None => format!("The companion has no tool that asks for this, so say what you would change and the person can do it in the {} app.", hit.app),
    };
    Some(format!(
        "Refused: `{}.{}` is an effect the person must see and press Allow on a card for ({}), and this road raises no card. {way} Nothing was changed.",
        hit.app,
        hit.action,
        hit.question.action()
    ))
}

/// [`refusal_in`] against this session's socket directory.
pub fn refusal(app: &str, action: &str) -> Option<String> {
    refusal_in(&yantrik_ipc_transport::server::socket_dir(), app, action)
}

#[cfg(test)]
#[path = "app_guard_tests.rs"]
mod tests;
