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
fn open_launcher(ui: &crate::App) -> Result<serde_json::Value, String> {
    // The launcher is drawn over whatever is on the screen, so it waits for a decision card like
    // every other thing that comes over the shell (card_watch).
    crate::card_watch::hold_windows("open_launcher")?;
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
    Ok(answer)
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
    ("about", 16),
    ("packages", 21),
    ("devices", 27),
    ("permissions", 28),
    ("problems", 33),
    ("agents", 34),
    ("recipes", 35),
];

/// Screens the shell drew for itself until #253, and where each went. A caller that learnt the
/// old names is told that, instead of a list without them.
const BECAME_WINDOWS: &[(&str, &str)] = &[
    ("images", "Images is an app of its own now: open_app name=image"),
    ("editor", "the Editor is an app of its own now: open_app name=editor, then its own surface, `editor`"),
    ("media", "sound and video play in mpv's own window now: open the file from Files"),
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

/// The taskbar entry for the screen the shell is on, as `(name, title)`, when that screen is one a
/// person reads as a window of its own — Files, Settings, Agents — and `None` on the desktop and
/// on the screens nobody switches away from (boot, lock, onboarding, login).
///
/// Those screens are drawn by the shell with a title bar and window buttons, and look exactly like
/// the app windows beside them in the taskbar; but they are not toplevels, so the taskbar, which
/// lists toplevels, never showed them. With Weather in front of Files there was no way back to
/// Files but the launcher, and a person reasonably read that as Files having closed.
pub(crate) fn screen_entry(id: i32) -> Option<(&'static str, String)> {
    let name = SCREENS.iter().find(|(_, s)| *s == id).map(|(n, _)| *n)?;
    if name == "desktop" {
        return None;
    }
    let mut title = name.to_string();
    if let Some(first) = title.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    Some((name, title))
}

/// The app id a screen's taskbar entry carries: no program has it, so the entry can never be
/// taken for a window of one.
pub(crate) const SCREEN_ENTRY_PREFIX: &str = "shell:";

/// Which of the two things `open_app` can do a name does, when it is one of the desktop's own.
///
/// `Some` for a name that is part of the shell rather than a program: the screen it lands on and,
/// for a section of Settings, which section — both in the words `describe shell` uses. Asked of
/// the dock's route table, which is the table the launch itself dispatches on, so the answer and
/// the launch cannot disagree about what just happened. `None` means a window is coming: a
/// program from the catalogue, the browser, Blender, or the launcher, which answers with its own
/// report before this is ever reached.
fn switches_the_shell(name: &str) -> Option<(&'static str, Option<&'static str>)> {
    use crate::wire::dock::Launch;
    match crate::wire::dock::route(name) {
        Some(Launch::Screen(id)) => SCREENS.iter().find(|(_, s)| *s == id).map(|(n, _)| (*n, None)),
        Some(Launch::SettingsSection(id)) => SETTINGS_SECTIONS
            .iter()
            .find(|(_, s)| *s == id)
            .map(|(n, _)| ("settings", Some(*n))),
        _ => None,
    }
}

/// `open_app`'s answer when the name was one of the desktop's own screens.
///
/// Two facts the caller needs and could not see from here: that no window is coming, and whether
/// the switch is in sight. The shell is one ordinary fullscreen toplevel to labwc and cannot
/// raise itself, so a screen switched while an app window is in front was switched underneath it
/// — `show_screen` and `open_launcher` both learned that in #71 and report the raise in these
/// same words. Failing to raise is not an error: the screen did change, it is just covered.
fn shell_screen_answer(
    screen: &'static str,
    section: Option<&'static str>,
    raised: Result<(), String>,
) -> serde_json::Value {
    let mut answer = serde_json::json!({ "showing": screen });
    if let Some(section) = section {
        answer["section"] = section.into();
    }
    match raised {
        Ok(()) => answer["raised"] = true.into(),
        Err(why) => {
            answer["raised"] = false.into();
            answer["note"] = format!(
                "the shell is on `{screen}`, but its own window could not be brought to the \
                 front, so an app window may still be covering it: {why}"
            )
            .into();
        }
    }
    answer
}

/// Whether the screen showing is one the desktop waits behind for the person: the lock screen
/// (3) or the login screen (32).
///
/// The installed machine autologins on tty1 and starts the shell on the login screen, so the
/// session is signed in and that screen is the only thing standing between anybody who can
/// reach this process and the desktop (#203). Locked is therefore a STATE of the shell, read
/// off the screen it is showing — nothing persists it, so a restart during the login screen
/// comes back locked — and every door has to hold to it: the socket's dispatch (the state rule
/// `publish` installs), the toasts that draw
/// over every screen, the command palette and the morning brief's boot timer.
pub(crate) fn locked_screen(screen: i32) -> bool {
    screen == 3 || screen == 32
}

/// The one sentence every refused call gets while the desktop waits for the person. Exact, so
/// a caller can branch on the prefix the way it branches on `GRANT:` or `CEILING:`.
pub(crate) const LOCKED_REFUSAL: &str = "LOCKED: the desktop is waiting for the person to sign in";

/// Whether `action` may run while the desktop is locked.
///
/// An allow-list, and the decision is a pure function so a test can ask it without a window:
/// anything not named here is refused, which means an action added tomorrow is refused by
/// default and its author has to come here — past a reader — to change that.
///
/// It names one action. Neither locked screen needs anything from this surface: both are driven
/// by Slint callbacks (`wire/login.rs` and `on_try_unlock` in `wire/callbacks.rs`), and
/// `describe` is not an action. `lock` is refused too — at the login screen it would trade the
/// password gate for the weaker PIN one.
///
/// `memory_validate` is the exception, and not a way past the lock: it moves nothing on the
/// desktop and is answered to the person's memory server alone (#447). The lock guards the seat,
/// not the person's memory; refusing it would take every mind's memory away whenever the screen
/// locked, a scheduled job's included, and give the person at the seat nothing.
pub(crate) const ALLOWED_WHILE_LOCKED: &[&str] = &["memory_validate"];

pub(crate) fn allowed_while_locked(action: &str) -> bool {
    ALLOWED_WHILE_LOCKED.contains(&action)
}

/// What the dispatch's state rule answers for `action` while `screen` is showing: the refusal
/// when the call must not run, `None` when it passes. Pure, for the same reason.
fn locked_refusal(screen: i32, action: &str) -> Option<String> {
    if locked_screen(screen) && !allowed_while_locked(action) {
        Some(LOCKED_REFUSAL.to_string())
    } else {
        None
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

/// The names `describe shell` puts in its "{...} not running" sentence.
///
/// The candidates are the machine rail's records — a service the manager says failed or
/// stopped — but the record is the manager's account of what *it* started, not the machine's
/// account of what answers. Notes is the standing case: the app keeps its own store and never
/// asks for notes-service, so the record stayed "stopped, on demand" beside an open Notes
/// window and a `yos describe notes` that answered. The headline built from it told every
/// mind, first thing, that notes was not running; Hermes believed it and refused to describe
/// the app at all (#34). So a name leaves the sentence the moment anything answers it: an
/// open window with that app id (`open_apps`), or a live socket (`socket_up`, which the
/// caller extends to the app's own surface — what `yos describe <name>` actually reaches).
/// A name nothing answers is genuinely not running, trouble or not, and stays; the full
/// `services` array keeps carrying the raw records for a caller that wants the manager's side.
fn not_running<'a>(
    services: &'a [serde_json::Value],
    open_apps: &[&str],
    socket_up: impl Fn(&str) -> bool,
) -> Vec<&'a str> {
    services
        .iter()
        .filter(|s| matches!(s["status"].as_str(), Some("failed") | Some("stopped")))
        .filter_map(|s| s["id"].as_str())
        .filter(|id| !open_apps.contains(id) && !socket_up(id))
        .collect()
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
/// Refuse a caller the desktop takes for an agent: what it asked for is the person's own.
pub(crate) fn persons_only(action: &str) -> Result<(), String> {
    if yantrik_app_runtime::control::agent_is_calling() {
        return Err(format!(
            "{action} is the person's, and an agent is calling. An agent reads its own session with read_agent."
        ));
    }
    Ok(())
}

/// `value` with `fields` taken out of every object in it, at any depth: what is the person's in a
/// part of `describe` an agent may otherwise read (the questions other agents put to them, the
/// arguments of what ran unasked, the command lines of every agent's jobs).
fn without_fields(value: serde_json::Value, fields: &[&str]) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .filter(|(k, _)| !fields.contains(&k.as_str()))
                .map(|(k, v)| (k, without_fields(v, fields)))
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(|v| without_fields(v, fields)).collect())
        }
        other => other,
    }
}

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
    crate::control_approvals::wire(ui, ctx.bridge.clone());

    // Private mode, as `main` read it back before anything started: the chip says so from the
    // first frame, and the desktop is made what it says — agents frozen, turns paused, nothing
    // recorded. Agents are refused by the file itself (`yantrik_ipc_transport::privacy`) from
    // boot. And the file is kept the shell's word from here on.
    let private = crate::private_mode::is_on();
    ui.set_private_mode(private);
    crate::private_mode::enforce(private, &ctx.bridge);
    crate::private_mode::watch();

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

            // A locked desktop still answers — a caller has to be able to learn that the
            // machine is waiting for its person rather than hung — but about nothing else.
            // The conversation, the window titles, the notifications, the agents and the rest
            // are the person's, and the login screen exists precisely so that whoever is
            // standing at the machine is not told them (#203). Reading is free, so this cannot
            // be enforced by refusing the call; it is enforced by having nothing to say.
            if locked_screen(screen) {
                return View::new(format!(
                    "Yantrik — {} screen, waiting for the person",
                    screen_name(screen)
                ))
                .with("locked", true)
                .with("screen", screen_name(screen))
                .with("screen_id", screen);
            }

            let bond = describe_bond(&ui.get_bond_data());

            // From the launch registry, not the Slint window-list model. The model is only
            // refreshed while the desktop screen is showing, so a describe from any other screen
            // reported "0 windows open" even with apps running — the registry is refreshed by
            // launches and exits, not by which screen is up, so it is right everywhere.
            // Held as a list, not just as the published array: the summary below asks it which
            // apps are standing open, because a window answers for its service's name.
            let windows = crate::windows::shell_windows();
            let open: Vec<serde_json::Value> = windows
                .iter()
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

            // Which names the record says are down, minus the ones the machine itself
            // contradicts: an open window, or a socket that answers — the service's own, or
            // the app's surface, which is what a caller reaches when it describes the app.
            // See `not_running` for why the record alone lied (#34).
            let open_apps: Vec<&str> = windows.iter().map(|w| w.app_id.as_str()).collect();
            let down: Vec<&str> = not_running(&services, &open_apps, |id| {
                yantrik_app_runtime::service::is_up(id)
                    || yantrik_app_runtime::service::is_up(&format!("app-{id}"))
            });

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
            // A mind is not shown a folder it could not have opened: the person may have left
            // Files in /etc or ~/.ssh, and the listing, the recent row and even the folder's
            // name are what the file tools would never give it (#443). The places, the view and
            // whether something is loading say nothing about the folder, and stay.
            let files = match (screen == 8).then(|| crate::control_files_mind::hidden_here(&ui.get_file_browser_path())).flatten() {
                Some(hidden) => crate::control_files_mind::hide_folder(files, hidden),
                None => files,
            };

            // The wizard, when the wizard is up. This is the screen an agent is most likely to
            // meet first and, until it published anything, the only one it could not read.
            let installer = if screen == 2 {
                crate::control_installer::state(&ui)
            } else {
                serde_json::Value::Null
            };

            // The one line worth reading first: where the user is, what is open, and whether
            // anything is wrong. Trouble comes before window count, because trouble is the
            // reason to look.
            let summary = if screen == 2 {
                crate::control_installer::summary(&ui)
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
                    files["path"].as_str().unwrap_or("a folder hidden from a mind"),
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

            // Whether the one reading this is an agent the desktop can tell apart from the person:
            // the mind account, by the kernel's word (`describe` carries no token). The person's
            // chat and what they asked each agent are theirs, not every mind's — a mind reading
            // `describe` used to be handed the last six messages of the person's chat with another
            // mind, and every agent's first prompt, commands and files (yantrik-mind-72's review).
            // An agent reads its own session with `read_agent`.
            let agent_reading = yantrik_app_runtime::control::agent_is_calling();

            // What was said. Roles and text, newest last, so a caller that asked a question can
            // read the answer instead of photographing it. The person's, so not an agent's.
            let conversation: Vec<serde_json::Value> = if agent_reading { Vec::new() } else {
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
                // What the answering mind says it is doing now ("Thinking… (60 s)"), while its
                // turn runs; empty otherwise. The chat's work card shows the same line.
                // Withheld from an agent caller with the conversation: the status line is the
                // person's chat in miniature. Flattened and clipped so it cannot bloat `describe`.
                .with(
                    "conversation_status",
                    if agent_reading { String::new() } else { crate::wire::agents::status_for_describe(&crate::wire::agents::latest_status()) },
                )
                // True when the conversation above was withheld: the chat is the person's, and an
                // agent reads its own session with `read_agent`.
                .with("conversation_private", agent_reading)
                .with("screen_id", screen)
                // Which build is answering. The report this came from asked a machine three
                // times what it was and got three answers, one of them months old; an agent
                // writing that report should be able to read the version off the same describe
                // it reads everything else off, rather than knowing which file to trust.
                .with("version", yantrik_version::version())
                .with("windows", serde_json::Value::Array(open))
                // The grounded dock as drawn: its buttons in order (app, running, windows,
                // focused), which page of them is showing, and whether a mind is waiting on the
                // person (the amber dot). What a pointer reads off the bar, a mind reads here.
                .with("dock", crate::wire::dock_bar::for_describe(&ui))
                // Which of them the person is looking at. The list's order is not that answer —
                // the registry's launches come first. `desktop_in_front` is true for the desktop
                // itself (or one of its screens), false for an app window, null when the
                // compositor could not say; `in_front` is that app window's title from `windows`.
                .with("desktop_in_front", crate::windows::shell_in_front())
                .with("in_front", crate::windows::in_front())
                .with("failed_launches", serde_json::Value::Array(failed))
                // Where the apps a mind opens are drawn (#239): whether minds open them in Mind
                // View, whether it is up and on which display, what is in it, and why not if it
                // could not start. Those apps are not in `windows` — they are not on this desktop.
                .with("mind_view", crate::mind_view::for_describe())
                // What is waiting on a person right now. Published so a second mind, or a
                // test, can tell "the machine is waiting for someone to press a button" from
                // "the machine is hung" — the two look identical from outside otherwise.
                .with("pending_approvals", crate::control_approvals::pending_for_describe())
                // And what an agent has asked the person (#25), waiting for an answer on its card.
                .with("pending_questions", {
                    let q = crate::wire::agents::questions_for_describe();
                    if agent_reading { without_fields(q, &["prompt", "options"]) } else { q }
                })
                // What went wrong on this machine, newest first: the local records a person
                // or a mind can choose to send with `report_problem`. Reading them sends nothing.
                .with("problems", crate::wire::problem_report::for_describe())
                // Every agent, one conversation with one mind: its state, what it has run and
                // what it is waiting on, with the counts the Agents screen's tabs show.
                .with(
                    "agents",
                    if agent_reading { crate::agents::for_describe_by_an_agent() } else { crate::agents::for_describe() },
                )
                // The Agents workroom as the screen says it: the page, the counts, each mind's
                // state — and, for the person's reader, the desks and the requests waiting.
                // `show_workroom` changes the page.
                .with("workroom", crate::wire::agents::workroom_for_describe(&ui, agent_reading))
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
                // Whether a test run has approvals off: every request that would raise a card is
                // refused and nothing is shown. Ends by itself; see `never_ask`.
                .with("approvals_off_for_test", crate::never_ask::snapshot())
                // And what it has already done unasked. A mode that stops the asking has to
                // replace the cards with something, or `auto` is only a quieter way of not
                // knowing. See `mind_mode`'s audit section.
                .with("mind_audit_recent", {
                    let audit = crate::control_approvals::mind_audit_for_describe();
                    if agent_reading { without_fields(audit, &["args", "verified"]) } else { audit }
                })
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
                .with("agent_jobs", {
                    let jobs = crate::control_agent_terminal::for_describe();
                    if agent_reading { without_fields(jobs, &["command"]) } else { jobs }
                })
                .with("files", files)
                .with("installer", installer)
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
                // The time told in full — date, weekday, time, UTC offset, zone — so a mind
                // learning what day it is is a read rather than an act: `agent_run date` is
                // graded sensitive, and "what's on my calendar today" used to raise an
                // approval card just to answer it (#207). Under `clock` rather than a key of
                // its own: `yos` renders the state sorted and minds condense a description
                // to its header plus the first ~900 characters, so a separate key sorting
                // after `conversation` was clipped out of what a mind ever saw, and `clock`
                // sorts near the top. The string the top bar drew ("11:55") is the object's
                // `time`; nothing read it as a string, and it carried no year, offset or
                // zone — "today" cannot be worked out without them.
                .with("clock", crate::app_context::clock_for_describe())
                .with("date", ui.get_date_text().to_string())
                .with("cpu_percent", ui.get_bar_cpu_percent())
                .with("memory", ui.get_bar_mem_text().to_string())
                .with("memory_percent", ui.get_bar_mem_percent())
                .with("disk", ui.get_bar_disk_text().to_string())
                .with("disk_percent", ui.get_bar_disk_percent())
                // `wifi` is the older field and stays for one release: the minds' tools and older
                // yos-mcp read it. It is true only for a Wi-Fi connection, so on a wired machine
                // it is false while the machine is online; `network` says the rest.
                .with("wifi", ui.get_wifi_connected())
                // The machine's own levels, as Quick Settings shows them. `null` / `available:
                // false` rather than a number when the machine has no audio server or no
                // backlight (a VM has neither a panel nor, sometimes, a mixer).
                .with(
                    "audio",
                    crate::control_levels::audio_for_describe(
                        ui.get_volume_available(),
                        ui.get_volume_level(),
                        ui.get_volume_muted(),
                    ),
                )
                .with(
                    "brightness",
                    crate::control_levels::brightness_for_describe(
                        ui.get_brightness_available(),
                        ui.get_brightness_level(),
                    ),
                )
                // What the bar's network mark shows, from the one NetworkManager reading: kind
                // (wired | wifi | none), state, ssid, strength, ip, vpn, connectivity. `online`,
                // `type` and `connection` are the keys it had before, kept for one release.
                // See `control_network`.
                .with(
                    "network",
                    crate::control_network::network_for_describe(yantrik_os::network::latest().as_ref(), &ui),
                )
                // The visible networks, only where there is a Wi-Fi device (null otherwise, like
                // `battery`). `connect_wifi` joins one that is open or saved.
                .with(
                    "wifi_networks",
                    crate::control_network::wifi_networks_for_describe(yantrik_os::network::latest().as_ref()),
                )
                // `null` with no battery. State, the times and the profile come from
                // `power_status`, which also composes the bar's words, so the two cannot drift.
                .with(
                    "battery",
                    crate::power_status::battery_for_describe(
                        ui.get_battery_available(),
                        ui.get_battery_level(),
                        crate::power_status::state_from_key(ui.get_battery_state().as_str()),
                        ui.get_battery_time_to_empty_mins(),
                        ui.get_battery_time_to_full_mins(),
                        ui.get_power_profile().as_str(),
                    ),
                )
                // Desktops have profiles too, so this stands whether or not there is a battery.
                // `null` when there is no power-profiles-daemon.
                .with(
                    "power_profile",
                    crate::power_status::profile_for_describe(
                        ui.get_power_profile().as_str(),
                        ui.get_power_performance_offered(),
                    ),
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
                        // What the person is typing, before they have sent it: theirs alone.
                        "text": if agent_reading { String::new() } else { ui.get_lens_input_text().to_string() },
                        "chat": ui.get_lens_chat_mode(),
                    }),
                )
                // The launcher, for the same reason. `open_app name=launchpad` opened it under
                // whatever window was in front, and nothing a caller could read said it was
                // open at all: `failed_launches` was empty because nothing had failed.
                .with("launcher", crate::wire::launcher::for_describe(&ui))
                // The bar's three panels, which open over any screen: whether each is on the
                // screen, and the action that puts it away. See `control_overlays`.
                .with("today", crate::control_overlays::panel_for_describe(ui.get_today_open(), screen, "close_today"))
                // The bar's Minds chip: what it says and where a click goes, read from the same two
                // numbers it draws, so "does anything need me" does not need a screenshot.
                .with("bar_minds", crate::control_overlays::bar_minds_for_describe(
                    ui.global::<crate::AgentsState>().get_request_minds(),
                    ui.global::<crate::AgentsState>().get_needs_count(),
                ))
                .with("quick_settings", crate::control_overlays::panel_for_describe(ui.get_quick_settings_open(), screen, "close_quick_settings"))
                .with("power_menu", crate::control_overlays::panel_for_describe(ui.get_power_menu_open(), screen, "close_power_menu"))
                // What the power popover offers right now: Hibernate only where logind does, and
                // how many minds a restart would stop.
                .with("power", crate::control_power::for_describe(&ui))
                .with("clipboard_panel", crate::control_overlays::panel_for_describe(ui.get_clip_panel_open(), screen, "close_clipboard"))
                .with("cheat_sheet", crate::control_overlays::panel_for_describe(ui.get_cheat_sheet_open(), screen, "close_cheat_sheet"))
                // The window overview (Super+Tab): whether it is up and what it has selected.
                .with("window_switcher", crate::control_switcher::for_describe(screen))
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
                .with("incognito", ui.get_settings_incognito_mode() || crate::private_mode::is_on())
                // Private mode: only the person ever reads this as true. While it is on, an agent
                // is refused before describe runs (`yantrik_ipc_transport::privacy`).
                .with("private", crate::private_mode::is_on())
                // The theme in use and the ones to choose from (`set_theme`).
                .with("theme", crate::wire::theme::for_describe(ui.get_settings_theme().as_str()))
                .with("settings", serde_json::json!({"category":ui.get_settings_category(),"query":ui.get_settings_query().to_string(),"dark":ui.get_settings_dark_mode(),"accent":ui.get_settings_accent_color().to_string(),"wallpaper":ui.get_wallpaper_path().to_string(),"save_error":ui.get_settings_save_error(),"save_status":ui.get_settings_save_status().to_string(),"auto_lock_secs":ui.get_settings_auto_lock_secs(),"auto_lock_available":ui.get_settings_auto_lock_available()}))
        }
    };

    let weak = ui.as_weak();
    let ui_for = move || weak.upgrade().ok_or_else(|| "the shell is gone".to_string());

    let open_ui = ui_for.clone();
    let report_ui = ui_for.clone();
    let screen_ui = ui_for.clone();
    let focus_ui = ui_for.clone();
    let dnd_ui = ui_for.clone();
    let profile_ui = ui_for.clone();
    let ask_ui = ui_for.clone();
    let lens_ui = ui_for.clone();
    let pin_ui = ui_for.clone();
    let pin_catalogue = ctx.installed_apps.clone();
    let read_ui = ui_for.clone();
    let panel_ui = ui_for.clone();
    let desk_ui = ui_for.clone();
    let rule_ui = ui_for.clone();
    let lock_ui = ui_for;

    let surface = ControlSurface::new("shell")
        .describe(describe)
        // The locked-desktop rule, installed on the dispatch every action on this surface
        // crosses rather than in the actions themselves (#203): the login screen used to be a
        // picture over a signed-in session, and `yos act shell open_lens` walked the desktop
        // straight past it. The rule reads the screen the shell is showing — the state IS the
        // screen, so there is nothing to fall out of sync — and refuses every action the
        // allow-list in `allowed_while_locked` does not name (today only `memory_validate`). An
        // action added to this surface tomorrow is held to it without its author doing
        // anything; getting out from under it means editing the allow-list, past a reader.
        .state_rule(move |action| {
            // Fail closed: a rule that cannot read the screen cannot know the desktop is open.
            let ui = rule_ui()?;
            match locked_refusal(ui.get_current_screen(), action) {
                Some(refusal) => Err(refusal),
                None => Ok(()),
            }
        })
        .action(
            Action::new(
                "report_problem",
                "Send one of this machine's problem records - a crash or failure this desktop wrote \
                 down, listed under `problems` in describe - to the project's report intake, with a \
                 note. Graded sensitive because the record leaves the machine. It carries no name, \
                 hostname or address; the bytes sent are exactly the record as the file holds it, \
                 which is what the Report a problem screen shows. The answer says where it landed. \
                 Once sent it cannot be undone.",
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
            // failed spawn is logged, not returned). The branches that only switch one of the
            // desktop's own screens settle on return, and now say which happened — the caller was
            // told `launching` either way, and waited for a window that was never coming (#45).
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
                    let mut answer = open_launcher(&ui)?;
                    answer["launching"] = name.into();
                    return Ok(answer);
                }
                // An app's window would come up over a waiting card (card_watch).
                crate::card_watch::hold_windows("open_app")?;
                // The id the window will be known by, and a past failure of it cleared, so what
                // the wait below reads back is this launch's and not an earlier one's.
                let window_id = crate::wire::dock::launcher_id_in(&name, &catalogue);
                crate::running::clear_launch_failure(&window_id);
                // The launcher's own path: it resolves the binary, enforces one window per app,
                // and focuses the running one instead of starting a second.
                // Asked here too, where the caller is in scope, so the answer can say when the
                // dock will not start a second copy for a mind (`mind_view::route`, B2).
                let already_open_for_mind = !crate::mind_view::route_now(&window_id).spawn;
                ui.invoke_launch_app(name.clone().into());
                // Which of the two that did. A program opens a window; a name that is part of the
                // desktop switches one of its screens, and no window exists or ever will. The
                // listing has always said which a name is (`opens: "app"` against `opens: "a
                // screen of the desktop itself"`); the answer said `launching` for both, so a
                // caller that opened `files` waited for a window and saw nothing arrive (#45).
                let mut answer = match switches_the_shell(&name) {
                    Some((screen, section)) => {
                        // Asked to come forward like any other window, because a screen switched
                        // underneath an app window has not been shown to anyone (#71).
                        shell_screen_answer(screen, section, crate::windows::raise_shell())
                    }
                    None => serde_json::json!({ "launching": name }),
                };
                // And the name to describe it by once it is up, which is not always the name it
                // was opened by (`sysmonitor` opens what answers as `system-monitor`).
                if let Some(surface) = crate::wire::dock::surface_for(&name, &catalogue) {
                    answer["describe_as"] = surface.into();
                }
                if already_open_for_mind {
                    answer["note"] = "already open on the person's desktop: not launched again, and not \
                                      raised over their work. Drive it through its surface."
                        .into();
                }
                // A program is not a window until one is seen. The dock spawns it on a worker and
                // `launching` told a mind "done" for a terminal that never drew anywhere (2 Oct
                // 2026), so the answer waits, off the UI thread, for the window to be listed —
                // in Mind View's compositor for a mind's launch, on the desktop for a person's —
                // and says where it is, or that it is not.
                if answer.get("launching").is_some() {
                    let wait = move || {
                        let mut probe = crate::mind_landing::probe_for(window_id);
                        let seen = crate::mind_landing::wait_for_window(
                            &mut probe,
                            crate::mind_landing::BUDGET,
                            crate::mind_landing::STEP,
                        );
                        crate::mind_landing::answer(answer, &name, seen)
                    };
                    return yantrik_app_runtime::control::answer_later(wait)
                        .map(|()| serde_json::json!({ "answering": "off the UI thread" }))
                        .or_else(|wait| wait());
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
            // verb rather than a way to fake the click. `sensitive` because the pin list is
            // written to the shell's settings: the decision outlives the turn that made it and
            // is still standing after a restart, which is what a stored setting is.
            Action::new("pin_app", "Pin an app to START, or unpin it")
                .risk("sensitive")
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
            // asked, not when it has been answered. The answer arrives in the person's `describe`
            // under `conversation`, where they can watch `streaming` go false.
            //
            // The person's, and no agent's (#476): the chat is theirs, and a mind that could put
            // words in it could have the answering mind repeat it back. An agent talks to another
            // agent with `new_agent` / `send_to_agent`. Said plainly, because a harness that
            // reaches this with a token has to know why, not see a question go unanswered.
            Action::new("send_message", "Ask the desktop something, as if typed into the Lens (the person's; an agent talks to another agent with new_agent or send_to_agent)")
                .arg(Param::text("text").describe("What to say"))
                .defers(),
            move |args| {
                if yantrik_app_runtime::control::agent_is_calling() {
                    return Err("send_message puts words in the person's chat, and an agent is calling: refused, nothing was sent. An agent talks to another agent with new_agent or send_to_agent.".into());
                }
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
                // The whole of any message, tool calls included: the person's chat, not a mind's
                // to page through (the security review of #475).
                persons_only("read_message")?;
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
            // in it, which is what "here, have a look at this" is. The desktop advertises Super+K
            // in two places for exactly this, and the compositor keybind behind the advert needs
            // a verb to call: config/labwc/rc.xml binds Super+K to this action, because that is
            // the only route that works while another app holds the keyboard.
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
            // be the one decision on this desktop reserved for the mouse. `sensitive` because the
            // choice is written to the shell's settings as the preferred mind: it decides who
            // answers from now on, stands after a restart, and belongs in front of the person
            // before it happens rather than after.
            Action::new(
                "reach_of",
                "What an agent token may reach, asked by the token's SHA-256: the reach its role holds \
                 it to, or null for a token with no role. What every door outside the shell asks \
                 before it runs an act that carries an agent token (#189).",
            )
            .risk("safe")
            .arg(Param::text("token_sha256").describe("The SHA-256 of the agent token, as lowercase hex")),
            move |args| {
                let digest = args["token_sha256"].as_str().unwrap_or_default();
                if digest.len() != 64 || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err("`token_sha256` is the token's SHA-256, 64 hex characters".into());
                }
                let known = crate::wire::harness::host().is_some_and(|h| {
                    h.knows_token_digest(&digest.to_ascii_lowercase(), yantrik_ipc_transport::reach::token_digest)
                });
                Ok(serde_json::json!({ "reach": crate::agents::reaches::lookup_digest(digest), "known": known }))
            },
        )
        .action(
            // The memory server's question (#447): a mind has shown it a credential, and before
            // anything is recalled or kept the server asks the desktop, the identity authority,
            // whose it is and what it may do. Answered to the person's mind account alone (the
            // account that serves the memory): asked by anyone else it would be a way to test
            // whether a stolen credential is still good.
            Action::new(
                "memory_validate",
                "Asked by the person's memory server: which mind a memory credential belongs to and \
                 what it may do, or null for one the desktop does not know",
            )
            .risk("safe")
            // The answer is about a credential, not about the desktop, and it is asked before
            // every memory call: the shell's whole state beside it was 35 KB the server discards.
            .stateless()
            .arg(Param::text("memory_sha256").describe(
                "The SHA-256 of what the mind presented, as lowercase hex: never the thing itself",
            )),
            // Asked with no agent token: the server is not an agent acting on the desktop, and
            // the surface's standing rule names this action as the one that needs none. The
            // check on the asker's account inside is what holds it instead.
            move |args| {
                crate::memory_grants::validate(
                    args,
                    crate::wire::harness::host(),
                    yantrik_ipc_transport::mind_door::is_mind,
                    crate::memory_grants::load,
                )
            },
        )
        .action(
            Action::new("use_harness", "Choose which mind answers when the shell is asked something")
                .risk("sensitive")
                .arg(Param::text("id").describe("Harness id, as `describe shell` lists under `minds`")),
            move |args| {
                let id = args["id"].as_str().unwrap_or_default().trim().to_string();
                if id.is_empty() {
                    return Err("`id` is empty".into());
                }
                let host = crate::wire::harness::host()
                    .ok_or_else(|| "the harness host is not running".to_string())?;
                // The same refusal the Settings page's *Use this* gets: a mind whose process
                // is gone has already left `harnesses`, and choosing it anyway is answered
                // with what is attached rather than a quiet success (#67).
                crate::wire::harness::choose(host, &id)?;
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
            // Sensitive: it writes a provider's key into a harness's own settings file and restarts
            // the harness, so the person answers the approval card first. The answer is the same
            // text the Settings card shows — which files, which provider and model, never the key.
            Action::new(
                "assign_provider",
                "Give a harness one of the person's saved providers, written into the harness's own settings",
            )
            .risk("sensitive")
            // The card names where the key goes — provider, address, model, file — which the
            // bare arguments (an id like `custom-3fa1b2`) do not. A per-call sentence also means
            // "allow for this session" is never offered: each key moved is its own question.
            .explain(|args| {
                crate::wire::harness_provider::explain_assign(
                    args["harness"].as_str().unwrap_or_default().trim(),
                    args["provider"].as_str().unwrap_or_default().trim(),
                )
            })
            .arg(Param::text("harness").describe("Harness id, as `describe shell` lists under `harnesses`, with can_assign_provider"))
            .arg(Param::text("provider").describe("A saved provider's id or name, as Settings → AI lists it")),
            move |args| {
                let harness = args["harness"].as_str().unwrap_or_default().trim().to_string();
                let provider = args["provider"].as_str().unwrap_or_default().trim().to_string();
                if harness.is_empty() || provider.is_empty() {
                    return Err("`harness` and `provider` are both needed".into());
                }
                let card = crate::wire::harness_provider::assign(&harness, &provider)?;
                Ok(serde_json::json!({ "assigned": harness, "provider": provider, "did": card }))
            },
        )
        .action(
            // Sensitive for the same reason: it rewrites a harness's settings file (back to the
            // person's own) and restarts it.
            Action::new(
                "revert_provider",
                "Put back a harness's own settings, undoing assign_provider",
            )
            .risk("sensitive")
            .explain(|args| crate::wire::harness_provider::explain_revert(args["harness"].as_str().unwrap_or_default().trim()))
            .arg(Param::text("harness").describe("Harness id with can_revert_provider")),
            move |args| {
                let harness = args["harness"].as_str().unwrap_or_default().trim().to_string();
                crate::wire::harness_provider::revert(&harness)?;
                Ok(serde_json::json!({ "reverted": harness }))
            },
        )
        .action(
            // Standard: it opens a window and nothing else. The harness's own setup runs in a
            // terminal the person types into, and nothing changes on this machine unless they
            // answer it — the agent cannot type there, and nothing here reads the window.
            Action::new(
                "configure_harness",
                "Open a harness's own setup (choosing its model, signing in) in a terminal for the person",
            )
            .risk("standard")
            .arg(
                Param::text("id")
                    .describe("Harness id, as `describe shell` lists under `harnesses`, with `configure` set"),
            ),
            move |args| {
                let id = args["id"].as_str().unwrap_or_default().trim().to_string();
                if id.is_empty() {
                    return Err("`id` is empty".into());
                }
                // The terminal comes up over a waiting card (card_watch). The person's own button
                // on the Harnesses page calls `configure` directly and is not held.
                crate::card_watch::hold_windows("configure_harness")?;
                let command = crate::wire::harness::configure(&id)?;
                Ok(serde_json::json!({
                    "opened": id,
                    "command": command,
                    "watch": "the person answers it in the terminal; describe the shell afterwards and read harnesses[] for what the row says now",
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
            // The taskbar's corner button and Super+D, for a caller that is not at the screen: the
            // same press, with the same memory of what it put away (#241).
            Action::new(
                "show_desktop",
                "Put every app window away and show the desktop; asked again with no window opened or closed in between, bring the same windows back",
            )
            .risk("safe"),
            move |_args| {
                let ui = desk_ui()?;
                // Putting everything away uncovers a card, so it is never held. The second press
                // brings every window back over the shell, and with it over a waiting card
                // (final review of the card fix), so that press waits for the card. Super+D
                // comes through here too, by `yos`, and waits with it.
                crate::wire::show_desktop::press(&ui, || crate::card_watch::hold_windows("show_desktop"))
            },
        )
        .action(
            Action::new("show_screen", "Switch the shell to one of its screens")
                .arg(
                    Param::text("screen")
                        .describe("desktop, files, settings, notifications, memory, system, permissions, bond, personality, about, packages, devices, problems, agents, recipes — or launchpad, the launcher, which opens over the desktop"),
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
                    let mut answer = open_launcher(&ui)?;
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
                        if let Some((_, went)) = BECAME_WINDOWS.iter().find(|(n, _)| *n == want) {
                            return format!("`{want}` is not a screen any more; {went}");
                        }
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
                crate::card_watch::hold_windows("show_app")?;
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
                // Bringing the desktop itself forward never covers a card; anything else would.
                if title != crate::windows::SHELL_WINDOW_TITLE {
                    crate::card_watch::hold_windows("focus_window")?;
                }
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
                let title = match crate::windows::window_to_close(want, &open) {
                    Ok(title) => title,
                    // Not on the person's desktop, but an app a mind opened is drawn in Mind View:
                    // close it there, as its × would.
                    Err(why) => match crate::mind_view::app_named(want) {
                        Some(app) => {
                            let name = crate::mind_view::close_app(&app)?;
                            return Ok(serde_json::json!({
                                "closing": name,
                                "where": "Mind View",
                                "note": "the app was drawn in Mind View, where the apps a mind opens go,                                          and was asked to close there as pressing × does; one with                                          unsaved work may put up its own dialog and stay. Read                                          `mind_view` in `describe shell` to see whether it went.",
                            }));
                        }
                        None => return Err(why),
                    },
                };
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
                // The shell minimised takes every card, the taskbar's count of them and the way
                // back with it, and focusing it does not bring a minimised window back (second
                // review of the card fix). The person minimises nothing of the desktop by a click
                // either: it has no minimise button.
                if title == crate::windows::SHELL_WINDOW_TITLE {
                    return Err(format!(
                        "`{title}` is the desktop itself, with the status bar, the taskbar and any \
                         card waiting for the person; it is not minimised"
                    ));
                }
                crate::card_watch::hold_windows("minimise_window")?;
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
            // The third thing a person does with a window from its bar, and the verb this
            // surface was missing while the taskbar's own menu (#232) needed it. The menu's
            // row and this action are one path: both resolve the title the same way and both
            // end in `windows::maximise`, so a mind and a pointer get the same behaviour and
            // the same name for it.
            //
            // There is no restore half and no toggle, because wlrctl 0.2.2 has no unmaximize:
            // a maximized window comes back by its app's own button or the compositor's
            // Super+Up (config/labwc/rc.xml). Deferred because the compositor decides, like
            // every other window verb here.
            Action::new(
                "maximise_window",
                "Maximise an open window, as pressing its maximise button does. There is no \
                 unmaximize here — the app's own button or Super+Up restores it",
            )
            .defers()
            .arg(Param::text("title").describe("Window title, or part of one")),
            move |args| {
                let want = args["title"].as_str().unwrap_or_default();
                let open = crate::windows::addressable_titles();
                let title = crate::windows::window_named(want, &open)?;
                crate::card_watch::hold_windows("maximise_window")?;
                crate::windows::maximise(&title)?;
                Ok(serde_json::json!({
                    "maximised": title,
                    // Said because a caller looking for the other half of a toggle would
                    // otherwise assume one exists and search the surface for it.
                    "note": "the window fills the screen until its app's own button or \
                             Super+Up restores it; wlrctl has no unmaximize to call.",
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
            //
            // `sensitive` because the setting stands after a restart — the description says so —
            // and a Do Not Disturb left on by a caller swallows every notification that follows,
            // quietly, until somebody notices. A lasting change to how the machine behaves is
            // for the person to see first.
            Action::new("set_do_not_disturb", "Hold or release notifications. Stays after a restart")
                .risk("sensitive")
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
            // The battery popover's profile choice, for a caller. `standard`: it changes how
            // the machine trades speed for battery, which the person can undo with the same
            // choice and which the daemon itself resets (it drops to power-saver on its own
            // when the battery runs low). Nothing is written to a file here; the daemon holds
            // it. The answer is read back from the daemon, so a profile it did not take is
            // not reported as taken, and one it does not offer is refused before it is asked.
            Action::new(
                "set_power_profile",
                "Set the power profile: power-saver, balanced or performance. Performance is only on hardware that has it; `describe shell` lists what is offered under `power_profile`. Answers with the profile in effect afterwards.",
            )
            .risk("standard")
            .arg(Param::text("profile").describe("power-saver, balanced or performance")),
            move |args| {
                let weak = profile_ui()?.as_weak();
                let profile = args["profile"]
                    .as_str()
                    .ok_or("`profile` must be power-saver, balanced or performance")?
                    .to_string();
                // The bus calls are off the UI thread: a power-profiles-daemon that hangs, or a
                // polkit prompt nobody answers, must not freeze the shell. The answer is what
                // the daemon reports afterwards, finished on the socket's side; each call has a
                // 2 s timeout, so a stuck daemon is an error here and not a wait.
                let work = move || {
                    let now = yantrik_os::power_profile::set(&profile)?;
                    crate::power_status::apply_profile_later(weak, Some(now.clone()));
                    tracing::info!(profile = %now.active, "Power profile set");
                    Ok(serde_json::json!({
                        "power_profile": now.active,
                        "offered": now.offered,
                    }))
                };
                yantrik_app_runtime::control::answer_later(work)
                    .map(|()| serde_json::json!({ "answering": "off the UI thread" }))
                    .or_else(|work| work())
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
            // own action rather than hiding inside `show_screen`.
            //
            // `safe`, because locking only takes access away — a caller that may do anything at
            // all may do this, and asking adds a way for the lock to fail: Super+L, which labwc
            // binds to `yos act shell lock`, raised an approval card and left the desktop open
            // for whoever walked past, and the card then moved from the Lens to the popup when
            // the screen changed, so the click aimed at where it had been missed (#215). What
            // unlocking costs is the description's job to say, not a grant's to decide.
            Action::new(
                "lock",
                "Lock the screen now; unlocking needs the person's password or PIN",
            )
            .risk("safe"),
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
    // Reading a display's text from its pixels, for windows that publish nothing else (#257).
    let surface = crate::control_screen::actions(surface);
    // Asking the person. Three actions, all `safe`, none of which decides anything — the
    // decision is a button in the Lens. See `control_approvals` for why that split is the
    // whole point.
    let surface = crate::control_approvals::actions(surface, ui);
    // The bar's panels: open_quick_settings, open_power_menu, open_clipboard, open_cheat_sheet and their closes.
    let surface = crate::control_overlays::actions(surface, ui);
    // The media keys' actions: set_volume, set_mute, set_brightness, set_mic_mute, mute_mic, show_caps_lock.
    let surface = crate::control_levels_actions::actions(surface, ui);
    // The desktop's theme: set_theme. See `control_theme`.
    let surface = crate::control_theme::actions(surface, ui);
    // The network mark's popover as data: set_wifi, disconnect_network, connect_wifi. See
    // `control_network`, which also says why no action there takes a password.
    let surface = crate::control_network::actions(surface, ui);
    // Suspend, hibernate, log out, restart and shut down: the power popover's acts, graded.
    let surface = crate::control_power::actions(surface, ui);
    // The window overview (Super+Tab): open_switcher, switcher_move, switcher_commit, switcher_cancel.
    let surface = crate::control_switcher::actions(surface, ui);
    // An agent's commands, each in a terminal of its own in its pane — agent_run, agent_job,
    // agent_input, agent_kill. The agent comes from its token, never an argument. See
    // `control_agent_terminal` and design/agents-workspace-2026-09-23.md, decision 3.
    let surface = crate::control_agent_terminal::actions(surface);
    // A recipe's question answered, and a recipe paused, resumed or cancelled — answer_recipe,
    // pause_recipe, resume_recipe, cancel_recipe. See `control_recipes`.
    let surface = crate::control_recipes::actions(surface, ctx.bridge.handle());
    // The decision model in use: the browser service's commitment check, and agents' own quick
    // questions of a model in the house. See `control_decide`.
    let surface = crate::control_decide::actions(surface, ctx.bridge.handle(), services.clone());
    // ── Agents glue: new_agent / send_to_agent / stop_agent / read_agent / show_agent / hand_off —
    // how a mind hands work to another agent, or to a role from the catalog. The caller's agent
    // comes from its token. See `control_agents` and design/agents-workspace-2026-09-23.md,
    // decision 1, and design/desk-and-mind-2026-09-23.md, section 5.
    // The chat in one read, for a client that is not the Lens: the Yantrik terminal. See
    // `control_chat`.
    let surface = crate::control_chat::actions(surface, ui);
    // The workroom's own navigation: show_workroom. See `control_workroom`.
    let surface = crate::control_workroom::actions(surface, ui);
    crate::control_agents::actions(surface, ui).serve();
}

#[cfg(test)]
mod screen_entry_tests {
    use super::{screen_entry, SCREENS};

    /// Every screen a person reads as a window gets a taskbar entry named as its title bar names
    /// it; the desktop, and the screens nobody switches away from, get none.
    #[test]
    fn a_screen_that_looks_like_a_window_is_on_the_taskbar() {
        assert_eq!(screen_entry(8), Some(("files", "Files".to_string())));
        assert_eq!(screen_entry(7), Some(("settings", "Settings".to_string())));
        assert_eq!(screen_entry(34), Some(("agents", "Agents".to_string())));
        for none in [1, 0, 2, 3, 32, 999] {
            assert_eq!(screen_entry(none), None, "screen {none}");
        }
        for (name, id) in SCREENS.iter().filter(|(n, _)| *n != "desktop") {
            let (entry, title) = screen_entry(*id).expect(name);
            assert_eq!(entry, *name);
            assert!(title.chars().next().is_some_and(|c| c.is_ascii_uppercase()), "{title}");
        }
    }
}

#[cfg(test)]
mod persons_fields_tests {
    use super::without_fields;
    use serde_json::json;

    /// What an agent reading `describe` loses, it loses at any depth, and nothing else goes with it.
    #[test]
    fn the_persons_fields_come_out_wherever_they_sit() {
        let jobs = json!([{ "agent": "pi:main", "running": [{ "job": "j1", "command": "printf 'Lunch with Sam'", "elapsed_secs": 3 }] }]);
        let told = without_fields(jobs, &["command"]);
        assert_eq!(told, json!([{ "agent": "pi:main", "running": [{ "job": "j1", "elapsed_secs": 3 }] }]));
        let audit = json!([{ "app": "email", "action": "send", "args": { "to": "mom" }, "verified": { "line": "yos act email send" }, "outcome": "ok" }]);
        assert_eq!(without_fields(audit, &["args", "verified"]), json!([{ "app": "email", "action": "send", "outcome": "ok" }]));
        assert_eq!(without_fields(json!("text"), &["args"]), json!("text"));
    }
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
mod describe_clock_tests {
    use std::path::Path;

    /// Learning what day it is has to be a read of the shell, not an act on it — and a read
    /// that survives the condensation: `yos` renders describe state sorted and minds keep
    /// the header plus the first ~900 characters, which is why this extends `clock` rather
    /// than adding a key of its own that sorts after `conversation`.
    ///
    /// The describe closure needs a live Slint window, so the wiring is pinned against the
    /// source the way `do_not_disturb_tests` pins its handler; the object itself — its
    /// keys, its offsets, the zone the machine names or does not — is tested for real in
    /// `app_context::clock_tests`.
    #[test]
    fn describe_carries_the_clock_as_an_object_a_mind_can_read_the_day_off() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/control.rs");
        let whole = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let src = whole.split("#[cfg(test)]").next().unwrap_or_default();
        assert!(
            src.contains(".with(\"clock\", crate::app_context::clock_for_describe())"),
            "`describe shell` must carry `clock` as the full date-and-time object: date, \
             weekday, time, UTC offset and zone. Without it the only way for a mind to \
             learn today's date was `shell.agent_run date`, which is graded sensitive and \
             raised an approval card just to answer \"what's on my calendar today\" (#207)."
        );
        assert!(
            !src.contains(".with(\"now\""),
            "the full time belongs under `clock`, not a `now` key of its own: `yos` sorts \
             state keys and minds condense a description to its first ~900 characters, so \
             `now` sorted after `conversation` and was clipped out of what a mind saw."
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

    /// S10: `open_app` and `files_open` wait for the window off the UI thread, and no process
    /// call or wait sits in the handler body. Source scans, like the neighbours.
    #[test]
    fn open_app_and_files_open_wait_for_the_window_off_the_ui_thread() {
        let open_app = action("open_app");
        let later = open_app.find("answer_later(").expect("open_app answers off the UI thread");
        let wait = open_app.find("wait_for_window(").expect("open_app waits for the window");
        assert!(wait < later, "the wait must be inside the closure handed to answer_later:\n{open_app}");
        assert!(open_app.contains("route_now("), "open_app must say when nothing is launched again");
        assert!(!open_app.contains("Command::new"), "no process call in the handler");
        let files = include_str!("control_files.rs");
        let files_open = &files[files.find("\"files_open\"").unwrap()..];
        let files_open = &files_open[..files_open.find("\"files_up\"").unwrap()];
        assert!(files_open.contains("answer_later(") && files_open.contains("wait_for_window("), "{files_open}");
        assert!(files_open.contains(".defers()"));
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
        let held = body.find("card_watch::hold_windows(\"open_launcher\")").expect("`open_launcher` asks hold_windows first");
        assert!(held < body.find("invoke_launch_app(").unwrap(), "hold_windows comes before the grid opens");
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
        for name in ["focus_window", "close_window", "minimise_window", "maximise_window"] {
            let handler = action(name);
            assert!(
                handler.contains("windows::addressable_titles()"),
                "`{name}` must resolve its `title` against the open windows — which include the \
                 shell's own — before it asks the compositor for anything. Handler as \
                 written:\n{handler}"
            );
        }
    }

    /// Maximise is the verb the taskbar's menu was missing (#232), and wlrctl 0.2.2 has no
    /// unmaximize — so the answer has to say the window stays big until something else restores
    /// it. A caller told only `maximised: <title>` would look for the restore half of a toggle
    /// and never find it.
    #[test]
    fn maximise_window_says_there_is_no_other_half() {
        let handler = action("maximise_window");
        assert!(
            handler.contains("windows::maximise(&title)"),
            "`maximise_window` must go through `windows::maximise` — the same function the \
             taskbar menu's row runs, so the menu adds no second path. Handler as \
             written:\n{handler}"
        );
        assert!(
            handler.contains("unmaximize"),
            "`maximise_window` must say wlrctl has no unmaximize, so a caller knows the window \
             stays maximized until its app or Super+Up restores it. Handler as written:\n{handler}"
        );
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

#[cfg(test)]
mod lock_grade_tests {
    use std::path::Path;

    /// The `lock` action's declaration and the head of its handler, as written above the tests.
    ///
    /// The handler needs a live Slint window to run, so the grade and the description are
    /// pinned against the source the way `do_not_disturb_tests` pins its handler. Found from
    /// the handler's one call — unique in this file — back to the `Action::new` above it,
    /// because `"lock"` on its own first appears in `screen_name`, which is not this action.
    fn declaration() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/control.rs");
        let whole = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let src = whole.split("#[cfg(test)]").next().unwrap_or_default();
        let handler = src
            .find("invoke_lock_screen()")
            .expect("the shell still locks the screen through invoke_lock_screen");
        let start = src[..handler]
            .rfind("Action::new(")
            .expect("the lock action is still declared with Action::new");
        src[start..handler].to_string()
    }

    /// Pressing Super+L has to lock the screen, not ask about locking it.
    ///
    /// labwc binds Super+L to `yos act shell lock`, and while this action was graded sensitive
    /// that press put an approval card on the screen and left the desktop unlocked: the person
    /// who walked away left the machine open, and the card moved from the Lens to the popup
    /// when the screen changed, so the click aimed at where it had been missed (#215).
    /// Locking only takes access away, so it is graded `safe` — the one grade every mode runs
    /// unasked — and its description says what getting back in will cost.
    #[test]
    fn locking_the_screen_locks_without_asking() {
        let declaration = declaration();
        assert!(
            declaration.contains(".risk(\"safe\")"),
            "`shell.lock` must be graded safe. Locking only takes access away, and while it \
             was graded sensitive, Super+L raised an approval card and left the screen \
             unlocked until somebody clicked Allow (#215). Declaration as written:\n{declaration}"
        );
        assert!(
            !declaration.contains("sensitive"),
            "`shell.lock` is graded sensitive again: in `ask` mode every lock — Super+L, the \
             power menu, \"lock my screen in 5 minutes\" — becomes a card, and the desktop \
             stays open until somebody answers it. Declaration as written:\n{declaration}"
        );
        assert!(
            declaration.contains("unlocking needs the person's password or PIN"),
            "`shell.lock` carries a description, and it says what unlocking will need: a card, \
             `yos ls` and a mind reading `describe` all show this sentence, and \"(the app \
             publishes no description for this action)\" is what the card in #215 said. \
             Declaration as written:\n{declaration}"
        );
    }
}

#[cfg(test)]
mod summary_running_tests {
    //! The headline said "notes and perception not running" beside a window list that named
    //! Notes and a `describe notes` that answered: the sentence was built from what the
    //! ServiceManager started, and Notes — which keeps its own store and never asks for
    //! notes-service — left that record at "stopped, on demand" for as long as it was open.
    //! Hermes read the headline and refused to describe the app (#34).
    use super::not_running;
    use serde_json::json;

    fn record(id: &str, status: &str) -> serde_json::Value {
        json!({ "id": id, "status": status, "note": "on demand" })
    }

    /// A stopped or failed name that the machine answers is not "not running", whichever
    /// way it is answered: a window for Notes, a socket for calendar, nothing for perception.
    #[test]
    fn a_name_the_machine_answers_is_never_called_not_running() {
        let services = [
            record("notes", "stopped"),
            record("calendar", "stopped"),
            record("perception", "stopped"),
        ];
        let open_apps = ["notes", "terminal"];
        let socket_up = |id: &str| id == "calendar";
        assert_eq!(
            not_running(&services, &open_apps, socket_up),
            vec!["perception"],
            "only the name nothing answers belongs in the sentence"
        );
    }

    /// Trouble still counts when nothing answers: a failed service and a stopped one stay,
    /// and a running record is not the headline's business either way.
    #[test]
    fn a_name_nothing_answers_still_shows() {
        let services = [
            record("weather", "failed"),
            record("email", "stopped"),
            record("network", "running"),
        ];
        let socket_up = |_id: &str| false;
        assert_eq!(not_running(&services, &[], socket_up), vec!["weather", "email"]);
    }

    /// The sentence itself, the way a person — and Hermes — read it.
    #[test]
    fn answered_names_drop_out_of_the_read_aloud_list() {
        let services = [
            record("calendar", "stopped"),
            record("email", "stopped"),
            record("notes", "stopped"),
            record("perception", "stopped"),
        ];
        // The September machine: Calendar, Email and Notes open; nothing serves perception.
        let open_apps = ["calendar", "email", "notes"];
        let socket_up = |_id: &str| false;
        let down = not_running(&services, &open_apps, socket_up);
        assert_eq!(super::list_of(&down), "perception");
    }

    /// The pure rule only holds if `describe` feeds it the live accounts. Pinned against the
    /// source, the way the other describe wirings are: the candidate list must come through
    /// `not_running` with both open windows and both sockets — the service's own name and the
    /// app's `app-` surface, which is the one `yos describe <name>` resolves first.
    #[test]
    fn describe_builds_the_sentence_from_what_answers() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/control.rs");
        let whole = std::fs::read_to_string(&path).expect("control.rs is readable");
        let src = whole.split("#[cfg(test)]").next().unwrap_or_default();
        let from = src
            .find("let down: Vec<&str> = not_running(")
            .expect("`describe` builds its not-running list through `not_running`");
        let end = src[from..].find("});").expect("the call ends") + 3;
        let wiring = &src[from..from + end];
        assert!(
            wiring.contains("open_apps"),
            "open windows are one of the accounts:\n{wiring}"
        );
        assert!(
            wiring.contains("service::is_up(id)"),
            "the service's own socket is the next:\n{wiring}"
        );
        assert!(
            wiring.contains("\"app-{id}\""),
            "so is the app's surface — `yos describe notes` reaches `app-notes`, and the \
             headline must not call it not running while it answers:\n{wiring}"
        );
        // The windows list the ids are taken from is the same merged one the headline's
        // `open` count and the published `windows` array use, so the two can never disagree.
        assert!(
            src.contains("let windows = crate::windows::shell_windows();"),
            "`describe` must consult the launch-registry window list for the open apps"
        );
    }
}

#[cfg(test)]
mod open_app_answer_tests {
    use super::{shell_screen_answer, switches_the_shell, SCREENS, screen_name};
    use std::path::Path;

    /// The `open_app` handler, as written above the tests.
    ///
    /// The handler needs a live Slint window and a compositor to run, so its wiring is pinned
    /// against the source the way `window_action_tests` pins the window verbs. What it is pinned
    /// to call is pure, and tested for real below.
    fn handler() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/control.rs");
        let whole = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let src = whole.split("#[cfg(test)]").next().unwrap_or_default();
        let from = src
            .find("\"open_app\"")
            .expect("the shell still publishes open_app");
        let rest = &src[from..];
        let end = rest.find(".action(").unwrap_or(rest.len());
        rest[..end].to_string()
    }

    /// A name that is part of the desktop is not a program, and the answer used to say it was.
    ///
    /// `open_app name=image-viewer` opened the shell's own Images screen, and `name=text-editor`
    /// its Editor screen, while both answered `launching` — #45's two shipped binaries no route
    /// could reach. #253 retired the screens, so the binaries open by their own names now; what
    /// is left is the other half of the finding, that the names still switching a screen said
    /// `launching` too. The listing has always said which a name is (`opens: "app"` against
    /// `opens: "a screen of the desktop itself"`). This is the answer a caller reads after it has
    /// already asked, which said the same thing either way.
    #[test]
    fn a_name_that_is_part_of_the_desktop_is_answered_as_a_screen_switch() {
        // The shell's own screens, including the spellings a route also answers to.
        assert_eq!(switches_the_shell("files"), Some(("files", None)));
        assert_eq!(switches_the_shell("settings"), Some(("settings", None)));
        assert_eq!(switches_the_shell("device-dashboard"), Some(("devices", None)));
        assert_eq!(switches_the_shell("report a problem"), Some(("problems", None)));
        assert_eq!(switches_the_shell("AGENT"), Some(("agents", None)));
        // Skills is a section of the Settings screen, not a screen of its own.
        assert_eq!(switches_the_shell("skills"), Some(("settings", Some("skills"))));

        // A program opens a window, so `launching` is the true word and the caller is right to
        // go looking for it — including for the two binaries #45 could not reach at all.
        for name in [
            "notes", "email", "image-viewer", "images", "image", "text-editor", "editor",
            "system-monitor", "terminal", "browser", "blender",
        ] {
            assert_eq!(switches_the_shell(name), None, "`{name}` opens a window");
        }
        // The launcher is neither: it answers with its own report, taken before this is reached.
        assert_eq!(switches_the_shell("launchpad"), None);
    }

    /// Both doors to a screen report the screen `describe` reports, for every name the shell routes.
    ///
    /// `open_app name=about` and `show_screen screen=about` move the same desktop screen, and a
    /// caller that read one answer should read the other the same way: which screen it is on, and
    /// whether the shell got in front. The two tables the answer is built from — the route table
    /// and `SCREENS` — are kept in step here, because a route to an id `SCREENS` does not name
    /// would answer `launching` again, quietly: the lookup finds nothing and the dispatch falls
    /// through to the other branch. It drifted once already, with the launcher's two doors (#71).
    #[test]
    fn every_screen_a_route_switches_to_is_one_describe_names() {
        let mut switched = 0;
        for name in crate::wire::dock::builtin_app_ids() {
            let Some((screen, _)) = switches_the_shell(name) else {
                continue;
            };
            let id = SCREENS.iter().find(|(n, _)| *n == screen).map(|(_, id)| *id);
            assert_eq!(
                id.map(screen_name),
                Some(screen),
                "`open_app name={name}` answers `{screen}`, which is not a name describe gives \
                 any screen"
            );
            switched += 1;
        }
        assert!(
            switched >= 15,
            "only {switched} of the desktop's own names switch a screen; the route table has \
             changed shape and this test has stopped checking anything"
        );
    }

    /// The action has to ask the question, not merely have the answer available beside it.
    #[test]
    fn open_app_answers_with_which_of_the_two_a_name_did() {
        let handler = handler();
        assert!(
            handler.contains("switches_the_shell("),
            "`open_app` must ask which of the two a name did before it answers. Answering \
             `launching` for a screen switch is #45: the caller waits for a window no route \
             will ever open. Handler as written:\n{handler}"
        );
        assert!(
            handler.contains("\"launching\""),
            "`open_app` must still answer `launching` for a program — that is the half of the \
             distinction that already worked. Handler as written:\n{handler}"
        );
        assert!(
            handler.contains("raise_shell()"),
            "`open_app` asks the compositor to bring the shell forward when it switches a \
             screen, as `show_screen` and `open_launcher` do: the shell is one fullscreen \
             toplevel to labwc and cannot raise itself, so a screen switched underneath an app \
             window has not been shown to anyone (#71). Handler as written:\n{handler}"
        );
    }

    /// What the switch answers, including the case where nobody can see it.
    #[test]
    fn a_screen_switch_answers_with_the_screen_and_whether_it_is_in_sight() {
        let raised = shell_screen_answer("files", None, Ok(()));
        assert_eq!(raised["showing"], "files");
        assert_eq!(raised["raised"], true);
        assert!(
            raised.get("launching").is_none(),
            "a screen switch answers `launching`, which is what a caller waits on: {raised}"
        );

        let section = shell_screen_answer("settings", Some("skills"), Ok(()));
        assert_eq!(section["showing"], "settings");
        assert_eq!(section["section"], "skills");

        // The screen DID change, so this is not an error — it is just possibly covered, and a
        // caller that has been told which screen it is on is owed the difference. The same words
        // `show_screen` uses, because it is the same fact.
        let covered = shell_screen_answer("about", None, Err("wlrctl: no compositor".into()));
        assert_eq!(covered["raised"], false);
        let note = covered["note"].as_str().unwrap_or_default().to_string();
        assert!(
            note.contains("`about`") && note.contains("wlrctl: no compositor"),
            "the note does not say which screen is up, or why it could not be raised: {note}"
        );
    }
}

#[cfg(test)]
pub(crate) mod locked_state_tests {
    //! The installed machine autologins on tty1, so the login screen is the only thing between
    //! anybody who can reach this process and the desktop — and `yos act shell open_lens` walked
    //! straight past it: `accepted: True, settled: True`, Lens open, conversation on screen (#203).
    //! Locked is a state now, read off the screen the shell is showing, held at the dispatch
    //! every action crosses. These tests are the state's: the pure decision (`locked_refusal`,
    //! which the dispatch calls with the live screen), the allow-list's default, and — for the
    //! wiring that needs a window to run — pins against the source, the way
    //! `do_not_disturb_tests` and `lock_grade_tests` pin theirs.
    use super::{LOCKED_REFUSAL, allowed_while_locked, locked_refusal, locked_screen};
    use std::path::Path;

    /// The lock screen and the login screen, and screens that are neither.
    const LOCKED_SCREENS: &[i32] = &[3, 32];
    const OPEN_SCREENS: &[i32] = &[0, 1, 2, 4, 8, 9, 16, 21, 34];

    /// This file, without its tests.
    fn source() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/control.rs");
        let whole = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        whole.split("#[cfg(test)]").next().unwrap_or_default().to_string()
    }

    /// Every action name the shell's control modules publish, read off their declarations
    /// rather than a list kept beside them — the scan `control_approvals` uses, minus its test
    /// halves so a test's own quoting of `Action::new` cannot feed it.
    pub(crate) fn published_actions() -> Vec<String> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("control") && n.ends_with(".rs"))
                    .unwrap_or(false)
            })
            .collect();
        files.sort();
        let mut out = Vec::new();
        for path in files {
            let whole = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            let src = whole.split("#[cfg(test)]").next().unwrap_or_default();
            for (index, _) in src.match_indices("Action::new(") {
                let rest = &src[index + "Action::new(".len()..];
                // The name is the first string literal after the paren, possibly on the next
                // line. Anything else is not an action declaration and is skipped.
                let Some(open) = rest.find('"') else { continue };
                if rest[..open].chars().any(|c| !c.is_whitespace()) {
                    continue;
                }
                let Some(close) = rest[open + 1..].find('"') else { continue };
                out.push(rest[open + 1..open + 1 + close].to_string());
            }
        }
        out
    }

    /// Which screens the desktop waits behind — and the refusal's exact words, because a
    /// caller branches on the `LOCKED:` prefix the way it branches on `GRANT:` or `CEILING:`,
    /// and the release-check greps for it.
    #[test]
    fn locked_is_the_lock_screen_and_the_login_screen() {
        for screen in LOCKED_SCREENS {
            assert!(locked_screen(*screen), "screen {screen} is a locked screen");
        }
        for screen in OPEN_SCREENS {
            assert!(!locked_screen(*screen), "screen {screen} is not a locked screen");
        }
        assert_eq!(
            LOCKED_REFUSAL, "LOCKED: the desktop is waiting for the person to sign in",
            "the refusal is one exact sentence so every door says the same thing"
        );
    }

    /// While the desktop waits for the person, every action it publishes is refused — starting
    /// with the two the bug report ran from a shell prompt.
    #[test]
    fn every_action_the_shell_publishes_is_refused_while_locked() {
        for screen in LOCKED_SCREENS {
            for action in ["open_lens", "show_screen"] {
                assert_eq!(
                    locked_refusal(*screen, action).as_deref(),
                    Some(LOCKED_REFUSAL),
                    "`yos act shell {action}` walked the desktop past screen {screen} (#203)"
                );
            }
        }

        let actions = published_actions();
        assert!(
            actions.len() > 30,
            "only {} actions were found — the scan is not reading the control modules any more, \
             which would make this test pass by seeing nothing. Found: {actions:?}",
            actions.len()
        );
        for screen in LOCKED_SCREENS {
            for action in actions.iter().filter(|a| !super::ALLOWED_WHILE_LOCKED.contains(&a.as_str())) {
                assert_eq!(
                    locked_refusal(*screen, action).as_deref(),
                    Some(LOCKED_REFUSAL),
                    "`{action}` is not on the allow-list, so screen {screen} must refuse it"
                );
            }
        }
        // What the allow-list names, and nothing else: an addition is a decision for a reader.
        assert_eq!(super::ALLOWED_WHILE_LOCKED, &["memory_validate"]);
        for screen in LOCKED_SCREENS {
            assert_eq!(locked_refusal(*screen, "memory_validate"), None, "the memory server is answered");
        }
    }

    /// An action added tomorrow is refused by default: the rule is the surface's, not the
    /// actions', so its author does not have to remember anything, and getting out from under
    /// it means editing the allow-list, past a reader.
    #[test]
    fn an_action_added_tomorrow_is_refused_by_default() {
        for screen in LOCKED_SCREENS {
            assert_eq!(
                locked_refusal(*screen, "an_action_added_tomorrow").as_deref(),
                Some(LOCKED_REFUSAL),
                "a name the allow-list has never seen must be refused on screen {screen}"
            );
        }
        assert!(
            !allowed_while_locked("lock"),
            "`lock` while locked is refused too: at the login screen it would trade the \
             password gate for the weaker PIN one"
        );
    }

    /// And the rule holds nothing on an open desktop — it is the lock's rule, not a second
    /// gate every call pays on the way through.
    #[test]
    fn the_rule_refuses_nothing_on_an_open_desktop() {
        for screen in OPEN_SCREENS {
            assert!(
                locked_refusal(*screen, "open_lens").is_none()
                    && locked_refusal(*screen, "an_action_added_tomorrow").is_none(),
                "screen {screen} is open; the state rule must refuse nothing on it"
            );
        }
    }

    /// The rule lives on the dispatch — installed once in `publish`, reading the live screen —
    /// not copied into handlers, where an action added beside them would miss it.
    #[test]
    fn the_rule_is_installed_on_the_dispatch() {
        let src = source();
        assert!(
            src.contains(".state_rule(move |action| {"),
            "`publish` must install the locked-desktop rule on the surface builder, so every \
             action crosses it whether its handler remembers to or not (#203)"
        );
        assert!(
            src.contains("locked_refusal(ui.get_current_screen(), action)"),
            "the installed rule must decide with `locked_refusal` against the screen the shell \
             is showing: the state IS the screen, so there is nothing to fall out of sync"
        );
    }

    /// A locked desktop still answers `describe` — a caller must be able to tell "waiting for
    /// the person" from "hung" — but says `locked: true` and nothing personal: no conversation,
    /// no window titles, no notifications.
    #[test]
    fn describe_says_locked_and_nothing_personal_while_locked() {
        let src = source();
        let locked_at = src
            .find("if locked_screen(screen) {")
            .expect("the describe closure still cuts itself short while the desktop is locked");
        let personal_at = src
            .find(".with(\"conversation\"")
            .expect("describe still reports the conversation on an open desktop");
        assert!(
            locked_at < personal_at,
            "the locked early-return must sit before the conversation is read: reading is free, \
             so a locked describe is enforced by having nothing to say, and that only works if \
             it returns before anything personal is gathered"
        );
        let branch = &src[locked_at..personal_at];
        assert!(
            branch.contains("return View::new(") && branch.contains(".with(\"locked\", true)"),
            "the locked describe must return early with `locked: true`. Branch as written:\n{branch}"
        );
    }

    /// Nothing persists the state, so a restart during the login screen comes back locked —
    /// as long as the rule exists before the shell is put on that screen. `main` applies
    /// `YANTRIK_START_SCREEN` after `publish`, and this pins the order.
    #[test]
    fn a_restart_during_the_login_screen_comes_back_locked() {
        assert!(locked_screen(32), "the login screen is a locked screen; there is no stored `unlocked` to read");
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs");
        let main = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let publish = main.find("control::publish(").expect("main still publishes the control surface");
        let start = main.find("YANTRIK_START_SCREEN").expect("main still honours the start screen");
        assert!(
            publish < start,
            "the surface — and with it the locked rule — must be installed before the start \
             screen is applied, or the shell sits on the login screen with no rule holding it"
        );
    }

    /// The login screen has exactly one way off, and it sits under the verified-password
    /// branch: a PAM-checked login here, or the lock screen's own unlock. No timer, no
    /// fallback, no other call.
    #[test]
    fn only_a_verified_login_leaves_the_login_screen() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/wire/login.rs");
        let login = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        assert_eq!(
            login.matches("ui.set_current_screen(1);").count(),
            1,
            "login.rs must navigate to the desktop in exactly one place; a second one is a \
             second way past the password"
        );
        let at = login.find("ui.set_current_screen(1);").unwrap();
        assert!(
            login[..at].contains("if authenticated {"),
            "and that one place must sit under `if authenticated`, the branch `verify_password` \
             opened. Login.rs before the navigation:\n{}",
            &login[..at]
        );
    }

    /// The socket's dispatch is the main door, but not the only one: toasts that draw over every screen, the palette, the morning brief's
    /// boot timer and Ctrl+K in the markup all move the shell on their own, and #203 names
    /// them. Each holds to the state through the same pure predicate; these handlers need a
    /// live window to run, so the placement is pinned against the source.
    #[test]
    fn the_doors_that_are_not_the_socket_are_held_to_the_same_state() {
        fn wire(rel: &str) -> String {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
            std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        }
        let needle = "crate::control::locked_screen(ui.get_current_screen())";
        // (file, how many of its own doors the predicate must hold) — notifications has two:
        // the toast body and a toast button's action, which reaches an app's own surface.
        for (file, doors) in [
            ("src/wire/command_palette.rs", 1),
            ("src/wire/notifications.rs", 2),
            ("src/wire/morning_brief.rs", 1),
        ] {
            let src = wire(file);
            assert_eq!(
                src.matches(needle).count(),
                doors,
                "{file} must hold its {doors} door(s) to `locked_screen` while the desktop \
                 waits for the person (#203); it does so {} time(s)",
                src.matches(needle).count()
            );
        }

        // Ctrl+K in the markup: the arm that walks the shell to the desktop with the Lens open
        // must give up on the lock and login screens before it navigates anywhere.
        let app = wire("../yantrik-ui-slint/ui/app.slint");
        let at = app
            .find("event.modifiers.control && (event.text == \"k\" || event.text == \"K\")")
            .expect("app.slint still captures Ctrl+K");
        let arm = &app[at..];
        let nav = arm.find("root.navigate(1);").expect("the Ctrl+K arm still goes to the desktop");
        assert!(
            arm[..nav].contains("root.current-screen == 3 || root.current-screen == 32"),
            "Ctrl+K must not walk the shell off lock (3) or login (32) — it was one of the ways \
             past the login screen (#203). Arm as written:\n{}",
            &arm[..nav]
        );
    }
}

#[cfg(test)]
mod lasting_settings_grade_tests {
    //! #48, on the shell's own surface: the walk of every published action found three that
    //! write settings which outlive the turn, graded as if they moved a window. `pin_app`
    //! writes the START pin list, `use_harness` writes the preferred mind — deciding who
    //! answers after a restart — and `set_do_not_disturb` writes `dnd_mode`, which swallows
    //! every notification that follows until somebody notices. All three write through
    //! `crate::wire::settings` into the shell's settings file; the same walk left the show,
    //! read and window verbs where they were, and left `lock` at `safe` (#215).
    //!
    //! The handlers need a live shell to run, so the grades are pinned against the source
    //! the way `lock_grade_tests` pins `lock`.
    use std::path::Path;

    /// One action's declaration and handler, from its quoted name to where the next
    /// `.action(` begins, as written above the tests.
    fn declaration(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/control.rs");
        let whole = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let src = whole.split("#[cfg(test)]").next().unwrap_or_default();
        let quoted = format!("\"{name}\"");
        let from = src
            .find(&quoted)
            .unwrap_or_else(|| panic!("the shell no longer publishes {name}"));
        let rest = &src[from..];
        let end = rest.find(".action(").unwrap_or(rest.len());
        rest[..end].to_string()
    }

    /// A choice written into the settings file is a choice the person sees first.
    #[test]
    fn what_outlives_the_turn_asks_first() {
        for (name, why) in [
            ("pin_app", "writes the START pin list into the shell's settings"),
            (
                "use_harness",
                "writes the preferred mind into the shell's settings, deciding which mind \
                 answers after a restart",
            ),
            (
                "set_do_not_disturb",
                "writes dnd_mode into the shell's settings, holding every notification that \
                 follows until somebody notices",
            ),
        ] {
            let declaration = declaration(name);
            assert!(
                declaration.contains(".risk(\"sensitive\")"),
                "`shell.{name}` must be graded sensitive: it {why}, and an effect that \
                 outlives the turn is at least `sensitive` (#48) — graded `standard`, or \
                 undeclared and taking the default, it runs unasked in the default mode. \
                 Declaration as written:\n{declaration}"
            );
            assert!(
                !declaration.contains(".risk(\"standard\")"),
                "`shell.{name}` is graded standard again (#48). Declaration as \
                 written:\n{declaration}"
            );
        }
    }

    /// And the walk's keeps are keeps: the shell's show, read and window verbs do not write
    /// settings, and `set_mind_panel` — which remembers how much of one panel is drawn, in
    /// the panel's own file — stays `safe` beside them. If one of these ever does start
    /// writing to the settings file, this is the test that asks what its grade became.
    #[test]
    fn showing_and_reading_stay_below_the_line() {
        for name in ["open_app", "show_screen", "focus_window", "set_mind_panel"] {
            let declaration = declaration(name);
            assert!(
                !declaration.contains(".risk(\"sensitive\")"),
                "`shell.{name}` shows, reads or moves a window and writes no setting: making \
                 it a card in `ask` mode is the everyday flow #48 says must keep working. \
                 Declaration as written:\n{declaration}"
            );
        }
    }
}

#[cfg(test)]
mod power_profile_tests {
    //! `set_power_profile` and the battery's `describe` fields (story 1.4). The handler needs a
    //! live shell and a system bus to run, so these pin its shape against the source, as
    //! `lock_grade_tests` does for `lock`; what the answers contain is tested on the pure
    //! functions in `power_status`.
    use std::path::Path;

    fn source() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/control.rs");
        let whole = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        whole.split("#[cfg(test)]").next().unwrap_or_default().to_string()
    }

    fn declaration() -> String {
        let src = source();
        let from = src.find("\"set_power_profile\"").expect("the shell publishes set_power_profile");
        let rest = &src[from..];
        rest[..rest.find(".action(").unwrap_or(rest.len())].to_string()
    }

    /// It changes how the machine trades speed for battery, nothing is written, and the person
    /// can undo it with the same choice: `standard`, not `safe` and not `sensitive`.
    #[test]
    fn setting_a_profile_is_standard() {
        let d = declaration();
        assert!(d.contains(".risk(\"standard\")"), "set_power_profile is graded standard. As written:\n{d}");
    }

    /// The answer is what the daemon reports afterwards, and a profile it does not offer is
    /// refused: both come from `power_profile::set`, so the handler must go through it and hand
    /// its error on with `?`, not report the profile it was given.
    #[test]
    fn the_answer_is_read_back_and_a_profile_not_offered_is_refused() {
        let d = declaration();
        assert!(d.contains("yantrik_os::power_profile::set(&profile)?"), "the handler calls the daemon and propagates a refusal. As written:\n{d}");
        assert!(d.contains("now.active"), "the answer is the daemon's read-back. As written:\n{d}");
        assert!(!d.contains("\"power_profile\": profile"), "never echo the request as the answer. As written:\n{d}");
    }

    /// A hung daemon froze the shell when the call was made in the handler itself, which runs on
    /// the UI thread. The call belongs inside the closure handed to `answer_later`, which runs
    /// on the socket's side.
    #[test]
    fn the_daemon_is_not_called_on_the_ui_thread() {
        let d = declaration();
        let later = d.find("answer_later(").expect("the answer is finished off the UI thread");
        let work = d.find("let work = move ||").expect("the bus work is a closure");
        let call = d.find("power_profile::set(").expect("the handler sets the profile");
        assert!(work < call && call < later, "`power_profile::set` must be inside the `answer_later` closure, not in the handler body. As written:
{d}");
        assert!(!d[..work].contains("power_profile::set("), "no bus call before the closure. As written:
{d}");
    }

    /// `describe shell` carries the battery's state and times, and the profile at the top level
    /// so a machine without a battery still reports it.
    #[test]
    fn describe_reports_the_battery_object_and_the_profile() {
        let src = source();
        assert!(src.contains("crate::power_status::battery_for_describe("), "describe's `battery` comes from power_status");
        assert!(src.contains("\"power_profile\","), "describe has a top-level `power_profile`, battery or not");
    }
}

#[cfg(test)]
mod status_gate_tests {
    /// The mind's status line is the person's chat in miniature: `describe shell` withholds it
    /// from an agent caller exactly as it withholds the conversation (#587 review).
    #[test]
    fn the_conversation_status_is_withheld_from_an_agent_caller() {
        let src = include_str!("control.rs");
        let at = src.find(".with(\n                    \"conversation_status\"").expect("conversation_status field");
        let field = &src[at..at + 260];
        assert!(field.contains("if agent_reading { String::new() }"), "{field}");
        assert!(field.contains("status_for_describe"), "{field}");
    }
}
