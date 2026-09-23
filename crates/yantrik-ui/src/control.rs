//! The shell describes itself.
//!
//! Every app under `apps/` now publishes `app.describe` / `app.act`, and the desktop is the one
//! window that matters most: it is where the companion lives. Without this, "what is on my
//! desktop right now" was answerable only by screenshotting the shell and asking a vision model
//! to read a status bar we wrote ourselves.
//!
//! Published on the same bus under `app-shell`, so `list_apps` finds it beside the others.
//!
//! The surface is deliberately narrow. The shell already serves the companion — memory, tools and
//! answers all arrive over `companion.*` on its own socket — so this covers only what those cannot
//! say: which screen is up, which windows are open, which services are running, and what the
//! status bar is reporting about the machine.

use slint::ComponentHandle;
use yantrik_app_runtime::control::{Action, App as ControlSurface, Param, View};

use crate::App;

/// The screens a caller may ask for by name.
///
/// Not every screen the shell can render. Boot, onboarding and login are states the shell enters
/// on its own and jumping into one would leave the session somewhere it cannot get back from;
/// locking has its own action, because locking a machine is an act rather than a view change.
/// Settings sections, by the name a caller would say.
///
/// Mirrors the list built in `wire::settings` — the ids are the same ints the sidebar uses.
const SETTINGS_SECTIONS: &[(&str, i32)] = &[
    ("appearance", 0),
    ("ai", 1),
    ("desktop", 2),
    ("network", 3),
    ("accounts", 4),
    ("privacy", 5),
    ("system", 6),
    ("skills", 7),
    ("harnesses", 8),
];

/// Refuse, with a reason a caller can act on, to open or pin an app that will not open.
fn check_launchable(name: &str, installed: &[crate::apps::DesktopEntry]) -> Result<(), String> {
    use crate::wire::dock::{availability, launchable_app_ids, Availability};
    match availability(name, installed) {
        Availability::Ready => Ok(()),
        Availability::Missing(what) => Err(format!(
            "`{name}` is not installed on this machine: {what} was not found. It can open: {}",
            launchable_app_ids(installed).join(", ")
        )),
        // Not "unknown" and not "not installed", because it is neither, and both of those
        // invite the caller to try again — to rescan, to install a package, to guess at another
        // spelling. This build does not have the app and no action on this machine will produce
        // it, so the refusal says that, says what is missing under the screen, and says what
        // would have to be built. An agent reading it can stop, or go and write the missing half.
        Availability::Shelved(shelf) => Err(format!(
            "`{name}` is not part of this build. {} is shelved: {}. It comes back when {} \
             — see design/shelved-2026-09-20.md. It can open: {}",
            shelf.name,
            shelf.reason,
            shelf.returns_when,
            launchable_app_ids(installed).join(", ")
        )),
        Availability::Unknown => Err(format!(
            "no app `{name}` on this machine; it can open: {}",
            launchable_app_ids(installed).join(", ")
        )),
    }
}

/// Open the Apps launcher where a person can see it, and answer with what is observed.
///
/// The launcher is an overlay on the desktop screen, not a screen of its own, and the dock's
/// `Launch::Launchpad` arm is what opens it: desktop first, then the grid. That arm did its job
/// every time — the log shows the catalogue rescan the grid triggers as it opens, right after
/// "Launching app app=launchpad" — and nobody saw it, because the shell is one toplevel under
/// labwc and the app window in front stayed in front. The photograph of "nothing" was Studio
/// in the evening and the Editor in the morning (#71, #118); `open_lens` later brought the
/// shell forward and the launcher was found standing open underneath, which is how it came to
/// be reported as living behind the Lens. Same defect `open_lens` had, same fix: ask the
/// compositor to raise the shell, and report whether it did rather than `accepted: true`.
///
/// Both doors end here — `open_app name=launchpad`, because that is what the listing says opens
/// it, and `show_screen screen=launchpad`, because a listing that called it a screen taught
/// every caller to try that next.
fn open_launcher(ui: &crate::App) -> serde_json::Value {
    // Through the dock's own arm, so there is one account of how the launcher opens.
    ui.invoke_launch_app("launchpad".into());
    let mut answer = serde_json::json!({
        "launcher_open": ui.get_app_grid_open(),
        "screen": screen_name(ui.get_current_screen()),
    });
    match crate::windows::raise_shell() {
        Ok(()) => answer["raised"] = true.into(),
        // Not an error: the launcher IS open. What it is not is visible, and a caller that
        // has just been told "open" is owed that difference — see `show_screen`.
        Err(why) => {
            answer["raised"] = false.into();
            answer["note"] = format!(
                "the launcher is open, but the shell's own window could not be brought to the \
                 front, so an app window may still be covering it: {why}"
            )
            .into();
        }
    }
    answer
}

/// Name to `current-screen` id, and the ids are the ones `app.slint` actually renders.
///
/// They were not. `("terminal", 16)` sent a caller to the ABOUT screen: 16 is about, terminal
/// is 14, and 14 has no branch in app.slint at all because the terminal became a separate app
/// binary and the shell screen went away. Nobody noticed because nothing compares this list to
/// the file that decides what a number means. A photograph of `show_screen screen=terminal`
/// showing "About" is how it surfaced.
///
/// `screens_match_the_shell` in the tests below now reads app.slint and checks every id here
/// against the `if current-screen == N` branches, so this cannot drift again in silence.
const SCREENS: &[(&str, i32)] = &[
    ("desktop", 1),
    ("bond", 4),
    ("personality", 5),
    ("memory", 6),
    ("settings", 7),
    ("files", 8),
    ("notifications", 9),
    ("system", 10),
    ("images", 11),
    ("editor", 12),
    ("media", 13),
    ("about", 16),
    ("packages", 21),
    ("devices", 27),
    ("permissions", 28),
    ("problems", 33),
    ("agents", 34),
    ("recipes", 35),
];

/// What `describe` calls the screen the shell is on.
///
/// Only the states a caller cannot ASK for belong in this match; everything else comes from
/// SCREENS, so a name can only be defined once. Two entries here were simply wrong -- 21 was
/// reported as "email" and 27 as "snippets", when 21 renders the package manager and 27 the
/// device dashboard. `describe` would tell an agent it was looking at email while the package
/// manager was on screen, which is worse than saying nothing.
pub(crate) fn screen_name(id: i32) -> &'static str {
    match id {
        0 => "boot",
        2 => "onboarding",
        3 => "lock",
        32 => "login",
        other => SCREENS
            .iter()
            .find(|(_, id)| *id == other)
            .map(|(name, _)| *name)
            .unwrap_or("unknown"),
    }
}

/// Join names the way a person would read them out: "a", "a and b", "a, b and c".
///
/// This line is the first thing anyone sees of the desktop, and "calendar and email and notes"
/// reads like a machine wrote it.
fn list_of(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// How much of the conversation `describe` reports, newest last.
///
/// The desktop could be asked a question by an agent and then had no way to tell it what came
/// back: the answer existed only as pixels, so confirming a reply meant screenshotting the shell
/// and reading the bubble with a vision model — the exact thing this surface exists to replace.
/// Six turns is enough to see a question and its answer with context around them, and short
/// enough that `describe` stays a glance.
const CONVERSATION_TAIL: usize = 6;

/// How much of one message travels. A long answer is read in the window; this is for confirming
/// what was said, not for moving a transcript through a control surface.
const MESSAGE_CLIP: usize = 600;

/// Cut to a length without splitting a character, and say that it was cut.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}… ({} characters total)", text.chars().count())
}

/// The tool calls in one message, each with its arguments, as the trail carried them.
fn calls_of(text: &str) -> Vec<serde_json::Value> {
    crate::trail::calls_in(text)
        .into_iter()
        .map(|c| {
            serde_json::json!({
                "name": c.name,
                "target": c.target,
                "summary": c.summary(),
                "arguments": c.arguments,
            })
        })
        .collect()
}

/// How many directory entries `describe` will list.
///
/// A glance, not a transcript: a model reading 4,000 filenames has spent its context on
/// something it could have asked a narrower question about. The true count travels beside
/// the list, so a caller always knows it is looking at a window onto something larger.
const FILE_LISTING_CAP: usize = 40;

/// The bond as `describe shell` reports it: the store's, or not loaded yet.
struct DescribedBond {
    loaded: bool,
    level: serde_json::Value,
    score: serde_json::Value,
    interactions: serde_json::Value,
}

/// What `describe shell` says about the bond.
///
/// `bond_data` is a Slint property with a default, and until the companion worker pushes the
/// store into it the default is all there is. That default used to be "Stranger, 0.0", and
/// `describe` reported it as the relationship — two minutes after a restart, over a store that
/// said Partner-in-Crime, 155 interactions. A reader can act on "not loaded yet"; on a level
/// nobody measured it can only be wrong. So before the first push every bond field is null and
/// `bond_loaded` is false; after it, they are the store's.
fn describe_bond(bond: &crate::BondData) -> DescribedBond {
    if !bond.loaded {
        return DescribedBond {
            loaded: false,
            level: serde_json::Value::Null,
            score: serde_json::Value::Null,
            interactions: serde_json::Value::Null,
        };
    }
    DescribedBond {
        loaded: true,
        level: bond.bond_level.to_string().into(),
        score: (bond.bond_score as f64).into(),
        interactions: bond.total_interactions.into(),
    }
}

/// Publish the desktop on the service bus. Call from the UI thread before `run()`.
///
/// Takes the service manager because the shell is the only process that owns service lifetimes:
/// `start_service` below is what makes the rail's "on demand" a mechanism rather than a caption
/// on a service nothing ever starts.
pub fn publish(
    ui: &App,
    ctx: &crate::app_context::AppContext,
    services: yantrik_shell_core::service_manager::ServiceManager,
) {
    // The Allow and Deny buttons, before anything can be asked for. They are Slint callbacks
    // and nothing else: granting is a click, never an action on this surface. See
    // `control_approvals`.
    crate::control_approvals::wire(ui);

    // The catalogue, not a copy of it. The control surface answers from the same live list
    // the launcher shows, so an app installed a moment ago is launchable by name without
    // restarting the shell — which is what `accepted: true` ought to mean.
    let installed = ctx.installed_apps.clone();
    let refresh_catalogue = ctx.installed_apps.clone();
    let describe = {
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return View::new("Yantrik — shutting down");
            };
            let screen = ui.get_current_screen();
            let bond = describe_bond(&ui.get_bond_data());

            // From the launch registry, not the Slint window-list model. The model is only
            // refreshed while the desktop screen is showing, so a describe from any other screen
            // reported "0 windows open" even with apps running — the registry is refreshed by
            // launches and exits, not by which screen is up, so it is right everywhere.
            let open: Vec<serde_json::Value> = crate::windows::shell_windows()
                .into_iter()
                .map(|w| {
                    serde_json::json!({
                        "title": w.title,
                        "app": w.app_id,
                    })
                })
                .collect();

            let service_model = ui.get_services();
            let services: Vec<serde_json::Value> = {
                use slint::Model;
                (0..service_model.row_count())
                    .filter_map(|i| service_model.row_data(i))
                    .map(|s| {
                        serde_json::json!({
                            "id": s.id.to_string(),
                            "status": s.status.to_string(),
                            "note": s.note.to_string(),
                        })
                    })
                    .collect()
            };

            let down: Vec<&str> = services
                .iter()
                .filter(|s| {
                    matches!(s["status"].as_str(), Some("failed") | Some("stopped"))
                })
                .filter_map(|s| s["id"].as_str())
                .collect();

            // What the Files screen is showing. A directory listing is the thing an agent
            // most often needed and could not get without photographing the window.
            let files = if screen == 8 {
                use slint::Model;
                let entries = ui.get_file_browser_entries();
                let total = entries.row_count();
                let listing: Vec<serde_json::Value> = (0..total.min(FILE_LISTING_CAP))
                    .filter_map(|i| entries.row_data(i))
                    .map(|e| {
                        let mut entry = serde_json::json!({
                            "name": e.name.to_string(),
                            "dir": e.is_dir,
                            "size": e.size_text.to_string(),
                            "modified": e.modified_text.to_string(),
                            "changed": e.changed_text.to_string(),
                            "selected": e.selected,
                        });
                        // What the folder tile says it holds. `null` with a reason when the
                        // folder could not be counted — never a 0 that was not read.
                        if e.is_dir {
                            entry["items"] = if e.count_known {
                                e.item_count.into()
                            } else {
                                serde_json::Value::Null
                            };
                            if !e.count_known {
                                entry["items_reason"] = e.count_reason.to_string().into();
                            }
                        }
                        entry
                    })
                    .collect();
                // The row under the grid: this folder's most recently changed files.
                let recent: Vec<serde_json::Value> = {
                    let recent = ui.get_file_recent();
                    (0..recent.row_count())
                        .filter_map(|i| recent.row_data(i))
                        .map(|r| {
                            serde_json::json!({
                                "name": r.name.to_string(),
                                "size": r.size_text.to_string(),
                                "changed": r.changed_text.to_string(),
                            })
                        })
                        .collect()
                };
                let places: Vec<serde_json::Value> = {
                    let places = ui.get_file_places();
                    (0..places.row_count())
                        .filter_map(|i| places.row_data(i))
                        .map(|p| serde_json::json!({ "label": p.label.to_string(), "path": p.path.to_string() }))
                        .collect()
                };
                let sel = ui.get_file_selected_index();
                let selected = if sel >= 0 {
                    entries
                        .row_data(sel as usize)
                        .map(|e| serde_json::Value::String(e.name.to_string()))
                        .unwrap_or(serde_json::Value::Null)
                } else {
                    serde_json::Value::Null
                };
                serde_json::json!({
                    "path": ui.get_file_browser_path().to_string(),
                    "entries": listing,
                    "shown": total.min(FILE_LISTING_CAP),
                    "total": total,
                    "selected": selected,
                    "selection_count": ui.get_file_selection_count(),
                    "loading": ui.get_file_browser_loading(),
                    "notice": ui.get_file_notice().to_string(),
                    "operation_busy": ui.get_file_operation_busy(),
                    "operation": ui.get_file_operation_text().to_string(),
                    "operation_progress": ui.get_file_operation_progress(),
                    "trash": ui.get_file_trash_mode(),
                    "can_undo": ui.get_file_can_undo(),
                    "has_clipboard": ui.get_file_has_clipboard(),
                    "preview_open": ui.get_file_quick_look_open(),
                    "preview_name": ui.get_file_quick_look_name().to_string(),
                    "tabs": ui.get_file_tabs().row_count(),
                    "free_space": ui.get_file_free_space_text().to_string(),
                    "view": if ui.get_file_grid_view() { "grid" } else { "list" },
                    "recent": recent,
                    "places": places,
                })
            } else {
                serde_json::Value::Null
            };

            // The wizard, when the wizard is up. This is the screen an agent is most likely to
            // meet first and, until it published anything, the only one it could not read.
            let installer = if screen == 2 {
                crate::control_installer::state(&ui)
            } else {
                serde_json::Value::Null
            };

            // The open document, when the editor is up — so an agent reads what it is editing
            // the same way it reads a directory or the weather.
            let editor = if screen == 12 {
                crate::control_editor::state(&ui)
            } else {
                serde_json::Value::Null
            };

            // The one line worth reading first: where the user is, what is open, and whether
            // anything is wrong. Trouble comes before window count, because trouble is the
            // reason to look.
            let summary = if screen == 2 {
                crate::control_installer::summary(&ui)
            } else if screen == 12 {
                crate::control_editor::summary(&ui)
            } else if !down.is_empty() {
                format!(
                    "Yantrik — {} screen, {} windows open, {} not running",
                    screen_name(screen),
                    open.len(),
                    list_of(&down)
                )
            } else if screen == 8 {
                // On the file screen the directory IS the answer to "where am I".
                format!(
                    "Yantrik — files at {}, {} items, {} windows open",
                    ui.get_file_browser_path(),
                    files["total"].as_u64().unwrap_or(0),
                    open.len()
                )
            } else {
                format!(
                    "Yantrik — {} screen, {} windows open, CPU {}%, memory {}",
                    screen_name(screen),
                    open.len(),
                    ui.get_bar_cpu_percent(),
                    ui.get_bar_mem_text()
                )
            };

            // Launches that died before they became a window. `open_app` defers, so it answers
            // "accepted" long before anything is on screen -- and when the app then exits, the
            // only account of it was a log line nobody reads. An agent that launched something
            // and sees nothing needs to be told why here, in the same place it reads everything
            // else.
            let failed: Vec<serde_json::Value> = crate::running::launch_failures()
                .into_iter()
                .map(|f| {
                    serde_json::json!({
                        "app": f.app_id,
                        "binary": f.binary,
                        "status": f.status,
                        "lived_ms": f.lived_ms,
                    })
                })
                .collect();

            // What was said. Roles and text, newest last, so a caller that asked a question can
            // read the answer instead of photographing it.
            let conversation: Vec<serde_json::Value> = {
                use slint::Model;
                let messages = ui.get_messages();
                let total = messages.row_count();
                (total.saturating_sub(CONVERSATION_TAIL)..total)
                    .filter_map(|i| messages.row_data(i).map(|m| (i, m)))
                    .map(|(i, m)| {
                        let text = m.content.to_string();
                        let mut entry = serde_json::json!({
                            // Good for as long as the transcript is: rows are only ever
                            // appended. `read_message` takes it.
                            "index": i,
                            "role": m.role.to_string(),
                            "text": clip(&text, MESSAGE_CLIP),
                            // Still arriving. A caller polling for an answer needs to know the
                            // difference between "this is the reply" and "this is the reply so
                            // far", and an empty streaming bubble is the normal first state.
                            "streaming": m.is_streaming,
                        });
                        // Said outright, so a caller does not have to notice an ellipsis in
                        // the text to know there is more, and knows where the rest is.
                        if text.chars().count() > MESSAGE_CLIP {
                            entry["clipped"] = true.into();
                        }
                        // The mind's tool calls, with their arguments — the same reading the
                        // panel gives them, so what the person sees and what a caller reads
                        // are one thing.
                        let calls = calls_of(&text);
                        if !calls.is_empty() {
                            entry["calls"] = serde_json::Value::Array(calls);
                        }
                        entry
                    })
                    .collect()
            };

            View::new(summary)
                .with("screen", screen_name(screen))
                .with("conversation", serde_json::Value::Array(conversation))
                .with("screen_id", screen)
                // Which build is answering. The report this came from asked a machine three
                // times what it was and got three answers, one of them months old; an agent
                // writing that report should be able to read the version off the same describe
                // it reads everything else off, rather than knowing which file to trust.
                .with("version", yantrik_version::version())
                // What the desktop draws with and why: "gpu" or "software", who decided (the
                // session's probe, a person's override, a fallback after the GPU failed in use),
                // the renderer Mesa reported, and the fallback record if there is one. Without it
                // "is this machine using its GPU" was a question for `ps` and /proc/PID/maps.
                .with("graphics", crate::render_backend::for_describe())
                .with("windows", serde_json::Value::Array(open))
                .with("failed_launches", serde_json::Value::Array(failed))
                // What is waiting on a person right now. Published so a second mind, or a
                // test, can tell "the machine is waiting for someone to press a button" from
                // "the machine is hung" — the two look identical from outside otherwise.
                .with("pending_approvals", crate::control_approvals::pending_for_describe())
                // What went wrong on this machine, newest first: the local records a person
                // or a mind can choose to send with `report_problem`. Reading them sends nothing.
                .with("problems", crate::wire::problem_report::for_describe())
                // Every agent, one conversation with one mind: its state, what it has run and
                // what it is waiting on, with the counts the Agents screen's tabs show.
                .with("agents", crate::agents::for_describe())
                // The agent catalog: the roles `hand_off` can start, what each may touch, and
                // whether a mind it runs on is attached now. See `agents::catalog`.
                .with("catalog", crate::agents::catalog::for_describe())
                // Every recipe the companion holds that is not a never-run built-in: its status,
                // the step it is on and what it waits for. `answer_recipe` and its siblings act.
                .with("recipes", crate::recipes::for_describe())
                // The owner's standing policy for callers on the socket, so a bridge can read
                // it instead of provoking a `CEILING:` refusal to find out. An approval cannot
                // exceed this, and a question the machine will refuse to answer should never
                // reach the person.
                .with("tool_permission", crate::control_approvals::machine_ceiling())
                // What the mind may do without being asked, and the rules a person has granted
                // for this session. The bridge takes this off the SAME read as the ceiling above
                // and makes the run/ask/refuse decision from it, so one `describe shell` answers
                // every question an `os_act` has to ask before it runs.
                .with("mind_mode", crate::control_approvals::mind_mode_for_describe())
                // And what it has already done unasked. A mode that stops the asking has to
                // replace the cards with something, or `auto` is only a quieter way of not
                // knowing. See `mind_mode`'s audit section.
                .with("mind_audit_recent", crate::control_approvals::mind_audit_for_describe())
                // The mind panel at the right edge: where it is, whether it is open, the choice
                // each place keeps, and how much it is showing. `set_mind_panel` changes it.
                .with("mind_panel", crate::mind_panel::for_describe(&ui))
                // Whether the credential vault is actually protected, and when it is not, why.
                //
                // Published because the honest answer on most machines is "no", and a machine
                // that quietly stores credentials under a key sitting in the same file is a
                // machine that has told nobody. `why` is the whole point of the key: an agent or
                // a person reading this should get "this session signs in without a password, so
                // there is nothing to lock the vault with" rather than a bare false they have to
                // interpret. Never carries the passphrase, or anything derived from one — see
                // `secret_never_reaches_a_message` in vault_unlock.rs.
                .with("vault", crate::vault_unlock::cached_status().to_json())
                // Which mind is answering, and what else could. An agent that can switch this
                // has to be able to see it first, and without the list it would be guessing at
                // ids for `use_harness`.
                // What is on START, in order. The person's choice, so an agent can read it
                // before proposing to change it.
                .with("pinned", crate::wire::settings::pinned_apps())
                // What `open_app` accepts. It took a name and this state offered none.
                .with("apps", serde_json::Value::Array(crate::wire::dock::openable()))
                .with(
                    "minds",
                    crate::wire::harness::host()
                        .map(|host| {
                            serde_json::Value::Array(
                                host.list()
                                    .iter()
                                    .map(|e| {
                                        serde_json::json!({
                                            "id": e.id,
                                            "name": e.name,
                                            "answering": e.active,
                                            "builtin": e.builtin,
                                            // What the mind said about itself when it attached:
                                            // its backend, its memory, wherever it is running.
                                            // The OS knows none of that on its own and does not
                                            // want to — this is the harness's own account.
                                            "detail": e.detail,
                                            "tools": e.capabilities.tools,
                                        })
                                    })
                                    .collect(),
                            )
                        })
                        .unwrap_or(serde_json::Value::Array(Vec::new())),
                )
                // What this machine could have, whether or not it is running — `minds` above is
                // only what has attached. Published because the question "why can I not talk to
                // Pi" has an answer the machine knows and nothing could read: it is not
                // installed, or its config file was never written, or its unit is stopped. An
                // agent asked to fix that needs the same list the Settings page draws.
                .with("harnesses", crate::wire::harness::catalogue_for_describe())
                // Agents' commands running now, per agent: the job, its command, how long, and
                // whether it seems to be waiting for input. See `control_agent_terminal`.
                .with("agent_jobs", crate::control_agent_terminal::for_describe())
                .with("files", files)
                .with("installer", installer)
                .with("editor", editor)
                .with("services", serde_json::Value::Array(services))
                .with("companion_online", ui.get_companion_online())
                .with("companion_status", ui.get_companion_status().to_string())
                .with("thinking", ui.get_is_thinking())
                .with("pending_suggestions", ui.get_pending_count())
                .with("memories", ui.get_memory_count())
                // Null, all three, until the worker has pushed the store once — see
                // `describe_bond`. `bond_loaded` says which of the two a reader is looking at.
                .with("bond_loaded", bond.loaded)
                .with("bond", bond.level)
                .with("bond_score", bond.score)
                // The count as well as the score: the score caps at 5.0, and on a machine
                // that reached it a reader has no other way to see a turn being counted.
                .with("bond_interactions", bond.interactions)
                .with("active_project", ui.get_active_project().to_string())
                .with("clock", ui.get_clock_text().to_string())
                .with("date", ui.get_date_text().to_string())
                .with("cpu_percent", ui.get_bar_cpu_percent())
                .with("memory", ui.get_bar_mem_text().to_string())
                .with("memory_percent", ui.get_bar_mem_percent())
                .with("disk", ui.get_bar_disk_text().to_string())
                .with("disk_percent", ui.get_bar_disk_percent())
                .with("wifi", ui.get_wifi_connected())
                // `wifi` alone was the whole of what the desktop said about
                // its network, and on the wired test machine it is false —
                // so a mind reading this concluded there was no network on a
                // machine that was online the entire time. What is up, and
                // over what, is the network service's answer, as the status
                // bar and the System screen show it.
                .with(
                    "network",
                    serde_json::json!({
                        "online": ui.get_network_online(),
                        "type": ui.get_network_medium().to_string(),
                        "connection": ui.get_network_detail().to_string(),
                    }),
                )
                .with(
                    "battery",
                    if ui.get_battery_available() {
                        serde_json::json!({
                            "percent": ui.get_battery_level(),
                            "charging": ui.get_battery_charging(),
                        })
                    } else {
                        serde_json::Value::Null
                    },
                )
                .with("do_not_disturb", ui.get_dnd_mode())
                // What the machine is trying to tell the person, so that "is anything waiting
                // for me" is a read of the shell rather than a second call to the notifications
                // service — and so a mind can see what it has already said.
                .with("notifications", crate::wire::notifications::describe_summary())
                // The ask bar, so "is the Lens up, and what is in it" is a read rather than a
                // screenshot. `open_lens` answers from these same two properties.
                .with(
                    "lens",
                    serde_json::json!({
                        "open": ui.get_lens_open(),
                        "text": ui.get_lens_input_text().to_string(),
                        "chat": ui.get_lens_chat_mode(),
                    }),
                )
                // The launcher, for the same reason. `open_app name=launchpad` opened it under
                // whatever window was in front, and nothing a caller could read said it was
                // open at all: `failed_launches` was empty because nothing had failed.
                .with("launcher", serde_json::json!({ "open": ui.get_app_grid_open() }))
                // The mode menu over the status bar's chip, for the same reason again:
                // `show_mind_audit` opens it, and a caller that opened it has to be able to see
                // that it is still there and put it away (`close_mind_menu`) (#184).
                .with(
                    "mind_menu",
                    crate::control_approvals::mind_menu_for_describe(
                        ui.get_mind_menu_open(),
                        ui.get_mind_menu_audit_open(),
                        ui.get_mind_menu_confirming(),
                        screen,
                    ),
                )
                .with("incognito", ui.get_settings_incognito_mode())
                .with("settings", serde_json::json!({"category":ui.get_settings_category(),"query":ui.get_settings_query().to_string(),"dark":ui.get_settings_dark_mode(),"accent":ui.get_settings_accent_color().to_string(),"wallpaper":ui.get_wallpaper_path().to_string(),"save_error":ui.get_settings_save_error(),"save_status":ui.get_settings_save_status().to_string(),"auto_lock_secs":ui.get_settings_auto_lock_secs()}))
        }
    };

    let weak = ui.as_weak();
    let ui_for = move || weak.upgrade().ok_or_else(|| "the shell is gone".to_string());

    let open_ui = ui_for.clone();
    let report_ui = ui_for.clone();
    let screen_ui = ui_for.clone();
    let focus_ui = ui_for.clone();
    let dnd_ui = ui_for.clone();
    let ask_ui = ui_for.clone();
    let lens_ui = ui_for.clone();
    let pin_ui = ui_for.clone();
    let pin_catalogue = ctx.installed_apps.clone();
    let read_ui = ui_for.clone();
    let panel_ui = ui_for.clone();
    let lock_ui = ui_for;

    let surface = ControlSurface::new("shell")
        .describe(describe)
        .action(
            Action::new(
                "report_problem",
                "Send one of this machine's problem records - a crash or failure this desktop wrote \
                 down, listed under `problems` in describe - to the project's report intake, with a \
                 note. Graded sensitive because the record leaves the machine. It carries no name, \
                 hostname or address; the bytes sent are exactly the record as the file holds it, \
                 which is what the Report a problem screen shows. The answer says where it landed.",
            )
            .risk("sensitive")
            .defers()
            .arg(Param::text("record")
                .describe("The record's file name from `problems`, e.g. 1790120000-yantrik-studio.json. Left out means the newest.")
                .optional())
            .arg(Param::text("note").describe("What was happening, in your words. Optional.").optional()),
            move |args| {
                let ui = report_ui()?;
                let record = args["record"].as_str().unwrap_or("").trim().to_string();
                let note = args["note"].as_str().unwrap_or("").trim().to_string();
                let (path, problem) = crate::wire::problem_report::pick(&record).ok_or_else(|| {
                    "no such problem record; `describe shell` lists the ones there are under `problems`".to_string()
                })?;
                let name = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                crate::wire::problem_report::send_from_ui(&ui, path, problem, note);
                Ok(serde_json::json!({
                    "record": name,
                    "sending": true,
                    "read_back": "the outcome lands on the Report a problem screen's status line; \
                                  describe shell again for `problems`",
                }))
            },
        )
        .action(
            // Deferred, and the handshake with yantrik-mind is what proved it. This returned
            // `settled: true` while the shell still reported "0 windows open" and no app socket
            // had appeared — a driver reading that would report a launch it had only requested.
            //
            // `invoke_launch_app` reaches the dock's callback, and most of its branches
            // `spawn()` a process: the window arrives seconds later, if it arrives at all (a
            // failed spawn is logged, not returned). A couple of branches only switch screens and
            // do settle on return, but the caller cannot tell which branch it took, so the
            // conservative claim is the only honest one.
            Action::new("open_app", "Launch an app, or focus it if it is already running")
                .arg(Param::text("name").describe("App id, e.g. notes, email, terminal, files"))
                .defers(),
            move |args| {
                let ui = open_ui()?;
                let name = args["name"].as_str().unwrap_or_default().trim().to_string();
                if name.is_empty() {
                    return Err("`name` is empty".into());
                }
                // Checked before answering. The dispatch discovers an unknown id too, but only
                // after this function has already reported the launch as under way — and a known
                // app whose program is not installed used to be answered "launching" as well.
                let catalogue = installed.get();
                check_launchable(&name, &catalogue)?;
                // The launcher is drawn by the shell itself and settles on return, so it is
                // answered with what is observed — open, on which screen, raised or not —
                // rather than with "launching" and an empty screen. See `open_launcher`.
                if matches!(
                    crate::wire::dock::route(&name),
                    Some(crate::wire::dock::Launch::Launchpad)
                ) {
                    let mut answer = open_launcher(&ui);
                    answer["launching"] = name.into();
                    return Ok(answer);
                }
                // The launcher's own path: it resolves the binary, enforces one window per app,
                // and focuses the running one instead of starting a second.
                ui.invoke_launch_app(name.clone().into());
                // And the name to describe it by once it is up, which is not always the name it
                // was opened by (`sysmonitor` opens what answers as `system-monitor`).
                let mut answer = serde_json::json!({ "launching": name });
                if let Some(surface) = crate::wire::dock::surface_for(&name, &catalogue) {
                    answer["describe_as"] = surface.into();
                }
                Ok(answer)
            },
        )
        .action(
            // What "on demand" in the machine rail is supposed to mean. calendar, email, notes
            // and perception are registered without autostart, so on a fresh session their
            // sockets do not exist; an app calling one got a connect failure and, in the
            // calendar's case, reported the appointment as saved anyway. Apps now ask for the
            // service first, and the manager that starts it is the same one the rail reads, so
            // a running service is never described as stopped.
            //
            // perception came to this last and is the reason to keep the action general: its
            // caller is not an app but `yos perception`, and so os_perception. The rail called
            // it "stopped, on demand" for every boot of the machine while nothing anywhere
            // supplied the demand, and every mind offered the tool was told "no socket".
            //
            // Standard, not sensitive: this starts one of the machine's own registered
            // services, which is what opening the app that needs it would have done.
            Action::new("start_service", "Start one of the machine's services if it is not running")
                .arg(Param::text("name").describe("Service id, as the machine rail lists it")),
            {
                let services = services.clone();
                move |args: &serde_json::Value| {
                    let name = args["name"].as_str().unwrap_or_default().trim().to_string();
                    if name.is_empty() {
                        return Err("`name` is empty".into());
                    }
                    // Already up is the outcome the caller wanted, not an error to handle.
                    if matches!(
                        services.status(&name),
                        Some(yantrik_shell_core::service_manager::ServiceStatus::Running)
                    ) {
                        return Ok(serde_json::json!({ "service": name, "state": "already running" }));
                    }
                    services.start(&name)?;
                    Ok(serde_json::json!({ "service": name, "state": "started" }))
                }
            },
        )
        .action(
            // The launcher rescans when it opens; this is the same thing without a person
            // having to open it. An agent that installs a package and then wants to run it
            // needs a way to say "look again" that is not "press the Apps button".
            Action::new("refresh_apps", "Rescan the installed applications"),
            {
                let catalogue = refresh_catalogue.clone();
                move |_args| {
                    let count = catalogue.refresh();
                    // And the new surfaces' other names, so `describe <alias>` works at once
                    // rather than on the watcher's next look.
                    crate::surfaces::link_aliases(&catalogue.get());
                    Ok(serde_json::json!({ "apps": count }))
                }
            },
        )
        .action(
            // Parity with the pin on every tile in All apps. Deciding what sits on START is a
            // person's call, and an agent tidying a desktop on someone's behalf needs the same
            // verb rather than a way to fake the click.
            Action::new("pin_app", "Pin an app to START, or unpin it")
                .arg(Param::text("name").describe("App id, e.g. notes, files, browser, chromium"))
                .arg(Param::flag("pinned").describe("true to pin, false to unpin")),
            move |args| {
                let ui = pin_ui()?;
                let name = args["name"].as_str().unwrap_or_default().trim().to_string();
                let want = args["pinned"].as_bool().ok_or("`pinned` must be true or false")?;
                if name.is_empty() {
                    return Err("`name` is empty".into());
                }
                let installed = pin_catalogue.get();
                // Checked, because a pin for something that cannot launch is a START tile that
                // does nothing when clicked — the worst kind of shortcut. Unpinning is always
                // allowed: it is how a person clears a pin for something they removed.
                if want {
                    check_launchable(&name, &installed)?;
                } else if !crate::wire::dock::is_known_app(&name, &installed)
                    && !crate::wire::pins::is_pinned(&name)
                {
                    return Err(format!("no app `{name}` on this machine"));
                }
                if !crate::wire::pins::is_pinnable(&name) {
                    return Err(format!(
                        "`{name}` is the launcher, and its button is already on the taskbar"
                    ));
                }
                if crate::wire::pins::is_pinned(&name) != want {
                    crate::wire::pins::toggle(&name);
                }
                crate::wire::pins::publish(&ui, &installed);
                Ok(serde_json::json!({
                    "app": crate::wire::pins::pin_id(&name),
                    "pinned": want,
                    "start": crate::wire::settings::pinned_apps(),
                }))
            },
        )
        .action(
            // Talking to the desktop, without a keyboard.
            //
            // Every other verb here moves the shell around; this one uses it. It existed only as
            // a text field, so the one thing the desktop is FOR — asking it something — was the
            // one thing this surface could not do, and proving the mind was reachable meant
            // clicking at pixel coordinates and typing into whatever had focus. That test can
            // fail in silence in four different ways before a single byte reaches a harness.
            //
            // Deferred, because the answer streams: this returns when the question has been
            // asked, not when it has been answered. The answer arrives in `describe` under
            // `conversation`, where a caller can watch `streaming` go false.
            Action::new("send_message", "Ask the desktop something, as if typed into the Lens")
                .arg(Param::text("text").describe("What to say"))
                .defers(),
            move |args| {
                let ui = ask_ui()?;
                let text = args["text"].as_str().unwrap_or_default().trim().to_string();
                if text.is_empty() {
                    return Err("`text` is empty".into());
                }
                // The shell's own callback, not a private path beside it: whatever a person
                // typing gets — the mind picker, the bubbles, the streaming state — this gets
                // too, because it is the same call.
                ui.invoke_send_message(text.clone().into());
                Ok(serde_json::json!({
                    "asked": text,
                    // Named here because it is the whole question this action tends to be
                    // asked in service of: which mind is about to answer.
                    "mind": crate::wire::harness::host()
                        .map(|h| h.active_id())
                        .unwrap_or_else(|| crate::wire::harness::BUILTIN_ID.to_string()),
                }))
            },
        )
        .action(
            // The rest of a long answer.
            //
            // `describe` clips every message to MESSAGE_CLIP characters, which is right for a
            // glance and wrong as the only way to read: a mind's two-thousand-character reply
            // came back as six hundred and "(2017 characters total)", and nothing on this
            // surface returned the rest (#125). This does. `safe`: it reads the transcript the
            // person is already looking at, and changes nothing.
            Action::new(
                "read_message",
                "Read one message of the conversation in full. `describe` clips each to 600 characters and marks the cut ones `clipped`; this returns the whole text, and every tool call in it with its arguments",
            )
            .risk("safe")
            .arg(Param::number("index").describe("The message's `index` from describe's `conversation`")),
            move |args| {
                let ui = read_ui()?;
                let index = args["index"]
                    .as_u64()
                    .or_else(|| args["index"].as_f64().map(|f| f as u64))
                    .ok_or("`index` must be a number")? as usize;
                use slint::Model;
                let messages = ui.get_messages();
                let m = messages.row_data(index).ok_or_else(|| {
                    format!(
                        "no message {index}: the conversation has {} (indexes 0 to {})",
                        messages.row_count(),
                        messages.row_count().saturating_sub(1)
                    )
                })?;
                let text = m.content.to_string();
                Ok(serde_json::json!({
                    "index": index,
                    "role": m.role.to_string(),
                    "text": text,
                    "characters": text.chars().count(),
                    "streaming": m.is_streaming,
                    "calls": calls_of(&text),
                }))
            },
        )
        .action(
            // Opening the ask bar, without asking it anything.
            //
            // `send_message` already puts a question to the desktop, but it asks it and is done —
            // there was no way to leave the Lens standing open in front of a person with a draft
            // in it, which is what "here, have a look at this" is. The desktop advertises Ctrl+K
            // in two places for exactly this, and a compositor keybind needs a verb to call:
            // config/labwc/rc.xml binds Super+K to this action, because that is the only route
            // that works while another app holds the keyboard.
            //
            // `safe`: it shows a panel. Nothing is sent, nothing is spawned, nothing is written.
            //
            // The answer is the OBSERVED state, read back off the shell after the calls, not an
            // `accepted: true` — which was the point of the exercise. The Lens is drawn by the
            // desktop screen and nowhere else, so opening it means going to the desktop first;
            // if that did not take, `lens_open` comes back false and the caller knows.
            Action::new("open_lens", "Open the ask bar (the Lens) and put the cursor in it")
                .risk("safe")
                .arg(
                    Param::text("text")
                        .optional()
                        .describe("Put this in the field, ready to edit. It is NOT submitted — use send_message to ask"),
                ),
            move |args| {
                let ui = lens_ui()?;
                let text = args["text"].as_str().unwrap_or_default().to_string();

                // The Lens lives on the desktop screen. Set-then-invoke, the pair every caller
                // in the shell uses: the property shows the screen, `navigate` loads it.
                if ui.get_current_screen() != 1 {
                    ui.set_current_screen(1);
                    ui.invoke_navigate(1);
                }

                // Prefilled before the panel opens, so the results the Lens builds on open are
                // the results for this text rather than for an empty field.
                if !text.is_empty() {
                    ui.set_lens_input_text(text.clone().into());
                    // What typing it would have done. `lens_query` is the as-you-type search,
                    // not the submit — the field is left for a person to edit or send.
                    ui.invoke_lens_query(text.clone().into());
                }

                ui.set_lens_open(true);
                ui.invoke_open_lens();

                // Whoever calls this is, by construction, somewhere else: Ctrl+K already works
                // when the shell has the keyboard, so this action is what Super+K reaches for
                // from inside another window. The first time it ran on a machine with Notes in
                // front, it answered `lens_open: true` and was right — the Lens had opened,
                // underneath Notes, where nobody could see it or type into it. The shell is an
                // ordinary toplevel to the compositor, so it is asked to come forward the way
                // the taskbar asks for any other window. Off the UI thread: wlrctl is a process.
                std::thread::spawn(|| {
                    match std::process::Command::new("wlrctl")
                        .args(["toplevel", "focus", "title:Yantrik OS"])
                        .status()
                    {
                        Ok(status) if status.success() => {}
                        Ok(status) => tracing::warn!(
                            code = status.code().unwrap_or(-1),
                            "the Lens is open but the shell could not be brought in front of it"
                        ),
                        Err(e) => tracing::warn!(error = %e, "could not run wlrctl to raise the shell"),
                    }
                });

                Ok(serde_json::json!({
                    "lens_open": ui.get_lens_open(),
                    "screen": screen_name(ui.get_current_screen()),
                    "text": ui.get_lens_input_text().to_string(),
                }))
            },
        )
        .action(
            // Parity, deliberately: anything a person can do on the Harnesses screen, an agent
            // can do here. A control surface that could not change which mind is answering would
            // be the one decision on this desktop reserved for the mouse.
            Action::new("use_harness", "Choose which mind answers when the shell is asked something")
                .arg(Param::text("id").describe("Harness id, as `describe shell` lists under `minds`")),
            move |args| {
                let id = args["id"].as_str().unwrap_or_default().trim().to_string();
                if id.is_empty() {
                    return Err("`id` is empty".into());
                }
                let host = crate::wire::harness::host()
                    .ok_or_else(|| "the harness host is not running".to_string())?;
                host.set_active(&id)?;
                // The same memory the Settings screen writes. A choice made here is a choice
                // about the machine, and an agent that switches minds should not have its
                // decision quietly undone by the next restart any more than a person should.
                crate::wire::settings::set_preferred_mind(&id);
                Ok(serde_json::json!({
                    "answering": id,
                    "tools": host.list().iter().find(|e| e.id == id).map(|e| e.capabilities.tools),
                }))
            },
        )
        .action(
            // Sensitive, and the word is meant: it runs the harness's own install command, which
            // fetches software and puts it on this machine. Not dangerous — nothing is erased and
            // nothing is replaced — so it is a card the person answers rather than a refusal.
            //
            // Deferred, because an `npm install -g` takes half a minute and the reply would
            // otherwise be a timeout. What it returns is the command it started; the progress is
            // on the Harnesses page and in `describe shell` under `harnesses`.
            Action::new(
                "install_harness",
                "Run a harness's own install command, so it can become one of this machine's minds",
            )
            .risk("sensitive")
            .defers()
            .arg(
                Param::text("id")
                    .describe("Harness id, as `describe shell` lists under `harnesses`"),
            ),
            move |args| {
                let id = args["id"].as_str().unwrap_or_default().trim().to_string();
                if id.is_empty() {
                    return Err("`id` is empty".into());
                }
                let command = crate::wire::harness::install(&id)?;
                Ok(serde_json::json!({
                    "installing": id,
                    "command": command,
                    "watch": "describe the shell and read harnesses[] — the row says `installing` \
                              until it finishes, then what is still missing",
                }))
            },
        )
        .action(
            // Also sensitive: enabling a user unit is a decision about what this machine runs on
            // every login, not just now.
            Action::new(
                "start_harness",
                "Enable and start a harness's user service, so it attaches and can answer",
            )
            .risk("sensitive")
            .defers()
            .arg(
                Param::text("id")
                    .describe("Harness id, as `describe shell` lists under `harnesses`"),
            ),
            move |args| {
                let id = args["id"].as_str().unwrap_or_default().trim().to_string();
                if id.is_empty() {
                    return Err("`id` is empty".into());
                }
                let command = crate::wire::harness::start(&id)?;
                Ok(serde_json::json!({
                    "starting": id,
                    "command": command,
                    // Attaching is the harness's own move and takes a moment after the unit is
                    // up, so "started" is not "answering" and this says which one it means.
                    "watch": "describe the shell and read harnesses[] — the row says `starting` \
                              until the harness attaches, then `attached`; `use_harness` after that",
                }))
            },
        )
        .action(
            Action::new("show_screen", "Switch the shell to one of its screens")
                .arg(
                    Param::text("screen")
                        .describe("desktop, files, settings, notifications, memory, system, permissions, bond, personality, about, packages, devices, images, editor, media, problems, agents, recipes — or launchpad, the launcher, which opens over the desktop"),
                )
                .arg(
                    Param::text("section")
                        .optional()
                        .describe("For `settings`: appearance, ai, desktop, network, accounts, privacy, system, skills, harnesses"),
                ),
            move |args| {
                let ui = screen_ui()?;
                let want = args["screen"].as_str().unwrap_or_default().trim().to_lowercase();

                // The launcher is not a screen, but `yos ls` listed it among the screens of the
                // desktop, so this is where a caller who read that arrives — and was refused
                // with a list the name is not on. Taken here, the same way `open_app` takes it.
                if want == "launchpad" {
                    let mut answer = open_launcher(&ui);
                    answer["showing"] = "launchpad".into();
                    return Ok(answer);
                }

                // Sending someone to Settings and leaving them to find the section is a chore,
                // not a link — for a person following an instruction and for an agent alike.
                let section = args["section"].as_str().unwrap_or_default().trim().to_lowercase();
                if !section.is_empty() {
                    let id = SETTINGS_SECTIONS
                        .iter()
                        .find(|(name, _)| *name == section)
                        .map(|(_, id)| *id)
                        .ok_or_else(|| {
                            let names: Vec<&str> =
                                SETTINGS_SECTIONS.iter().map(|(n, _)| *n).collect();
                            format!(
                                "no settings section called `{section}`; there is: {}",
                                names.join(", ")
                            )
                        })?;
                    ui.set_settings_category(id);
                }

                let id = SCREENS
                    .iter()
                    .find(|(name, _)| *name == want)
                    .map(|(_, id)| *id)
                    .ok_or_else(|| {
                        let names: Vec<&str> = SCREENS.iter().map(|(n, _)| *n).collect();
                        format!("no screen called `{want}`; there is: {}", names.join(", "))
                    })?;
                // Set and invoke, in that order — the same pair every caller in the shell uses.
                // `navigate` is what loads a screen's data; setting the property alone shows an
                // empty one.
                ui.set_current_screen(id);
                ui.invoke_navigate(id);

                // And then get in front of whatever is covering it.
                //
                // This used to stop at the line above and answer `settled: true`, which was a
                // claim about a screen nobody could see: with an app window open, `show_screen
                // about` changed the screen underneath it and the photograph shows the Text
                // Editor with About peeking out at the edges. The shell is one ordinary
                // fullscreen toplevel to labwc — see the `<margin>` note in
                // config/labwc/rc.xml — and a Wayland client cannot raise itself, so it has to
                // ask, exactly as `control_approvals` asks when a card goes up. Super+D,
                // Super+E and Super+I are `show_screen` too, so the same omission meant those
                // three keys did nothing visible from inside any app.
                //
                // Raised AFTER navigate, so what comes forward is the screen that was asked for
                // rather than the previous one changing in front of the person.
                let mut showing = serde_json::json!({ "showing": want });
                if !section.is_empty() {
                    showing["section"] = section.into();
                }
                match crate::windows::raise_shell() {
                    Ok(()) => showing["raised"] = true.into(),
                    // Not an error: the shell IS on that screen, and saying otherwise would
                    // undo a navigation that happened. What it is not is visible, and a caller
                    // that has just been told "shown" is owed that difference.
                    Err(why) => {
                        showing["raised"] = false.into();
                        showing["note"] = format!(
                            "the shell is on `{want}`, but its own window could not be brought \
                             to the front, so an app window may still be covering it: {why}"
                        )
                        .into();
                    }
                }
                Ok(showing)
            },
        )
        .action(
            // For the bridge, which brings an app forward when a mind starts acting on it.
            //
            // A mind asked to build slides, book the calendar and start the slideshow did all
            // three — behind the Notes window, which happened to be the last one opened. The
            // deck was built unseen and the slideshow ran where nobody could watch it: acting on
            // an app's surface does nothing to its window, and a Wayland client cannot raise
            // itself. The person who handed the job over should be able to see it being done.
            //
            // Off the UI thread: `wlrctl` is a process, and this surface gives an action three
            // seconds.
            Action::new("show_app", "Bring one of this desktop's open apps to the front")
                .defers()
                .arg(Param::text("name").describe(
                    "The app, by the name you describe it by or open it by, e.g. presentation",
                )),
            move |args| {
                let name = args["name"].as_str().unwrap_or_default().trim().to_string();
                if name.is_empty() {
                    return Err("`name` is empty".into());
                }
                let id = crate::wire::dock::launcher_id(&name);
                let _ = std::thread::Builder::new().name("yos-show-app".into()).spawn({
                    let id = id.clone();
                    move || {
                        if !crate::windows::present_app(&id) {
                            tracing::info!(app = %id, "show_app: no window by that app's title is open");
                        }
                    }
                });
                Ok(serde_json::json!({ "showing": name }))
            },
        )
        .action(
            // Deferred for the same reason as `open_app`: focus is the compositor's to grant,
            // not ours to assert — we do not own labwc, and `wlrctl toplevel focus` returns as
            // soon as the request has been sent rather than when the window is in front.
            Action::new(
                "focus_window",
                "Bring an open window to the front. `Yantrik OS` is the desktop itself",
            )
            .defers()
            .arg(Param::text("title").describe("Window title, or part of one")),
            move |args| {
                let ui = focus_ui()?;
                let want = args["title"].as_str().unwrap_or_default();
                // The same list `describe` publishes under `windows`, plus the shell's own
                // toplevel. It used to read the Slint window-switcher model, which leaves the
                // shell out so that the taskbar does not offer to switch you to the desktop you
                // are already on — a good reason there and the wrong one here, because the
                // desktop is the one window a caller cannot reach any other way.
                // `focus_window title=Yantrik` answering "no open window matches" with the shell
                // plainly running is what that cost.
                let open = crate::windows::addressable_titles();
                let title = crate::windows::window_named(want, &open)?;
                ui.invoke_switch_window(title.clone().into());
                Ok(serde_json::json!({ "focused": title }))
            },
        )
        .action(
            // There is a × on every window and nothing on this surface could press it.
            //
            // A person driving the desktop through `yos`, and a mind tidying up after a job, both
            // ended a session with every window they had opened still open, because the surface
            // could launch an app and focus it and never close it. So: the compositor's own close
            // request, which is the × exactly — the app is told the person wants it gone and
            // decides what to do about it. An editor holding unsaved work puts up its own dialog
            // and stays, which is right; nothing here kills a process.
            //
            // Deferred for that reason. `Ok` means the request was delivered, not that the window
            // went, and only `describe shell` can say whether it did.
            Action::new(
                "close_window",
                "Ask an open window to close, as pressing its × does. An app with unsaved work \
                 may put up its own dialog and stay",
            )
            .defers()
            .arg(Param::text("title").describe("Window title, or part of one")),
            move |args| {
                let want = args["title"].as_str().unwrap_or_default();
                let open = crate::windows::addressable_titles();
                let title = crate::windows::window_to_close(want, &open)?;
                crate::windows::close(&title)?;
                Ok(serde_json::json!({
                    "closing": title,
                    "note": "the window was asked to close, which is what pressing × does — an \
                             app with unsaved work may answer with its own dialog and stay. Read \
                             `windows` in `describe shell` to see whether it went.",
                }))
            },
        )
        .action(
            // Out of the way, rather than gone. The other half of what a person does with a
            // window they are not using, and the gentle one: nothing is asked of the app, nothing
            // can be lost, and `focus_window` brings it straight back — `present` un-minimizes
            // before it focuses, so the way back needs no second verb.
            //
            // Deferred because the compositor decides, like every other window verb here.
            Action::new(
                "minimise_window",
                "Put an open window out of the way without closing it. `focus_window` brings it \
                 back",
            )
            .defers()
            .arg(Param::text("title").describe("Window title, or part of one")),
            move |args| {
                let want = args["title"].as_str().unwrap_or_default();
                let open = crate::windows::addressable_titles();
                let title = crate::windows::window_named(want, &open)?;
                crate::windows::minimise(&title)?;
                Ok(serde_json::json!({
                    "minimised": title,
                    // Said because it surprises: a minimized toplevel is still a toplevel, so it
                    // is still in `describe shell`, and a caller checking the list to confirm
                    // would otherwise read that as the action having done nothing.
                    "note": "it is still open and still listed under `windows`; it is behind \
                             everything else now. `focus_window` brings it back.",
                }))
            },
        )
        .action(
            // The write is the action, and a failed write is a failed action.
            //
            // `settled` is not a field a handler fills in: this surface computes it as
            // `!deferred`, and the shell publishes this one as "settles on return", so every Ok
            // out of here is a promise that the settings file has been written. It used to press
            // the settings screen's own toggle and return the flag it had been handed. That
            // toggle does write — the audit's machine writes `dnd_mode` on every flip — but it
            // calls `persist` and drops the `Result`, which is right for a row with a "Not
            // saved" line under it and wrong here. A read-only settings file, a file changed
            // underneath the shell, existing values the shell refuses to overwrite: each of
            // those came back to a caller as a durable setting.
            //
            // So the file first, the error propagated, the screen after. An error out of here
            // means the shell is exactly as the caller found it.
            Action::new("set_do_not_disturb", "Hold or release notifications. Stays after a restart")
                .arg(Param::flag("on")),
            move |args| {
                let ui = dnd_ui()?;
                let on = args["on"].as_bool().ok_or("`on` must be true or false")?;
                crate::wire::settings::set_dnd_mode(on)?;
                ui.set_dnd_mode(on);
                tracing::info!(dnd = on, "Do Not Disturb set, and written to the settings file");
                // Read back out of the preference store, the way `pin_app` answers with the
                // pinned list rather than with the flag it was given.
                Ok(serde_json::json!({
                    "do_not_disturb": crate::wire::settings::dnd_mode(),
                }))
            },
        )
        .action(
            // The mind panel's chevron, for a caller. `safe`: it changes how much of one panel is
            // drawn and remembers that in the panel's own file; nothing is sent, run or granted.
            // Settles on return: the file is written first and the screen after, so an error
            // means the panel is as the caller found it.
            Action::new(
                "set_mind_panel",
                "Open the mind panel at the right edge, or fold it to its strip. Kept across restarts, one choice for the desktop and one for everywhere else",
            )
            .risk("safe")
            .arg(Param::flag("expanded").describe("true to open the panel, false for the strip"))
            .arg(
                Param::text("where")
                    .describe("`desktop` or `elsewhere`. Left out, wherever the shell is now")
                    .optional(),
            ),
            move |args| {
                let ui = panel_ui()?;
                let expanded = args["expanded"].as_bool().ok_or("`expanded` must be true or false")?;
                let place = match args["where"].as_str().map(str::trim).filter(|w| !w.is_empty()) {
                    Some(w) => crate::mind_panel::Place::parse(w)
                        .ok_or_else(|| format!("`where` is `desktop` or `elsewhere`, not `{w}`"))?,
                    None => crate::mind_panel::Place::of_screen(ui.get_current_screen()),
                };
                crate::mind_panel::set(&ui, place, expanded)?;
                // Read back, the way `pin_app` answers with the pinned list.
                Ok(crate::mind_panel::for_describe(&ui))
            },
        )
        .action(
            // Locking is not a view change: the person has to type their way back in. It gets its
            // own action and its own risk rather than hiding inside `show_screen`.
            Action::new("lock", "Lock the session").risk("sensitive"),
            move |_| {
                let ui = lock_ui()?;
                ui.invoke_lock_screen();
                Ok(serde_json::json!({ "locked": true }))
            },
        );

    // The installer and the updater keep their actions in their own modules: they are the
    // riskiest things the shell can be asked to do (one erases a disk, the other replaces every
    // binary and restarts) and they deserve to be read together, not buried at the end of a file
    // about status bars.
    let surface = crate::control_installer::actions(surface, ui);
    let surface = crate::control_update::actions(surface, ui);
    let surface = crate::control_files::actions(surface, ui);
    // Asking the person. Three actions, all `safe`, none of which decides anything — the
    // decision is a button in the Lens. See `control_approvals` for why that split is the
    // whole point.
    let surface = crate::control_approvals::actions(surface, ui);
    // An agent's commands, each in a terminal of its own in its pane — agent_run, agent_job,
    // agent_input, agent_kill. The agent comes from its token, never an argument. See
    // `control_agent_terminal` and design/agents-workspace-2026-09-23.md, decision 3.
    let surface = crate::control_agent_terminal::actions(surface);
    // A recipe's question answered, and a recipe paused, resumed or cancelled — answer_recipe,
    // pause_recipe, resume_recipe, cancel_recipe. See `control_recipes`.
    let surface = crate::control_recipes::actions(surface, ctx.bridge.handle());
    // ── Agents glue: new_agent / send_to_agent / stop_agent / read_agent / show_agent / hand_off —
    // how a mind hands work to another agent, or to a role from the catalog. The caller's agent
    // comes from its token. See `control_agents` and design/agents-workspace-2026-09-23.md,
    // decision 1, and design/desk-and-mind-2026-09-23.md, section 5.
    let surface = crate::control_agents::actions(surface, ui);
    crate::control_editor::actions(surface, ui).serve();
}

#[cfg(test)]
mod screen_table_tests {
    use super::{SCREENS, SETTINGS_SECTIONS, check_launchable, screen_name};
    use std::path::Path;

    /// Elements that wrap a screen rather than being one.
    const CHROME: &[&str] = &[
        "WindowFrame", "Rectangle", "Text", "HorizontalLayout", "VerticalLayout",
        "Image", "TouchArea", "Flickable", "GridLayout", "Timer", "FocusScope",
    ];

    /// `app.slint` decides what a screen id means. This reads it: for each
    /// `if current-screen == N`, the id and the component actually drawn there.
    fn rendered() -> Vec<(i32, String)> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../yantrik-ui-slint/ui/app.slint");
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let lines: Vec<&str> = src.lines().collect();

        let mut out = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let Some(rest) = line.trim().strip_prefix("if current-screen == ") else {
                continue;
            };
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            let Ok(id) = digits.parse::<i32>() else { continue };

            // The first CamelCase element under the branch that is not chrome.
            let mut component = String::new();
            'scan: for l in lines.iter().skip(i).take(30) {
                for token in l.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
                    if token.len() < 4 || !token.starts_with(|c: char| c.is_ascii_uppercase()) {
                        continue;
                    }
                    if CHROME.contains(&token) {
                        continue;
                    }
                    if l.contains(&format!("{token} {{")) || l.contains(&format!("{token}{{")) {
                        component = token.to_string();
                        break 'scan;
                    }
                }
            }
            out.push((id, component));
        }
        out
    }

    fn rendered_ids() -> Vec<i32> {
        rendered().into_iter().map(|(id, _)| id).collect()
    }

    /// Every screen a caller can ask for must be one the shell actually draws.
    ///
    /// `("terminal", 16)` sat in this table pointing at the ABOUT screen. Terminal is 14, and
    /// 14 stopped being rendered when the terminal became its own app binary — so `yos act
    /// shell show_screen screen=terminal` quietly showed you About, and had done for as long
    /// as that was true. The list and the file that gives the numbers meaning were maintained
    /// by different hands and never compared.
    #[test]
    fn every_screen_a_caller_can_ask_for_is_one_the_shell_draws() {
        let rendered = rendered_ids();
        let missing: Vec<String> = SCREENS
            .iter()
            .filter(|(_, id)| !rendered.contains(id))
            .map(|(name, id)| format!("{name} -> {id}"))
            .collect();

        assert!(
            missing.is_empty(),
            "these screens are offered by the control surface but app.slint renders no \
             `if current-screen == N` branch for them:\n  {}\n\n\
             Either the screen was removed (drop it here) or the id is wrong. The ids are \
             defined by app.slint, not by this table.",
            missing.join("\n  ")
        );
    }

    /// One id, one name — in both directions.
    ///
    /// `screen_name` used to carry its own entries alongside SCREENS, and two of them
    /// disagreed with the shell: 21 was reported as "email" when it renders the package
    /// manager, and 27 as "snippets" when it renders the device dashboard. `describe` told
    /// callers which screen they were on, and for those two it was lying.
    #[test]
    fn a_screen_answers_to_exactly_one_name() {
        for (name, id) in SCREENS {
            assert_eq!(
                screen_name(*id), *name,
                "screen {id} is offered as `{name}` but describe() calls it `{}`",
                screen_name(*id)
            );
        }

        let mut ids: Vec<i32> = SCREENS.iter().map(|(_, id)| *id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "two names in SCREENS map to the same screen id");

        let mut names: Vec<&str> = SCREENS.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "the same name appears twice in SCREENS");
    }

    /// The name a caller says must describe the screen that id draws.
    ///
    /// This is the check that would have caught `("terminal", 16)`. The other two would not:
    /// 16 IS rendered, and the name was self-consistent because both directions read the same
    /// table. What was wrong is the only thing neither could see — that the screen drawn at 16
    /// is AboutScreen, and nobody calls that a terminal.
    ///
    /// Matching a name against a component name is a heuristic, so it is deliberately loose:
    /// singular/plural is ignored, and an id whose component could not be parsed is skipped
    /// rather than failed. A loose check that runs beats a strict one that gets deleted.
    #[test]
    fn a_screens_name_matches_what_it_draws() {
        let drawn = rendered();
        let mut wrong = Vec::new();

        for (name, id) in SCREENS {
            let Some((_, component)) = drawn.iter().find(|(rid, c)| rid == id && !c.is_empty())
            else {
                continue; // not parseable from the markup; the other tests still cover the id
            };
            let stem = name.strip_suffix('s').unwrap_or(name).to_ascii_lowercase();
            if !component.to_ascii_lowercase().contains(&stem) {
                wrong.push(format!("`{name}` -> {id}, which draws {component}"));
            }
        }

        assert!(
            wrong.is_empty(),
            "these names do not describe the screen they point at:\n  {}\n\n\
             The ids come from app.slint. If the screen moved, take its id from the \
             `if current-screen == N` branch that renders it.",
            wrong.join("\n  ")
        );
    }

    /// Asking to open a shelved app is refused, and the refusal says why and what would fix it.
    ///
    /// The three refusals have to read differently, because they ask different things of the
    /// caller. "Not installed" means install it. "No app by that name" means try another name.
    /// "Not part of this build" means neither will help — the app is in the tree and nothing on
    /// this machine will produce it — so the refusal carries the reason and what would have to be
    /// built, and an agent reading it can stop instead of retrying four spellings.
    #[test]
    fn opening_a_shelved_app_is_refused_with_its_reason() {
        for name in ["music", "music-player", "Music Player", "spreadsheet", "ySheets"] {
            let err = check_launchable(name, &[]).expect_err("a shelved app must not open");
            assert!(err.contains("not part of this build"), "{name}: {err}");
            assert!(err.contains("shelved"), "{name}: {err}");
            assert!(err.contains("It comes back when"), "{name}: {err}");
            assert!(err.contains("design/shelved-2026-09-20.md"), "{name}: {err}");
            // And the list it offers instead never names the app it has just refused.
            let offered = err.split("It can open: ").nth(1).unwrap_or("");
            assert!(
                !offered.split(", ").any(|id| crate::wire::dock::shelved(id).is_some()),
                "{name} was refused and then offered something shelved: {offered}"
            );
        }
        assert!(check_launchable("files", &[]).is_ok(), "a shipped app still opens");
    }

    /// The settings sections a caller can name are the ones the sidebar has.
    #[test]
    fn settings_sections_are_contiguous_from_zero() {
        let mut ids: Vec<i32> = SETTINGS_SECTIONS.iter().map(|(_, id)| *id).collect();
        ids.sort_unstable();
        let expected: Vec<i32> = (0..ids.len() as i32).collect();
        assert_eq!(
            ids, expected,
            "settings section ids are the sidebar's indices, so they run 0..n with no gaps"
        );
    }
}

#[cfg(test)]
mod do_not_disturb_tests {
    use std::path::Path;

    /// `set_do_not_disturb`'s handler as it is written, taken from the code above the tests: a
    /// check that names what it forbids is worth nothing if it can match itself.
    fn handler() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/control.rs");
        let whole = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let src = whole.split("#[cfg(test)]").next().unwrap_or_default();
        let from = src
            .find("\"set_do_not_disturb\"")
            .expect("the shell still publishes set_do_not_disturb");
        let rest = &src[from..];
        let end = rest.find(".action(").unwrap_or(rest.len());
        rest[..end].to_string()
    }

    /// `settled: true` is a promise about the settings file, so the write has to be the action.
    ///
    /// This action is not deferred, which means the surface answers `settled: true` on every Ok
    /// the handler returns. It used to return Ok after pressing the settings screen's own toggle
    /// — a Slint callback that flips the property, saves, and drops the result — so `settled` was
    /// a claim about the disk that nothing had checked, and every way that save can fail came
    /// back to the caller as a durable setting.
    #[test]
    fn set_do_not_disturb_reports_settled_only_once_the_file_is_written() {
        let handler = handler();
        assert!(
            handler.contains("settings::set_dnd_mode(on)?"),
            "`set_do_not_disturb` must write the preference through \
             `wire::settings::set_dnd_mode` and hand its failure on with `?`. This action is \
             not deferred, so returning Ok answers `settled: true`, and that is a claim about \
             ~/.config/yantrik/settings.yaml rather than about the chip in the status bar. \
             Handler as written:\n{handler}"
        );
        assert!(
            !handler.contains("invoke_toggle_dnd_mode"),
            "`set_do_not_disturb` presses the settings screen's own toggle. That callback \
             flips the property and throws its save result away, which is how this action came \
             to report a durable preference it had never written. Handler as written:\n{handler}"
        );
    }
}

#[cfg(test)]
mod window_action_tests {
    use std::path::Path;

    /// One action's declaration and handler, as written above the tests.
    ///
    /// The same trick `do_not_disturb_tests` uses, and for the same reason: these handlers need a
    /// live Slint window and a compositor to run, so the property worth pinning — that the handler
    /// asks the right thing of the right module — is pinned against the source. The pure parts it
    /// delegates to are tested for real, in `windows.rs`.
    /// This file, without its tests.
    fn source() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/control.rs");
        let whole = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        whole.split("#[cfg(test)]").next().unwrap_or_default().to_string()
    }

    fn action(name: &str) -> String {
        let src = source();
        let from = src
            .find(&format!("\"{name}\""))
            .unwrap_or_else(|| panic!("the shell no longer publishes `{name}`"));
        let rest = &src[from..];
        let end = rest.find(".action(").unwrap_or(rest.len());
        rest[..end].to_string()
    }

    /// A screen the shell reports as shown has to be a screen somebody can see.
    ///
    /// `show_screen about` answered `settled: true` with the Text Editor covering the whole
    /// display and About peeking out at the edges. The navigation was real; the claim was not,
    /// because a Wayland client cannot raise itself and nothing here was asking the compositor to.
    #[test]
    fn show_screen_raises_the_shell_and_says_so_when_it_could_not() {
        let handler = action("show_screen");
        assert!(
            handler.contains("windows::raise_shell()"),
            "`show_screen` must ask the compositor to bring the shell's own toplevel forward. \
             Without it the screen changes underneath whatever app window is in front and the \
             caller is told it was shown. Handler as written:\n{handler}"
        );
        assert!(
            handler.contains("\"raised\""),
            "`show_screen` settles on return, so its answer has to carry whether the shell \
             actually came forward — a caller that reads `settled: true` and sees the Text \
             Editor has been told nothing it can use. Handler as written:\n{handler}"
        );
        assert!(
            handler.contains("showing[\"note\"]"),
            "when the raise fails, the answer has to say so in words: the screen DID change, so \
             this is not an error, but it is not visible either. Handler as written:\n{handler}"
        );
    }

    /// The launcher opened every time, and nobody could see it.
    ///
    /// `open_app name=launchpad` answered "launching"; the log showed the grid opening — the
    /// catalogue rescan it triggers — and the photograph showed Studio, because the grid was
    /// under it and nothing asked the compositor to bring the shell forward. `show_screen
    /// screen=launchpad`, the next thing a caller tries, was refused (#71, #118). Both doors go
    /// through `open_launcher`, which opens the grid by the dock's own arm, raises the shell and
    /// answers with what it sees; and `describe` says whether the launcher is open, so a caller
    /// no longer has to photograph the screen to find out.
    #[test]
    fn opening_the_launcher_raises_the_shell_and_answers_what_it_sees() {
        for name in ["open_app", "show_screen"] {
            let handler = action(name);
            assert!(
                handler.contains("open_launcher(&ui)"),
                "`{name}` must open the launcher through `open_launcher`, which raises the \
                 shell and reports what it observed. Handler as written:\n{handler}"
            );
        }

        let src = source();
        let from = src.find("fn open_launcher(").expect("the shell no longer has `open_launcher`");
        let body = &src[from..];
        let body = &body[..body.find("\n}\n").unwrap_or(body.len())];
        assert!(
            body.contains("invoke_launch_app("),
            "`open_launcher` must open the grid through the dock's own arm, so there is one \
             account of how the launcher opens. As written:\n{body}"
        );
        assert!(
            body.contains("windows::raise_shell()"),
            "`open_launcher` must ask the compositor to bring the shell forward: the grid \
             opens underneath whatever app window is in front. As written:\n{body}"
        );
        assert!(
            body.contains("\"launcher_open\"") && body.contains("\"raised\""),
            "`open_launcher` settles on return, so its answer has to carry whether the \
             launcher is open and whether the shell came forward. As written:\n{body}"
        );
        assert!(
            body.contains("answer[\"note\"]"),
            "when the raise fails the answer has to say so in words: the launcher DID open, so \
             this is not an error, but it is not visible either. As written:\n{body}"
        );

        // And the state a caller reads between calls says whether the launcher is open.
        let describe = &src[..src.find("ControlSurface::new(\"shell\")").unwrap_or(src.len())];
        assert!(
            describe.contains("\"launcher\""),
            "`describe shell` must report the launcher, or an open grid nobody can see leaves \
             no trace a caller can read"
        );
    }

    /// Closing a window is asking the app, never ending the process.
    ///
    /// An app holding unsaved work is entitled to answer with its own dialog and stay open. That
    /// is what the × does and it is the only thing this action may do — `kill`, `pkill` and
    /// `Child::kill` all take that answer away from the person whose work it is.
    #[test]
    fn closing_a_window_asks_the_app_rather_than_killing_it() {
        let handler = action("close_window");
        assert!(
            handler.contains("windows::close(&title)"),
            "`close_window` must go through `windows::close`, which sends the compositor's close \
             request. Handler as written:\n{handler}"
        );
        for killing in ["kill", "SIGTERM", "SIGKILL", "pkill", "terminate"] {
            assert!(
                !handler.contains(killing),
                "`close_window` mentions `{killing}`. Closing a window is a request the app may \
                 refuse — an editor with unsaved work must get its own say. Handler as \
                 written:\n{handler}"
            );
        }
    }

    /// Every window verb resolves what a person typed against the windows that are open.
    ///
    /// Not politeness: wlrctl's `title:` is an exact, case-sensitive comparison, so
    /// `title:editor` matches no window called `Editor` and `title:Yantrik` matches no shell
    /// called `Yantrik OS`. Handing a caller's string straight to wlrctl is a match on nothing,
    /// and for `focus` a match on nothing exits zero.
    #[test]
    fn the_window_verbs_resolve_the_title_before_asking_the_compositor() {
        for name in ["focus_window", "close_window", "minimise_window"] {
            let handler = action(name);
            assert!(
                handler.contains("windows::addressable_titles()"),
                "`{name}` must resolve its `title` against the open windows — which include the \
                 shell's own — before it asks the compositor for anything. Handler as \
                 written:\n{handler}"
            );
        }
    }
}

#[cfg(test)]
mod bond_not_loaded_tests {
    //! Two minutes after a shell restart, `describe shell` said `bond: "Stranger", bond_score:
    //! 0.0`; eighteen minutes later, "Partner-in-Crime", 5.0, 155 interactions, with nothing in
    //! between to explain it. The property is a Slint default until the worker's first push,
    //! and both `describe` and the machine rail were reading the default as the relationship.
    use std::path::Path;

    fn slint(rel: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yantrik-ui-slint/ui").join(rel);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
    }

    /// The Slint side: the default the property starts with, and no screen inventing a level.
    ///
    /// The machine rail's Bond row, which this test used to read too, went with the rail's
    /// companion section when the mind panel replaced it; the bond is on the Bond screen and in
    /// `describe shell`, both of which read `loaded` (the describe half is the test below).
    #[test]
    fn before_the_first_push_nothing_says_stranger() {
        let app = slint("app.slint");
        let start = app
            .find("in property <BondData> bond-data: {")
            .expect("app.slint declares the bond-data property with a default");
        let default = &app[start..start + app[start..].find("};").expect("the default literal ends")];
        assert!(
            default.contains("loaded: false"),
            "the property's default must say it is not loaded. As written:\n{default}"
        );
        assert!(
            !default.contains("Stranger"),
            "the default is not a level anybody measured; it must not name one. As written:\n{default}"
        );

        for file in ["components/mind_panel.slint", "desktop.slint", "bond.slint"] {
            assert!(
                !slint(file).contains("bond-level: \"Stranger\""),
                "{file} still defaults the level to Stranger — the made-up value the rail showed for the window between boot and the first push"
            );
        }
    }

    /// The `describe shell` side.
    #[test]
    fn before_the_first_push_describe_says_not_loaded_not_stranger() {
        // The property as Slint initialises it, before the worker has pushed anything.
        let unloaded = crate::BondData::default();
        let d = super::describe_bond(&unloaded);
        assert!(!d.loaded, "nothing has been pushed, so the bond is not loaded");
        assert!(
            d.level.is_null() && d.score.is_null() && d.interactions.is_null(),
            "and no level, score or count is reported in its place: got {:?} / {:?} / {:?}",
            d.level, d.score, d.interactions
        );

        let pushed = crate::BondData {
            loaded: true,
            bond_level: "Partner-in-Crime".into(),
            bond_score: 5.0,
            total_interactions: 155,
            ..Default::default()
        };
        let d = super::describe_bond(&pushed);
        assert!(d.loaded);
        assert_eq!(d.level, "Partner-in-Crime");
        assert_eq!(d.score, 5.0);
        assert_eq!(d.interactions, 155);
    }
}

#[cfg(test)]
mod conversation_tests {
    use std::path::Path;

    /// This file, without its tests.
    fn source() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/control.rs");
        let whole = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        whole.split("#[cfg(test)]").next().unwrap_or_default().to_string()
    }

    /// `describe` is a window onto the transcript, and a window is fine as long as the surface
    /// also offers the rest. It did not: a 2,017-character reply came back as 600 and a count,
    /// and there was no verb that returned the other 1,417 (#125).
    #[test]
    fn a_message_describe_clipped_can_be_read_in_full() {
        let src = source();
        let from = src.find("\"read_message\"").expect(
            "the shell publishes `read_message`; without it a reply longer than the clip has no way back to the caller",
        );
        let rest = &src[from..];
        let handler = &rest[..rest.find(".action(").unwrap_or(rest.len())];
        assert!(
            handler.contains("row_data(index)"),
            "read_message reads the one message the caller named:\n{handler}"
        );
        assert!(
            !handler.contains("clip("),
            "the whole point of read_message is the whole text; it must not clip:\n{handler}"
        );
        assert!(
            handler.contains("calls_of("),
            "the calls in the message travel with it, with their arguments:\n{handler}"
        );
    }

    /// A caller has to be able to tell a cut message from a short one without parsing an
    /// ellipsis, and has to have the number `read_message` takes.
    #[test]
    fn describe_names_each_message_and_says_which_ones_it_cut() {
        let src = source();
        let describe = src
            .split("ControlSurface::new(\"shell\")")
            .next()
            .expect("describe is built before the surface");
        let conversation = describe
            .split(".with(\"conversation\"")
            .next()
            .expect("describe reports the conversation");
        assert!(conversation.contains("\"index\": i"), "each entry carries its row index");
        assert!(conversation.contains("\"clipped\""), "a cut entry says so");
        assert!(conversation.contains("\"calls\""), "a message's tool calls are read out with it");
    }

    #[test]
    fn clipping_keeps_the_count_so_a_caller_knows_what_it_is_missing() {
        let long = "x".repeat(700);
        assert!(super::clip(&long, 600).ends_with("… (700 characters total)"));
        assert_eq!(super::clip("short", 600), "short");
    }

    #[test]
    fn the_calls_read_out_of_a_message_carry_their_arguments() {
        let calls = super::calls_of(
            "On it.\n⚙️ os_act studio.generate {\"args\":{\"prompt\":\"a red kite\"}}\n\nDone.",
        );
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["name"], "os_act");
        assert_eq!(calls[0]["target"], "studio.generate");
        assert_eq!(calls[0]["arguments"]["args"]["prompt"], "a red kite");
        assert_eq!(calls[0]["summary"], "os_act studio.generate prompt=\"a red kite\"");
    }
}
