//! Dock wiring — on_launch_app callback, and the one answer to "can this app open here?".

use std::path::{Path, PathBuf};

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::apps::DesktopEntry;
use crate::App;

/// What opening one of the shell's own things does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    /// One of the shell's own screens. Compiled into the shell, so always there.
    Screen(i32),
    /// Settings, opened at one of its sections.
    SettingsSection(i32),
    /// The editor screen, on a blank file.
    Editor,
    /// The Apps launcher. Not a screen of its own: it is an overlay drawn over the desktop
    /// screen, so opening it means going to the desktop and opening the grid there.
    Launchpad,
    /// Whichever web browser this machine has.
    Browser,
    /// Blender, opened with the Yantrik addon so its control surface comes up with it.
    Blender,
}

/// What the shell opens by name that no `.desktop` file can say, and what opening it does.
///
/// The first name in each row is the one it is listed by; the rest are other spellings, after
/// `canonical_id` has folded their punctuation.
///
/// This table IS the dispatch for what is in it. There used to be a match in `wire()` and, beside
/// it, a list of the ids that match accepted, with a comment asking the two to agree. They did
/// not: ten arms were missing from the list, and the launcher's About and Skills tiles had no arm
/// at all, so clicking them logged "Unknown app" while `open_app` answered "launching". One table
/// cannot disagree with itself.
///
/// It used to hold every app this OS ships as well — a row per program with its binary and every
/// spelling of its name — and so did two more tables beside it (the purposes below, the apps'
/// alias table in the runtime). That made our apps the only ones a mind could find while they
/// were closed. Each app now declares itself in its own `.desktop` file (`X-Yantrik-Surface`,
/// `-Purpose`, `-Aliases`, `-Adapter`; see `crate::surfaces`), which is also how an app somebody
/// else wrote does it, and what is left here is what a `.desktop` file cannot express:
///
/// - the shell's own screens, the sections of Settings and the launcher overlay — parts of this
///   process, not programs;
/// - `browser`, which is "whichever browser this machine has", with the flags that let `yos web`
///   drive it;
/// - `blender`, which is a program with a `.desktop` file of its own but is opened only after
///   three checks no Exec line can make: the addon is there, and there is an X display for it
///   (#96). Its surface's purpose still comes from `yantrik-blender.desktop`.
const ROUTES: &[(&[&str], Launch)] = &[
    (&["browser"], Launch::Browser),
    (&["blender"], Launch::Blender),
    (&["files"], Launch::Screen(8)),
    (&["settings"], Launch::Screen(7)),
    (&["bond"], Launch::Screen(4)),
    (&["personality"], Launch::Screen(5)),
    (&["memory"], Launch::Screen(6)),
    (&["notifications"], Launch::Screen(9)),
    (&["system"], Launch::Screen(10)),
    (&["media"], Launch::Screen(13)),
    (&["about"], Launch::Screen(16)),
    // "Install companion skills" is a section of Settings, not a screen of its own.
    (&["skills"], Launch::SettingsSection(7)),
    (&["packages"], Launch::Screen(21)),
    (&["devices", "device_dashboard"], Launch::Screen(27)),
    (&["permissions", "permission_dashboard"], Launch::Screen(28)),
    (&["problems", "report_problem", "report_a_problem"], Launch::Screen(33)),
    (&["agents", "agent"], Launch::Screen(34)),
    (&["recipes", "recipe"], Launch::Screen(35)),
    (&["launchpad"], Launch::Launchpad),
];

/// An app that is in this tree and not in this build.
///
/// `reason` is a sentence a person reads; `returns_when` is what somebody would have to build.
/// Both are quoted verbatim to whoever asked to open the app, so they are written to be read by
/// a person or by a mind that has just been refused and needs to know whether to try something
/// else or to go and write the missing half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shelved {
    /// Every canonical spelling the app arrives under, `canonical_id`-folded.
    pub ids: &'static [&'static str],
    /// What the app is called on screen, from `windows::APP_NAMES`.
    pub name: &'static str,
    /// The binary it would have run. A .desktop entry naming this is dropped from the catalogue.
    pub binary: &'static str,
    /// Why it is not in this build.
    pub reason: &'static str,
    /// The minimum real thing that would put it back.
    pub returns_when: &'static str,
}

/// The apps that are not shipped, and what each is waiting on.
///
/// The rule this table enforces: an app with nothing under its screen is not shipped. Music and
/// ySheets were both complete drawings over nothing — Music has no playback engine, no scanner
/// and no library, so only `YANTRIK_MUSIC_DEMO=1` could ever put a song on screen; ySheets never
/// sets `cell-grid`, `row-count` or `col-count`, so its 50x26 grid has no cells and every guard
/// in cell-click, cell-edit and the formula bar fails. The decision and its reasoning are in
/// design/apps-plan-2026-09-20.md, Wave 3: build it or take it off the shelf. Shipping the
/// drawing is the option that plan rules out.
///
/// Shelved is not deleted. Both crates stay workspace members so they keep compiling and cannot
/// rot in silence, both keep their entry in the lints' debt, and the `.desktop` files stay in the
/// tree — they are simply not packaged, not routed, and not offered to anybody.
///
/// This table is the whole of the shelf. Routes, the catalogue filter, the Lens and the launch
/// refusal all consult it rather than each carrying their own list of names to omit, because a
/// shelf spread over seven files is one a future contributor undoes one line at a time without
/// ever deciding to. Un-shelving an app is deleting one entry here and putting its row back in
/// ROUTES.
const SHELVED: &[Shelved] = &[
    Shelved {
        ids: &["music", "music_player"],
        name: "Music",
        binary: "yantrik-music-player",
        reason: "nothing plays audio yet — there is no playback engine, no scanner and no \
                 library behind the screen",
        returns_when: "mpv is driven over its JSON IPC socket, a folder scan fills a small \
                       library store, and play/pause/next/queue and `open <file>` work",
    },
    Shelved {
        ids: &["spreadsheet", "ysheets"],
        name: "ySheets",
        binary: "yantrik-spreadsheet",
        reason: "there is no cell model behind the grid, so nothing can be typed into it, by \
                 mouse or by mind",
        returns_when: "a cell model, CSV load and save, and arithmetic with references plus \
                       SUM/AVG/MIN/MAX/COUNT are there",
    },
];

/// The shelved app a name refers to, matched the way the dispatch matches a route.
///
/// Every spelling has to reach it, because a refusal that only fires for one of them is not a
/// refusal: a caller reads `music-player` off the binary, `Music Player` off the window title and
/// `music` off the dock, and the launcher used to treat those as three different questions.
pub fn shelved(app: &str) -> Option<&'static Shelved> {
    let id = canonical_id(app);
    // The catalogue names this OS's own apps by their .desktop filename, so `yantrik_music_player`
    // arrives here the same way `music` does.
    let id = id.strip_prefix("yantrik_").unwrap_or(&id);
    SHELVED.iter().find(|s| s.ids.contains(&id))
}

/// The shelved app an Exec line would run, if any.
///
/// This is what keeps the shelf honest on a machine that already has the binary and its .desktop
/// file on disk from an earlier release. Nothing removes those on update, so the catalogue will
/// keep finding the entry; matching on the program it would run means the tile never comes back.
pub fn shelved_exec(exec: &str) -> Option<&'static Shelved> {
    let bin = exec.split_whitespace().next()?;
    let name = bin.rsplit('/').next()?;
    SHELVED.iter().find(|s| s.binary == name)
}

/// One spelling of an app id, from whatever a caller had to hand.
///
/// Callers read app ids from places that punctuate them differently. An app's own control surface
/// says `download-manager` (and `yos ls` shows `app-download-manager`); the arms below were
/// written `download_manager`; a person types `Download Manager`. They are the same app, and
/// which separator arrived should not decide whether the window opens — but it did: a hyphenated
/// id fell through every arm to `_`, so `open_app name=download-manager` logged "Unknown app"
/// after the caller had already been told the launch was under way.
pub fn canonical_id(app: &str) -> String {
    app.trim().to_lowercase().replace([' ', '-'], "_")
}

/// What opening `app` does, if it is one of the shell's own.
pub fn route(app: &str) -> Option<Launch> {
    let id = canonical_id(app);
    ROUTES.iter().find(|(names, _)| names.contains(&id.as_str())).map(|(_, launch)| *launch)
}

/// The surface a route's thing answers on: `shell` for a part of the desktop, Blender's addon for
/// Blender, and nothing for the browser.
///
/// The browser is a window we did not write. It publishes nothing, and the desktop's own surface
/// is not a stand-in for it: a notification button that named the browser and was answered by
/// `shell` would run somebody else's action on this desktop. Blender is not ours either, but it
/// does publish — the addon the route starts it with binds app-blender.sock.
pub fn route_surface(launch: Launch) -> Option<&'static str> {
    match launch {
        Launch::Browser => None,
        Launch::Blender => Some("blender"),
        Launch::Screen(_) | Launch::SettingsSection(_) | Launch::Editor | Launch::Launchpad => {
            Some("shell")
        }
    }
}

/// The name an app answers to on the control surface, given any name it is known by.
///
/// A driver opens `sysmonitor` and then has to describe `system-monitor`; opens `downloads` and
/// describes `download-manager`. Nothing said so anywhere a driver could read, so the second
/// step of the most ordinary job on this desktop — open an app, look at it — was a guess.
///
/// The names are the apps' own and are not decided here: each app's `.desktop` file declares the
/// id it publishes and the other names it answers to, and `crate::surfaces::find` reads them —
/// for an app somebody else wrote exactly as for ours. `shell` and `yantrik` (the name the shell
/// sends its own notifications under) are the desktop, and a screen of the desktop is described
/// as `shell`, because that is the surface it is part of.
pub fn surface_for(app: &str, installed: &[DesktopEntry]) -> Option<String> {
    let key = canonical_id(app);
    if key.is_empty() {
        return None;
    }
    if key == "shell" || key == "yantrik" {
        return Some("shell".to_string());
    }
    if let Some(launch) = route(app) {
        return route_surface(launch).map(str::to_string);
    }
    crate::surfaces::find(app, installed).map(|(surface, _)| surface.id)
}

/// The id the shell registers a `.desktop` entry's window under, which is what `APP_NAMES`, the
/// taskbar and the icon set call it: `sysmonitor` for `yantrik-system-monitor`, the entry id
/// itself for anybody else's.
pub fn window_id(app_id: &str) -> String {
    super::app_grid::icon_id_for(app_id)
}

/// What the shell's own things are FOR, in a few words, for a reader choosing between them.
///
/// Three different models — a local 27B and two hosted ones — were each asked to "write a
/// short document titled Launch Plan and save it", and each wrote a note. They were shown a
/// list of names: `notes` was open with a summary that fitted, and `documents` was one word in
/// a row of closed apps. A name is not a description. When every model makes the same choice,
/// the choice was made here.
///
/// Only the routes' rows. An app's purpose is its `X-Yantrik-Purpose`, written in its own
/// `.desktop` file beside its name — the words above for Notes and yDoc moved there unchanged.
const PURPOSES: &[(&str, &str)] = &[
    ("problems", "what went wrong on this machine, and the report you can choose to send"),
    ("agents", "every agent at work — each mind's conversation, its tool calls and their output, in one list"),
    ("recipes", "every recipe the companion holds, as its stages while it runs — answer the one waiting on you, pause, resume or cancel"),
    ("files", "browse, move, rename and delete files"),
    ("settings", "this desktop's settings"),
    ("browser", "the web"),
    ("launchpad", "every app on this machine, by category, searchable"),
];

/// The launcher's id for an app, given any name the app answers to.
///
/// A mind knows an app by the name it passes to `describe` and `act` — `system-monitor`,
/// `download-manager` — while windows are titled from the launcher's id (`sysmonitor`,
/// `downloads`). `show_app` takes whichever the caller has.
///
/// Read from the entry that opens the app, so every name it answers to arrives at the same
/// window: `container-manager` used to fall through unchanged and `present_app` then looked for a
/// window titled after a name no window carries.
pub fn launcher_id(name: &str) -> String {
    launcher_id_in(name, &crate::apps::Catalogue::shared().get())
}

/// [`launcher_id`], against a catalogue the caller holds.
pub fn launcher_id_in(name: &str, installed: &[DesktopEntry]) -> String {
    let want = name.trim().to_lowercase();
    if route(&want).is_some() {
        return want;
    }
    match crate::surfaces::find(&want, installed) {
        // The id the window is registered and titled under — what the launch hands the registry
        // (`resolve` → `Resolved::Catalogue::id`) — not the surface's.
        Some((_, entry)) => super::app_grid::icon_id_for(&entry.app_id),
        None => want,
    }
}

/// Everything `open_app` will open, for a caller that cannot read this file.
///
/// `open_app(name)` took a name and the shell's state listed none, so a mind had to guess what
/// this desktop calls its apps — and a wrong guess reads, from outside, exactly like a model
/// inventing things. Each entry is the name to pass, what opening it does, and, for an app, the
/// name to `describe` it by, what it is for, the other names it answers to and whether it is
/// running. Shelved apps are left out: they cannot be opened, and `open_app` says why if one is
/// asked for by name.
///
/// Every app whose `.desktop` file declares a surface is here, closed or open — this OS's own and
/// anybody else's alike — listed by its surface id, so the name that opens it is the name that
/// describes it.
pub fn openable() -> Vec<serde_json::Value> {
    openable_in(&crate::apps::Catalogue::shared().get())
}

/// [`openable`], against a catalogue the caller holds.
pub fn openable_in(installed: &[DesktopEntry]) -> Vec<serde_json::Value> {
    openable_with(installed, &|surface| {
        yantrik_app_runtime::service::is_up(&format!("app-{surface}"))
    })
}

/// [`openable_in`], with the question "is this surface's window answering right now" asked of
/// `running` rather than of the socket directory.
pub fn openable_with(
    installed: &[DesktopEntry],
    running: &dyn Fn(&str) -> bool,
) -> Vec<serde_json::Value> {
    let declared = crate::surfaces::declared(installed);
    let mut listed: Vec<serde_json::Value> = ROUTES
        .iter()
        .filter_map(|(names, launch)| {
            let name = *names.first()?;
            if SHELVED.iter().any(|shelf| shelf.ids.contains(&name)) {
                return None;
            }
            let purpose = |id: &str| {
                PURPOSES.iter().find(|(app, _)| *app == id || *app == name).map(|(_, what)| *what)
            };
            let mut entry = match launch {
                Launch::Browser => serde_json::json!({ "name": name, "opens": "web browser" }),
                // An app, and listed as one: opening it brings up a surface a caller can
                // describe and act on, which is the whole reason the route exists. What it is
                // for, and what else it is called, is its own `.desktop` file's to say.
                Launch::Blender => {
                    let mut entry = serde_json::json!({
                        "name": name,
                        "opens": "app",
                        "describe_as": "blender",
                        "running": running("blender"),
                    });
                    let installed_blender = declared.iter().find(|d| d.id == "blender").cloned();
                    if let Some(surface) = installed_blender.or_else(shipped_blender) {
                        describe_declared(&mut entry, &surface);
                    }
                    entry
                }
                // Listed as what it is. It was "a screen of the desktop itself", which is what
                // `yos ls` printed, so the next thing a caller tried was `show_screen
                // screen=launchpad` — refused, because the launcher is not a screen but an
                // overlay on the desktop one. The listing is where a caller learns that.
                Launch::Launchpad => serde_json::json!({
                    "name": name,
                    "opens": "the launcher, over the desktop",
                    "describe_as": "shell",
                }),
                // A section of Settings, not a screen of its own: `show_screen screen=skills` is
                // refused, because Settings is the screen and Skills is a section of it. Listed as
                // what it is, with the exact call that shows it, so a caller does not have to guess
                // (release-check found this by making the same mistake a mind would).
                Launch::SettingsSection(_) => serde_json::json!({
                    "name": name,
                    "opens": "a section of Settings",
                    "describe_as": "shell",
                    "show_with": { "action": "show_screen", "screen": "settings", "section": name },
                }),
                _ => serde_json::json!({ "name": name, "opens": "a screen of the desktop itself", "describe_as": "shell" }),
            };
            if entry.get("for").is_none() {
                if let Some(what) = purpose(name) {
                    entry["for"] = serde_json::Value::String(what.to_string());
                }
            }
            Some(entry)
        })
        .collect();

    // Every surface a .desktop file declares, whether it is running or not. A route already
    // listed one (Blender) under the same id; it is not listed twice.
    for surface in &declared {
        if route(&surface.id).is_some() {
            continue;
        }
        let mut entry = serde_json::json!({
            "name": surface.id,
            "opens": "app",
            "describe_as": surface.id,
            "running": running(&surface.id),
        });
        describe_declared(&mut entry, surface);
        listed.push(entry);
    }
    listed
}

/// Blender's declaration as this OS ships it, for a machine where its entry is not installed.
///
/// The route is listed whether or not Blender is on the disk (`open_app` then says what is
/// missing), and its row should still say what it is for. Read from the shipped file itself
/// (`surfaces::SHIPPED_ENTRIES`) rather than copied here, so the purpose has one home.
fn shipped_blender() -> Option<crate::surfaces::Declared> {
    crate::surfaces::declared(&crate::surfaces::shipped()).into_iter().find(|d| d.id == "blender")
}

/// What a `.desktop` file says about a surface, added to its row in the listing.
fn describe_declared(entry: &mut serde_json::Value, surface: &crate::surfaces::Declared) {
    entry["title"] = serde_json::Value::String(surface.title.clone());
    if !surface.purpose.trim().is_empty() {
        entry["for"] = serde_json::Value::String(surface.purpose.clone());
    }
    if !surface.aliases.is_empty() {
        entry["aliases"] = serde_json::json!(surface.aliases);
    }
}

/// Every name the shell's own routes answer to.
pub fn builtin_app_ids() -> impl Iterator<Item = &'static str> {
    ROUTES.iter().flat_map(|(names, _)| names.iter().copied())
}

/// What `open_app` will do for a name. Decided here, once, and read by the dispatch — so a
/// test can ask the question without a window, a catalogue thread or a spawn.
#[derive(Debug)]
pub enum Resolved {
    /// In the tree, not in this build; carries why.
    Shelved(&'static Shelved),
    /// One of the shell's own: a screen, the launcher, the browser, Blender with its addon.
    Route(Launch),
    /// A program from its .desktop entry: the id its window is registered under, the binary and
    /// its args — and, when the entry declares a surface, which one, and the adapter that
    /// provides it if the app cannot.
    Catalogue {
        id: String,
        bin: String,
        args: Vec<String>,
        surface: Option<String>,
        adapter: Option<String>,
    },
    /// Nothing answers to that name.
    Unknown,
}

/// The shell's own route is asked before the .desktop catalogue, and the order is the point.
///
/// It used to be the other way round, on the reasoning that a pin or the Lens can name an app
/// ("notes") that also has a .desktop entry, and that entry should launch like the route does.
/// It does — for our own apps the two agree by construction. Where they disagree is a program
/// the distribution also ships an entry for: Debian's `blender.desktop` says `Exec=blender %f`,
/// ours says `blender --python …/bootstrap.py`, and both are called "Blender". The catalogue
/// matched first, so `open_app name=blender` opened a Blender without the addon — the window
/// came up, the control surface never did, `describe blender` said "no socket", and the launch
/// had been reported as done. A window a mind can only photograph: the exact thing the launch
/// arm's own comment promises not to open (#96).
///
/// The route is the shell's account of how to open a thing that is part of it. The catalogue is
/// for programs, and within it a declared surface is asked first — by its id, any alias, or what
/// the app is called — so `open_app name=sysmonitor` finds System Monitor through the name its
/// own `.desktop` file gives it, exactly as a third-party app's alias finds that app.
pub fn resolve(app: &str, installed: &[DesktopEntry]) -> Resolved {
    if let Some(shelf) = shelved(app) {
        return Resolved::Shelved(shelf);
    }
    if let Some(launch) = route(app) {
        return Resolved::Route(launch);
    }
    let entry = crate::surfaces::find(app, installed)
        .map(|(_, entry)| entry)
        .or_else(|| catalogue_entry(app, installed));
    if let Some(entry) = entry {
        if let Some(shelf) = shelved_exec(&entry.exec) {
            return Resolved::Shelved(shelf);
        }
        if entry.exec != "__builtin__" {
            let mut parts = entry.exec.split_whitespace().map(str::to_string);
            if let Some(bin) = parts.next() {
                // The surface as the catalogue settled it, not merely as the file wrote it: a
                // declaration that lost its name to the desktop or to another app declares nothing.
                let surface = crate::surfaces::declared(installed)
                    .into_iter()
                    .find(|d| entry.surface.as_deref() == Some(d.id.as_str()))
                    .map(|d| d.id);
                return Resolved::Catalogue {
                    id: super::app_grid::icon_id_for(&entry.app_id),
                    bin,
                    args: parts.collect(),
                    adapter: surface.as_ref().and(entry.adapter.clone()),
                    surface,
                };
            }
        }
    }
    Resolved::Unknown
}

/// The .desktop entry a name refers to, matched the way the dispatch matches it.
fn catalogue_entry<'a>(app: &str, installed: &'a [DesktopEntry]) -> Option<&'a DesktopEntry> {
    let lower = app.trim().to_lowercase();
    installed.iter().find(|e| e.app_id == app || e.name.to_lowercase() == lower)
}

/// Whether an app can be opened on this machine, as it is right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// Opening it opens it.
    Ready,
    /// The shell knows the app, but what it runs is not on this machine. Carries what is missing.
    Missing(String),
    /// The app is in the tree and not in this build. Carries why, and what would bring it back.
    Shelved(&'static Shelved),
    /// Nothing answers to that name.
    Unknown,
}

/// Whether opening `app` will actually open something.
///
/// "Known" was the only question anybody asked, and it is the wrong one. The Browser pin was
/// known — it had an arm — and the arm ran `chromium`, which the installer does not put on the
/// disk. So START showed Browser, `open_app browser` answered "launching", and a click did
/// nothing but log ENOENT. Anything that lists an app or promises to open one asks this instead,
/// which is the dispatch's own decision (`resolve`), followed down to whether the program it
/// would run exists — so the answer and the launch cannot take different paths. (They did:
/// this asked the catalogue before the routes while the launch asked the routes first, so a
/// distribution's `blender.desktop` answered "ready" for a Blender the route would have refused.)
pub fn availability(app: &str, installed: &[DesktopEntry]) -> Availability {
    // The shelf is `resolve`'s first question, so a stale .desktop file and a stale binary left
    // on disk by an earlier release cannot answer Ready for something this build does not ship.
    let launch = match resolve(app, installed) {
        Resolved::Shelved(shelf) => return Availability::Shelved(shelf),
        Resolved::Unknown => return Availability::Unknown,
        Resolved::Catalogue { bin, .. } => return program_availability(&bin),
        Resolved::Route(launch) => launch,
    };
    match launch {
        Launch::Browser => match find_browser() {
            Some(_) => Availability::Ready,
            None => Availability::Missing(format!(
                "a web browser (looked for {})",
                BROWSERS.iter().map(|(name, _)| *name).collect::<Vec<_>>().join(", ")
            )),
        },
        // Both halves are checked: a Blender without the addon is a window a mind can only
        // photograph, and an addon without Blender has nothing to run inside. The refusal
        // names the half that is missing, because the fix is different for each.
        Launch::Blender => match find_program("blender") {
            None => Availability::Missing("blender".to_string()),
            Some(_) => match blender_bootstrap() {
                None => Availability::Missing(
                    "the Yantrik addon for Blender (share/blender/bootstrap.py)".to_string(),
                ),
                Some(_) => match blender_display(std::env::var("DISPLAY").ok().as_deref()) {
                    Ok(()) => Availability::Ready,
                    Err(why) => Availability::Missing(why),
                },
            },
        },
        Launch::Screen(_) | Launch::SettingsSection(_) | Launch::Editor | Launch::Launchpad => {
            Availability::Ready
        }
    }
}

/// Whether the program an Exec line runs is on this machine.
fn program_availability(exec: &str) -> Availability {
    let Some(bin) = exec.split_whitespace().next() else {
        return Availability::Missing("a program to run".to_string());
    };
    match find_program(bin) {
        Some(_) => Availability::Ready,
        None => Availability::Missing(bin.to_string()),
    }
}

/// Whether the launch_app dispatch will do anything with this id.
pub fn is_known_app(app: &str, installed: &[DesktopEntry]) -> bool {
    availability(app, installed) != Availability::Unknown
}

/// Whether opening this app will open it.
pub fn is_launchable(app: &str, installed: &[DesktopEntry]) -> bool {
    availability(app, installed) == Availability::Ready
}

/// Whether a catalogue entry belongs in the launcher at all.
///
/// A tile is a promise that clicking it opens something. A .desktop file can outlive its package,
/// and a built-in tile can name a screen nothing routes to — both were in the launcher, and both
/// did nothing when clicked.
///
/// A shelved app is the third case, and the only one where the program on the disk works
/// perfectly well: an installed machine keeps `/opt/yantrik/bin/yantrik-music-player` and its
/// .desktop entry from whatever release put them there, because the updater installs binaries
/// over binaries and never removes one the new bundle does not carry. Matching the shelf by the
/// program the Exec line runs is what keeps that stale pair out of the launcher.
pub fn entry_is_launchable(entry: &DesktopEntry) -> bool {
    if shelved(&entry.app_id).is_some() || shelved_exec(&entry.exec).is_some() {
        return false;
    }
    if entry.exec == "__builtin__" {
        return route(&entry.app_id).is_some();
    }
    program_availability(&entry.exec) == Availability::Ready
}

/// The names of the apps that will open on this machine, for telling a caller what it can ask for:
/// the shell's own, then every declared surface, by the name the listing gives it.
pub fn launchable_app_ids(installed: &[DesktopEntry]) -> Vec<String> {
    let mut ids: Vec<String> = ROUTES
        .iter()
        .map(|(names, _)| names[0].to_string())
        .filter(|id| is_launchable(id, installed))
        .collect();
    for surface in crate::surfaces::declared(installed) {
        if !ids.contains(&surface.id) && is_launchable(&surface.id, installed) {
            ids.push(surface.id);
        }
    }
    ids
}

/// The web browsers the Browser pin will open, in order of preference, with what each needs.
///
/// Chromium first: it is what this OS installs, and the browser tools drive it. The flags keep
/// what the old hardcoded launch had — native Wayland, no first-run wizard, no default-browser
/// nag. The rest open as they are.
///
/// The profile is the browser's own. The visible browser used to run from
/// `--user-data-dir=/tmp/chromium-visible`, because a headless instance might hold the default
/// profile's lock; the browser tools now keep every profile under ~/.local/share/yantrik/browsers,
/// so that reason is gone, and a /tmp profile forgot every login and bookmark at each reboot.
const BROWSERS: &[(&str, &[&str])] = &[
    ("chromium", CHROMIUM_FLAGS),
    ("chromium-browser", CHROMIUM_FLAGS),
    ("google-chrome-stable", CHROMIUM_FLAGS),
    ("google-chrome", CHROMIUM_FLAGS),
    ("firefox", &[]),
    ("firefox-esr", &[]),
    ("epiphany-browser", &[]),
    ("epiphany", &[]),
];

/// How the desktop starts a Chromium-family browser.
///
/// The three debugging flags are what make the browser one of this desktop's apps rather than a
/// window a mind can only look at. `yos web` — and through it a harness's `web_read`, `web_find`,
/// `web_click` — drives the page over the DevTools protocol, and the browser this launcher
/// started had no DevTools port: the first mind asked to research something was told "no browser
/// with a debug port on 9222", on a desktop whose dock had a browser open. The built-in companion
/// never hit it because it launches its own.
///
/// Loopback only, and only the origin `yos` itself presents: `--remote-allow-origins=*` would
/// let any web page's script open the socket if it learned a target id. What this does grant is
/// what it sounds like — a process running as this user can drive this browser — which such a
/// process could already do by reading the profile on disk. The gates a MIND meets are at the
/// bridge: plan mode, the taint rule, and the refusal to press anything that reads as a
/// commitment.
const CHROMIUM_FLAGS: &[&str] = &[
    "--ozone-platform=wayland",
    "--no-first-run",
    "--no-default-browser-check",
    // No "--disable-gpu". It was here on the reasoning that a VM has no GPU, and it cost every
    // machine WebGL: the flag also switches off SwiftShader, the software renderer Chromium
    // uses precisely when there is no GPU. The desktop Browser answered
    // getContext("webgl") with null for every page, and no Arcade game could draw in it (#133).
    // The browser tools already leave it out when headed, for the same reason.
    "--remote-debugging-address=127.0.0.1",
    "--remote-debugging-port=9222",
    "--remote-allow-origins=http://127.0.0.1:9222",
];

/// The first browser from `BROWSERS` this machine has.
pub fn find_browser() -> Option<(&'static str, &'static [&'static str])> {
    BROWSERS.iter().copied().find(|(name, _)| find_program(name).is_some())
}

/// The Blender addon's entry point, where a release stages it.
///
/// `blender --python <this file>` is what makes a Blender this desktop opened into a Blender
/// this desktop can talk to: the bootstrap binds app-blender.sock before the first frame is
/// drawn. The deploy path first, then the same layout relative to the shell's own binary,
/// which is what a development tree or a relocated install has.
pub fn blender_bootstrap() -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from("/opt/yantrik/share/blender/bootstrap.py")];
    if let Some(root) = std::env::current_exe()
        .ok()
        .as_ref()
        .and_then(|exe| exe.parent())
        .and_then(|bin| bin.parent())
    {
        candidates.push(root.join("share/blender/bootstrap.py"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Whether Blender has a display it can open a window on, given the session's `DISPLAY`.
///
/// Debian's `blender` package is built with the X11 GHOST back-end only. On this desktop the
/// session is Wayland, so the window Blender opens is an X11 window served by Xwayland, and the
/// only thing that tells a client where that server is, is `DISPLAY`. labwc sets it for every
/// child it starts — when Xwayland is installed. When it is not, `DISPLAY` is absent, and
/// Blender's whole contribution is:
///
///     GHOST: failed to initialize display for back-end(s): ['X11']
///     GHOST: unable to initialize, exiting!
///
/// after `open_app` has already answered "launching" (#96). The addon still works headless —
/// the control surface comes up and renders — but the person sees nothing, which defeats the
/// point of an app whose value is that the mind and the person look at the same screen.
///
/// The check reads only the variable, on purpose. Whether the server behind it is alive is
/// Xwayland's business (labwc starts it on the first connection); whether this Blender was built
/// with a Wayland back-end instead is not knowable without running it. An absent `DISPLAY` is
/// the one case that is certain, and it is the case the ISO produces when the package is missing.
pub fn blender_display(display: Option<&str>) -> Result<(), String> {
    match display {
        Some(d) if !d.trim().is_empty() => Ok(()),
        _ => Err(
            "an X display for Blender's window: this Blender speaks X11 only, and no DISPLAY is              set, so Xwayland is not running in this session (is the xwayland package installed?)"
                .to_string(),
        ),
    }
}

/// A child that did not exit cleanly leaves a problem record, written by the shell on its behalf.
///
/// An app that panics writes its own record through the runtime's hook; this covers what a hook
/// cannot: a segfault in a native library, an abort, a kill, an exit code the app chose. Nothing is
/// sent anywhere — the record is a local file the "Report a problem" screen can show. A clean
/// exit is not a problem, and neither is SIGTERM, which is what the shell itself sends to close
/// an app; recording those would bury the real ones.
fn record_crash(name: &str, status: &std::process::ExitStatus, lived_ms: u64) {
    use std::os::unix::process::ExitStatusExt as _;
    if status.success() || status.signal() == Some(15) {
        return;
    }
    let message = format!("{status} after {lived_ms} ms");
    let record = yantrik_app_runtime::problems::problem("crash", name, &message, None, None);
    if let Some(path) = yantrik_app_runtime::problems::write(&record) {
        tracing::info!(app = %name, path = %path.display(), "Problem record written for the crash");
    }
}

/// Where a program is, if it is anywhere it could be run from.
///
/// The shell is started from `/opt/yantrik/bin` (or a cargo target dir in development), and the
/// apps are deployed beside it — but nothing puts that directory on PATH, so a bare
/// `Command::new("yantrik-notes")` fails with ENOENT on a clean install. Prefer the shell's own
/// directory, then the deploy path, then PATH.
pub fn find_program(bin: &str) -> Option<PathBuf> {
    if bin.is_empty() {
        return None;
    }
    if bin.contains('/') {
        let path = PathBuf::from(bin);
        return is_executable(&path).then_some(path);
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) {
        dirs.push(dir);
    }
    dirs.push(PathBuf::from("/opt/yantrik/bin"));
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.into_iter().map(|dir| dir.join(bin)).find(|p| is_executable(p))
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Wire on_launch_app callback.
pub fn wire(ui: &App, ctx: &AppContext) {
    let catalogue = ctx.installed_apps.clone();
    let ui_weak = ui.as_weak();
    // The catalogue follows the application directories from here on, and every declared alias
    // is linked at its surface's socket — see `crate::surfaces::watch`.
    crate::surfaces::watch(catalogue.clone());

    ui.on_launch_app(move |app_id| {
        let app = app_id.to_string();
        tracing::info!(app = %app, "Launching app");

        // The shelf, before anything that could run a program. `check_launchable` already
        // refuses `open_app`, but this callback is also reached by a tile, a pin and by
        // `invoke_launch_app` from anywhere in the shell, and the binary is still on the disk of
        // every machine that installed an earlier release — so the last gate before spawn says no
        // as well, and says why.
        // One decision, made in `resolve` where the tests can reach it. The shell's own route
        // is asked before the .desktop catalogue — see `resolve` for the Blender that taught us.
        let installed = catalogue.get();
        let launch = match resolve(&app, &installed) {
            Resolved::Shelved(shelf) => {
                tracing::warn!(
                    app = %app,
                    "{} is not part of this build: {}. It comes back when {}.",
                    shelf.name, shelf.reason, shelf.returns_when
                );
                return;
            }
            Resolved::Catalogue { id, bin, args, surface, adapter } => {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                // An app that cannot host its own surface has its adapter started beside it,
                // and stopped with it.
                let adapter = surface.as_deref().zip(adapter.as_deref());
                spawn_launch(&id, &bin, &args, None, adapter);
                return;
            }
            Resolved::Unknown => {
                tracing::warn!(app = %app, "Unknown app");
                return;
            }
            Resolved::Route(launch) => launch,
        };
        let show = |screen: i32| {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_current_screen(screen);
                ui.invoke_navigate(screen);
            }
        };
        match launch {
            Launch::Screen(screen) => show(screen),
            Launch::SettingsSection(section) => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_settings_category(section);
                }
                show(7);
            }
            Launch::Editor => spawn_app("editor", "yantrik-text-editor"),
            // This does open the launcher: the grid's `changed` handler fires and the
            // catalogue is rescanned, which is the "Scanned .desktop files count=31" line that
            // followed every `open_app name=launchpad` in the log. What it does not do is put
            // the launcher where anyone can see it. The shell is one fullscreen toplevel under
            // labwc, so with an app window in front the grid opens underneath it — the
            // photograph of "nothing" is Studio, or the Editor, exactly as before (#71, #118).
            // Raising the shell is the caller's job, done in `control::open_launcher`, because
            // that is where the answer is built and the raise can be reported with it; a person
            // reaching this arm from the desktop's own menu is already looking at the shell.
            Launch::Launchpad => {
                show(1);
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_app_grid_open(true);
                }
            }
            // A browser is a window like any other, so it goes through the one launcher: the
            // registry learns it is open, the reaper notices if it dies at once, and it gets the
            // session's display environment. The old arm set WAYLAND_DISPLAY and XDG_RUNTIME_DIR
            // by hand, for uid 1000 only.
            Launch::Browser => match find_browser() {
                Some((bin, args)) => spawn_app_with_args("browser", bin, args),
                None => tracing::error!(
                    "Cannot open the browser: none is installed (looked for {})",
                    BROWSERS.iter().map(|(name, _)| *name).collect::<Vec<_>>().join(", ")
                ),
            },
            // Blender opens through the one launcher like any other window — registry, reaper,
            // session environment — with the addon as its argument. A Blender started without
            // `--python` would still be Blender, but a window a mind can only photograph, and
            // the refusal below says so rather than quietly opening the lesser thing.
            Launch::Blender => match (find_program("blender"), blender_bootstrap()) {
                (Some(bin), Some(bootstrap)) => {
                    // Third check, same reason as the other two: a launch that dies in under a
                    // second is a launch the shell reported and nobody saw. Debian's Blender
                    // speaks X11 only, so without an X display it prints one GHOST line and
                    // exits, and `open_app` had already answered "launching".
                    if let Err(why) = blender_display(std::env::var("DISPLAY").ok().as_deref()) {
                        tracing::error!("Cannot open Blender: {why}");
                        return;
                    }
                    let bin = bin.to_string_lossy().into_owned();
                    let bootstrap = bootstrap.to_string_lossy().into_owned();
                    spawn_app_with_args("blender", &bin, &["--python", &bootstrap]);
                }
                (None, _) => {
                    tracing::error!("Cannot open Blender: it is not installed on this machine")
                }
                (_, None) => tracing::error!(
                    "Cannot open Blender: the Yantrik addon is missing \
                     (expected /opt/yantrik/share/blender/bootstrap.py), and a Blender \
                     without it answers to nothing"
                ),
            },
        }
    });
}

/// Where an app binary lives, or the bare name for `Command` to look up if it is nowhere.
pub fn resolve_app_binary(bin: &str) -> PathBuf {
    find_program(bin).unwrap_or_else(|| PathBuf::from(bin))
}

/// Launch a standalone app binary. The app's own single-instance guard handles repeats.
/// Launch a windowed app under a logical id.
///
/// `app_id` is the id the app is known by everywhere a caller reads it — the dock, `open_app`,
/// `describe shell` — and `bin` is the binary to run. They differ (`notes` vs `yantrik-notes`),
/// and the id is what the running-apps registry is keyed on, so "what is open" answers in the
/// same vocabulary a caller uses to open things.
pub fn spawn_app(app_id: &str, bin: &str) {
    spawn_app_with_args(app_id, bin, &[]);
}

/// The one place the shell starts an app process, whatever path asked for it.
pub fn spawn_app_with_args(app_id: &str, bin: &str, args: &[&str]) {
    spawn_app_in(app_id, bin, args, None)
}

/// The same launcher, started in a particular directory.
///
/// For "open a terminal here", where the directory IS the request. Goes through one body with
/// `spawn_app_with_args` so the registry, the environment scrubbing and the reaper cannot end up
/// applying to one launch path and not the other — which is how the dock grew two of them before.
/// The display environment a launched app needs, which the shell itself does not have.
///
/// This is the whole of why third-party software did not run.
///
/// labwc starts Xwayland and the socket is there — /tmp/.X11-unix/X0 exists — but nothing in
/// the session exports DISPLAY, and a child inherits only what the shell has. So Chromium,
/// whose .desktop file says `Exec=/usr/bin/chromium %U`, started, failed to find an X server,
/// printed "Missing X server or $DISPLAY", and exited. `open_app` had already answered
/// `accepted: true`. Setting DISPLAY is the entire fix: with it, the same command opens a
/// window.
///
/// The toolkit hints are the other half of the same thought. GTK and Qt both take a
/// preference LIST, so a Wayland-native app uses Wayland and one that cannot falls back to
/// Xwayland on its own. Neither is forced, and an app that already sets them keeps its choice.
///
/// An app's adapter gets the same environment (`crate::surfaces::start_adapter`): it drives the
/// app, and has to be able to reach whatever the app reaches.
pub(crate) fn session_env() -> Vec<(&'static str, String)> {
    let mut env = Vec::new();

    if std::env::var_os("DISPLAY").is_none() {
        if let Some(display) = x_display() {
            env.push(("DISPLAY", display));
        }
    }
    if std::env::var_os("GDK_BACKEND").is_none() {
        env.push(("GDK_BACKEND", "wayland,x11".to_string()));
    }
    if std::env::var_os("QT_QPA_PLATFORM").is_none() {
        env.push(("QT_QPA_PLATFORM", "wayland;xcb".to_string()));
    }
    env
}

/// Which X display Xwayland is serving, read from its socket rather than assumed.
///
/// `:0` is the usual answer and hardcoding it would work today, but the number is chosen by
/// whoever started Xwayland, and a session that already had one running gets `:1`.
fn x_display() -> Option<String> {
    let dir = std::fs::read_dir("/tmp/.X11-unix").ok()?;
    let mut numbers: Vec<u32> = dir
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            name.strip_prefix('X')?.parse::<u32>().ok()
        })
        .collect();
    numbers.sort_unstable();
    numbers.first().map(|n| format!(":{n}"))
}

pub fn spawn_app_in(app_id: &str, bin: &str, args: &[&str], dir: Option<&std::path::Path>) {
    spawn_launch(app_id, bin, args, dir, None)
}

/// What the reaper makes of an app whose `child.wait()` has returned.
#[derive(Debug, PartialEq, Eq)]
enum ExitVerdict {
    /// A second copy of a single-instance app, which exits at once and leaves the window already
    /// open — raised here if it was on screen. Its adapter stays where it is.
    SecondCopy,
    /// The launch died before it could show a window: a non-zero exit or a signal inside the grace
    /// period. This is what `describe shell` reports under `failed_launches`.
    Failed,
    /// The app ran and exited: past the grace period, or cleanly inside it. Not a failure.
    Exited,
}

/// What an app's exit means, decided from the facts the reaper has. Pure, and kept apart from the
/// wait, so the cases two issues turned on can be tested without spawning a process.
///
/// `brought_forward` is whether the app's window was on screen at the moment of exit and has just
/// been raised — the `present_app` call, which asks the compositor, is I/O the caller does, and
/// only for an exit that could be a handover.
///
/// A second copy of a single-instance app exits at once, on purpose. Three comments in this shell
/// once said it "asks the window that is already open to show itself"; nothing did, so clicking
/// the tile of an app that was open behind something did nothing a person could see, and after a
/// shell restart — the registry no longer knowing the app was open — the same click was recorded
/// as a failed launch and raised a notice about it. The handover is therefore recognised here,
/// where the exit is seen: a clean exit inside the grace with the window on screen IS the
/// handover, whoever the registry thought owned the record, and so is a clean exit inside the
/// grace from a launch that never owned the record because a live window held it.
///
/// Inside the grace, only an exit that was NOT clean — a non-zero status or a signal — is a failed
/// launch (#200, #168). A clean exit inside the grace is a window that was opened and then closed
/// on purpose: a person closing Studio two seconds after opening it, `close_window` doing the
/// same, or the browser check that opens a page, reads it and quits, all within about 2 s. Those
/// used to be recorded as a launch that never showed a window, with status `exit status: 0`, so a
/// caller reading `failed_launches` believed an app could not start when it had started and
/// closed — and `release-check --tier rc` failed its last check on the browser's own clean close.
/// An app that genuinely cannot reach the display panics before its event loop, and a panic is a
/// non-zero exit, so status 0 inside the grace is evidence the app ran.
fn exit_verdict(lived_ms: u64, status_success: bool, owns_window: bool, brought_forward: bool) -> ExitVerdict {
    let inside_grace = lived_ms < crate::running::LAUNCH_GRACE_MS;
    if brought_forward || (inside_grace && !owns_window && status_success) {
        ExitVerdict::SecondCopy
    } else if inside_grace && !status_success {
        ExitVerdict::Failed
    } else {
        ExitVerdict::Exited
    }
}

/// The body of every launch, with the adapter a `.desktop` file may declare for the app.
///
/// `adapter` is `(surface, command)`: started once the app process exists, told which process it
/// serves, and stopped when that process exits — by the same reaper that already watches the app,
/// so an adapter cannot outlive its app on one launch path and not another.
fn spawn_launch(
    app_id: &str,
    bin: &str,
    args: &[&str],
    dir: Option<&std::path::Path>,
    adapter: Option<(&str, &str)>,
) {
    let path = resolve_app_binary(bin);
    let mut command = std::process::Command::new(&path);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    command.args(args);
    for (key, value) in session_env() {
        command.env(key, value);
    }
    match command
        // The shell is often started with SLINT_FULLSCREEN=1 (dev runs, kiosk sessions). A child
        // inherits the environment, and an app that inherits that variable opens fullscreen too.
        // The renderer choice (SLINT_BACKEND, GALLIUM_DRIVER) is deliberately left inherited so
        // apps draw with the same backend the shell settled on.
        .env_remove("SLINT_FULLSCREEN")
        .stdin(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            let pid = child.id();
            tracing::info!(app = app_id, bin, pid, path = %path.display(), "App launched");
            // The shell now knows this window is open without asking the compositor. Recorded
            // before the reaper thread starts, so a describe that lands in the same instant sees
            // it.
            let owns_window = crate::running::mark_launched(app_id, pid, bin);
            if let Some((surface, command)) = adapter {
                crate::surfaces::start_adapter(surface, command, pid);
            }
            // Reap it when it exits. Without a wait, every app the shell ever launched lingers
            // as a zombie until the shell itself quits — and a zombie still has a /proc entry,
            // which is enough to confuse anything that checks "is that pid alive". The same wait
            // is where the registry learns the window has closed.
            let id = app_id.to_string();
            let name = bin.to_string();
            // A launch is not a window. Spawning succeeds for anything executable, so an app that
            // came up and one that did not both start, and the exit this thread waits for is what
            // tells them apart. It used to be logged at info and discarded, which is how
            // "accepted: true" and an empty screen could both be true at once. What an exit means
            // is decided in `exit_verdict`, where the cases can be tested.
            crate::running::clear_launch_failure(&id);
            let started = std::time::Instant::now();
            std::thread::spawn(move || {
                // Whether this exit was a second copy handing over to a window already open,
                // which leaves that window's adapter where it is.
                let mut second_copy = false;
                match child.wait() {
                    Ok(status) => {
                        let lived_ms = started.elapsed().as_millis() as u64;
                        let success = status.success();
                        // Raising the window asks the compositor, so it is done only for the one
                        // exit that could be a handover — a clean one inside the grace period —
                        // and never for a crash or an app that lived its life.
                        let brought_forward = lived_ms < crate::running::LAUNCH_GRACE_MS
                            && success
                            && crate::windows::present_app(&id);
                        match exit_verdict(lived_ms, success, owns_window, brought_forward) {
                            ExitVerdict::SecondCopy => {
                                second_copy = true;
                                tracing::info!(
                                    app = %name, lived_ms, brought_forward,
                                    "A second copy handed over to the window already open"
                                );
                            }
                            ExitVerdict::Failed => {
                                tracing::warn!(
                                    app = %name, %status, lived_ms,
                                    "App exited immediately — it never showed a window"
                                );
                                crate::running::mark_launch_failed(
                                    &id, &name, &status.to_string(), lived_ms,
                                );
                                record_crash(&name, &status, lived_ms);
                            }
                            ExitVerdict::Exited => {
                                // A clean exit inside the grace period lands here too: the window
                                // was opened and closed again, which the log says in so many
                                // milliseconds rather than calling it a launch that never was.
                                tracing::info!(app = %name, %status, lived_ms, "App exited");
                                record_crash(&name, &status, lived_ms);
                            }
                        }
                    }
                    Err(e) => tracing::warn!(app = %name, error = %e, "Could not wait for app"),
                }
                crate::surfaces::app_exited(pid, second_copy);
                crate::running::mark_exited(&id, pid);
            });
        }
        Err(e) => tracing::error!(app = app_id, bin, path = %path.display(), error = %e, "Failed to launch app"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The id each app under `apps/` publishes on its control surface, read out of the call that
    /// publishes it — `(app directory, id)`.
    ///
    /// This is the vocabulary an agent actually has: it reads an id from `yos ls` or from an
    /// app's own describe, and hands that back to `open_app`. Anything it can describe, it must
    /// be able to open.
    ///
    /// It was a hand-written list of eight ids and six of the fourteen were missing from it —
    /// among the missing, the rows whose other spellings reached no surface at all. A list of
    /// names kept by hand beside the names themselves is the shape of the bug this file is being
    /// changed for, so the ids are read from the apps instead. `windows::app_name_tests` reads
    /// the same tree for the same reason.
    ///
    /// An app that publishes nothing (the two on the shelf) has no `App::new` to find and is
    /// simply absent, which is what it should be.
    fn published_ids() -> Vec<(String, String)> {
        let apps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps");
        let Ok(entries) = std::fs::read_dir(&apps) else {
            // Packaged source without the apps tree; nothing to read the ids out of.
            return Vec::new();
        };
        let mut found: Vec<(String, String)> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let dir = e.file_name().into_string().ok()?;
                let source = std::fs::read_to_string(e.path().join("src/main.rs")).ok()?;
                published_id(&source).map(|id| (dir, id))
            })
            .collect();
        found.sort();
        assert!(
            found.len() > 10,
            "only {} apps were read out of {}; the scan has stopped finding them",
            found.len(),
            apps.display()
        );
        found
    }

    /// The id one app publishes, from its `App::new(…)`.
    ///
    /// Matched on the exact type name so that `EmailApp::new()` — a window, not a surface — is
    /// not mistaken for one, and the argument is taken either as the literal or through the
    /// file's own `APP_ID`, because three apps name it that way.
    fn published_id(source: &str) -> Option<String> {
        for (at, _) in source.match_indices("::new(") {
            let before = &source[..at];
            let receiver = before
                .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()
                .unwrap_or_default();
            if !matches!(receiver, "App" | "Surface" | "ControlSurface") {
                continue;
            }
            let argument = source[at + "::new(".len()..].split(')').next()?.trim();
            if let Some(literal) = argument.strip_prefix('"') {
                return literal.split('"').next().map(str::to_string);
            }
            // `App::new(APP_ID)`, with the id a const at the top of the same file.
            let declaration = format!("const {argument}: &str = \"");
            let value = source.find(&declaration)? + declaration.len();
            return source[value..].split('"').next().map(str::to_string);
        }
        None
    }

    /// The catalogue as a machine that installed this build has it: the shell's own screens and
    /// every `.desktop` file this OS ships, minus the shelf.
    fn shipped() -> Vec<DesktopEntry> {
        crate::surfaces::shipped_catalogue()
    }

    #[test]
    fn every_app_with_a_control_surface_opens_by_the_id_it_publishes() {
        let shipped = shipped();
        for (dir, id) in published_ids() {
            assert!(
                is_known_app(&id, &shipped),
                "apps/{dir} publishes `{id}`, so an agent will ask for it by that name"
            );
            assert_eq!(
                surface_for(&id, &shipped).as_deref(),
                Some(id.as_str()),
                "apps/{dir} publishes `{id}` and the catalogue does not know it by that name"
            );
        }
    }

    /// Every name that opens an app also describes it, and is a name its socket answers to.
    ///
    /// The container manager was opened as `containers` or as `container_manager`, was
    /// `yantrik-container-manager` in `/opt/yantrik/bin`, and published `containers`. `yos ls`
    /// showed `app-containers`, `describe containers` answered, and `describe container-manager`
    /// said "no socket for 'container-manager'" — so a mind that found the app by the name
    /// everything else calls it and then asked it to describe itself was refused by an app that
    /// was open in front of it.
    ///
    /// The names are the app's own `.desktop` file's now, and what reaches a socket is the id and
    /// the aliases (the shell links each alias there). So the name on the binary and the id the
    /// shell registers the window under must each be one of those: a name that opens the app and
    /// is not linked at its socket is the refusal above under a different word.
    #[test]
    fn every_launchable_name_reaches_a_surface() {
        let shipped = shipped();
        let declared = crate::surfaces::declared(&shipped);
        assert!(declared.len() > 10, "only {} shipped surfaces were read", declared.len());
        for surface in &declared {
            let entry = shipped.iter().find(|e| e.app_id == surface.entry).expect("its entry");
            let program = entry.exec.split_whitespace().next().unwrap_or_default();
            let program = program.rsplit('/').next().unwrap_or(program);
            let program = program.strip_prefix("yantrik-").unwrap_or(program);
            let window = super::super::app_grid::icon_id_for(&entry.app_id);
            let on_the_socket = |name: &str| {
                let name = crate::apps::fold_name(name);
                surface.id == name || surface.aliases.contains(&name)
            };
            for name in [program, window.as_str()] {
                assert!(
                    on_the_socket(name),
                    "`{name}` opens `{}` and is not among its names on the socket bus, so \
                     `describe {name}` is refused. Add it to X-Yantrik-Aliases in {}.desktop.",
                    surface.id,
                    surface.entry
                );
            }
            for name in std::iter::once(&surface.id)
                .chain(surface.aliases.iter())
                .map(String::as_str)
                .chain([program, window.as_str(), entry.name.as_str(), entry.app_id.as_str()])
            {
                assert_eq!(
                    surface_for(name, &shipped).as_deref(),
                    Some(surface.id.as_str()),
                    "`open_app name={name}` opens the app that publishes `{}`, and `describe` \
                     has to reach it",
                    surface.id
                );
            }
        }
        // A screen of the desktop is part of the desktop's own surface, which is what a caller
        // has to describe to see it.
        assert_eq!(surface_for("files", &shipped).as_deref(), Some("shell"));
        assert_eq!(surface_for("settings", &shipped).as_deref(), Some("shell"));
        assert_eq!(surface_for("yantrik", &shipped).as_deref(), Some("shell"));
        assert_eq!(surface_for("no-such-app", &shipped), None);
        // Chromium is not one of ours and the desktop's own surface does not answer for it.
        assert_eq!(surface_for("browser", &shipped), None);
        // Blender is not one of ours either, but it does answer: the addon the route starts it
        // with binds app-blender.sock.
        assert_eq!(surface_for("blender", &shipped).as_deref(), Some("blender"));
        assert_eq!(route("blender"), Some(Launch::Blender));
        // The mismatches this is really about, spelled out, so the intent survives a refactor.
        assert_eq!(surface_for("container-manager", &shipped).as_deref(), Some("containers"));
        assert_eq!(surface_for("sysmonitor", &shipped).as_deref(), Some("system-monitor"));
        assert_eq!(surface_for("text_editor", &shipped).as_deref(), Some("editor"));
        assert_eq!(surface_for("Downloads", &shipped).as_deref(), Some("download-manager"));
        assert_eq!(surface_for("yDoc", &shipped).as_deref(), Some("documents"));
    }

    /// Nothing is written down twice: every app under `apps/` that publishes a surface declares
    /// that id in its own `.desktop` file, and the file declares nothing else.
    ///
    /// The id is read from the app's `App::new(…)`, and the file is what the shell lists the app
    /// by while it is closed — two statements of one fact, held to each other here, which is
    /// what the apps' old alias table in the runtime and the route table in this file were, with
    /// a third copy of every purpose beside them.
    #[test]
    fn every_app_that_publishes_a_surface_declares_it_in_its_desktop_file() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop-files");
        for (dir, id) in published_ids() {
            let file = root.join(format!("yantrik-{dir}.desktop"));
            let text = std::fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("apps/{dir} publishes `{id}` and {} is unreadable: {e}", file.display()));
            let entry = crate::apps::parse_desktop_text(&format!("yantrik-{dir}"), &text)
                .unwrap_or_else(|| panic!("{} is not an application entry", file.display()));
            assert_eq!(
                entry.surface.as_deref(),
                Some(id.as_str()),
                "apps/{dir} publishes `{id}`; {} must say X-Yantrik-Surface={id}, or the app is \
                 not listed while it is closed",
                file.display()
            );
            assert!(
                entry.purpose.chars().count() > 8,
                "{} does not say what the app is for (X-Yantrik-Purpose)",
                file.display()
            );
        }
    }

    /// The apps in apps/, by the name each binary carries.
    ///
    /// `yantrik-network-manager` is launched by typing `network-manager` long before anyone
    /// learns the arm is called `network`, and that is the name `open_app` gets. Three of these
    /// had no arm at all — network-manager, text-editor and image-viewer — and the first two
    /// were ACCEPTED by the guard because a .desktop entry matched, so the call reported success
    /// and did nothing.
    const SHIPPED_APPS: &[&str] = &[
        "calendar", "container-manager", "document-editor", "download-manager", "email",
        "image-viewer", "music-player", "network-manager", "notes", "presentation",
        "snippet-manager", "spreadsheet", "studio", "system-monitor", "terminal", "text-editor",
        "weather",
    ];

    /// Whether `open_app` would launch a program for this name, on a machine with this build.
    fn opens_a_program(name: &str, installed: &[DesktopEntry]) -> bool {
        matches!(resolve(name, installed), Resolved::Catalogue { .. })
    }

    #[test]
    fn every_app_we_ship_opens_by_the_name_of_its_binary() {
        let shipped = shipped();
        for app in SHIPPED_APPS {
            if shelved(app).is_some() {
                continue;
            }
            assert!(
                opens_a_program(app, &shipped),
                "apps/{app} ships a binary that `open_app name={app}` cannot launch"
            );
        }
    }

    /// The id an app launches under is the id the rest of the shell must know it by.
    ///
    /// `windows::merge_windows` pairs the launch registry against the compositor's snapshot by
    /// app id: the registry names a window by the id it was launched under, and the snapshot
    /// names it by resolving the window's title through `APP_NAMES`. When those two disagree the
    /// merge sees two applications and keeps both, so one open window becomes two rows in
    /// `describe shell` and two buttons in the taskbar.
    ///
    /// That is not hypothetical. The image viewer launched under `images` while every other table
    /// — `APP_NAMES`, `STEM_TO_ID`, the icon map — called it `image`, and it was listed twice for
    /// as long as it was open (#93). Whatever name an app is opened by, it launches under the id
    /// its entry maps to, and that id has to be one the shell knows.
    #[test]
    fn an_app_launches_under_a_name_the_rest_of_the_shell_knows() {
        let known: Vec<&str> = crate::windows::APP_NAMES.iter().map(|(id, _)| *id).collect();
        let shipped = shipped();
        for surface in crate::surfaces::declared(&shipped) {
            for name in std::iter::once(&surface.id).chain(surface.aliases.iter()) {
                match resolve(name, &shipped) {
                    Resolved::Catalogue { id, bin, .. } => assert!(
                        known.contains(&id.as_str()),
                        "`{bin}` launches under id `{id}` (opened as `{name}`), which no APP_NAMES \
                         row matches. The shell would list one of its windows twice."
                    ),
                    // Blender opens through its route, which registers it as `blender`.
                    Resolved::Route(Launch::Blender) => {}
                    other => panic!("`{name}` is a declared surface and resolves to {other:?}"),
                }
            }
        }
    }

    /// Every app under `apps/` is either shipped or shelved, and never both.
    ///
    /// The two lists are what a reader compares to answer "what is in this build", so a name that
    /// is in neither, or in both, is the drift this whole table exists to prevent.
    #[test]
    fn an_app_is_either_shipped_or_shelved() {
        let shipped = shipped();
        for app in SHIPPED_APPS {
            let on_shelf = shelved(app).is_some();
            let opens = opens_a_program(app, &shipped);
            assert!(
                on_shelf != opens,
                "apps/{app} is {}",
                if on_shelf { "both shelved and opened" } else { "neither shelved nor opened" }
            );
        }
        // And every shelf entry names an app that is really there. A shelf row for something
        // that has been deleted refuses a name nothing would ever ask for.
        for shelf in SHELVED {
            assert!(
                SHIPPED_APPS.iter().any(|a| shelved(a).map(|s| s.binary) == Some(shelf.binary)),
                "the shelf names {}, which is not one of the apps in apps/",
                shelf.binary
            );
        }
    }

    // ── An app somebody else wrote, declared the same way ──

    /// A LibreOffice-shaped app that declares a surface, an alias per program and an adapter.
    fn third_party() -> Vec<DesktopEntry> {
        let mut installed = shipped();
        let entry = |stem: &str, text: &str| crate::apps::parse_desktop_text(stem, text).unwrap();
        installed.push(entry(
            "libreoffice-writer",
            "[Desktop Entry]\nType=Application\nName=LibreOffice Writer\nExec=/opt/yantrik-test-nowhere/libreoffice --writer %U\n\
             X-Yantrik-Surface=libreoffice\nX-Yantrik-Purpose=Documents, spreadsheets and slides: open, read, edit and export them\n\
             X-Yantrik-Aliases=writer;office\nX-Yantrik-Adapter=/usr/lib/yantrik/adapters/libreoffice\n",
        ));
        installed.push(entry(
            "libreoffice-calc",
            "[Desktop Entry]\nType=Application\nName=LibreOffice Calc\nExec=/opt/yantrik-test-nowhere/libreoffice --calc %U\n\
             X-Yantrik-Surface=libreoffice\nX-Yantrik-Aliases=calc\nX-Yantrik-Adapter=/usr/lib/yantrik/adapters/libreoffice\n",
        ));
        // And one that reaches for names that are not its to take. Between two apps, an alias
        // goes to the first in the catalogue's order (by name, as the scan sorts it), so this one
        // sorts last and loses `writer` to LibreOffice as well.
        installed.push(entry(
            "greedy",
            "[Desktop Entry]\nType=Application\nName=Zz Greedy\nExec=/usr/bin/greedy\n\
             X-Yantrik-Surface=greedy\nX-Yantrik-Aliases=files;notes;shell;writer;sysmonitor;greed\n",
        ));
        installed.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        installed
    }

    /// A third-party surface is listed while it is closed, with what the author wrote about it,
    /// exactly as ours are.
    #[test]
    fn a_third_party_surface_is_listed_while_closed_like_ours() {
        let installed = third_party();
        let listed = openable_with(&installed, &|surface| surface == "notes");
        let lo = listed.iter().find(|a| a["name"] == "libreoffice").expect("libreoffice is listed");
        assert_eq!(lo["opens"], "app");
        assert_eq!(lo["describe_as"], "libreoffice");
        assert_eq!(lo["running"], false);
        assert_eq!(lo["title"], "LibreOffice Calc", "the first entry in the catalogue's order names it");
        assert!(lo["for"].as_str().is_some_and(|f| f.starts_with("Documents")), "{lo}");
        assert_eq!(lo["aliases"], serde_json::json!(["calc", "writer", "office"]), "{lo}");
        // One row per surface, however many entries declare it.
        assert_eq!(listed.iter().filter(|a| a["describe_as"] == "libreoffice").count(), 1);
        // Ours, in the same shape.
        let notes = listed.iter().find(|a| a["name"] == "notes").expect("notes is listed");
        assert_eq!(notes["running"], true);
        assert_eq!(notes["describe_as"], "notes");
        // Names that belong to the desktop or to another app are not handed out.
        let greedy = listed.iter().find(|a| a["name"] == "greedy").expect("greedy is listed");
        assert_eq!(greedy["aliases"], serde_json::json!(["greed"]), "{greedy}");
    }

    /// It opens by its id, by any alias, and by the program each alias names — with its adapter.
    #[test]
    fn a_third_party_surface_opens_by_its_id_and_every_alias() {
        let installed = third_party();
        for (name, program_arg) in [
            ("libreoffice", "--calc"),
            ("calc", "--calc"),
            ("writer", "--writer"),
            ("Office", "--writer"),
            ("LibreOffice Writer", "--writer"),
        ] {
            match resolve(name, &installed) {
                Resolved::Catalogue { bin, args, surface, adapter, .. } => {
                    assert_eq!(bin, "/opt/yantrik-test-nowhere/libreoffice", "{name}");
                    assert_eq!(args.first().map(String::as_str), Some(program_arg), "{name}");
                    assert_eq!(surface.as_deref(), Some("libreoffice"), "{name}");
                    assert_eq!(adapter.as_deref(), Some("/usr/lib/yantrik/adapters/libreoffice"), "{name}");
                }
                other => panic!("`{name}` resolves to {other:?}"),
            }
            assert_eq!(surface_for(name, &installed).as_deref(), Some("libreoffice"), "{name}");
            // Not installed here, so known-but-missing, which is how `open_app` says so.
            assert_eq!(
                availability(name, &installed),
                Availability::Missing("/opt/yantrik-test-nowhere/libreoffice".into()),
                "{name}"
            );
        }
        // The names it tried to take still open what they opened before.
        assert!(matches!(resolve("files", &installed), Resolved::Route(Launch::Screen(8))));
        assert_eq!(surface_for("notes", &installed).as_deref(), Some("notes"));
        assert_eq!(surface_for("sysmonitor", &installed).as_deref(), Some("system-monitor"));
        assert_eq!(surface_for("writer", &installed).as_deref(), Some("libreoffice"));
        // And the refusal lists it among what can be asked for, once it is on the disk.
        assert!(!launchable_app_ids(&installed).contains(&"libreoffice".to_string()));
    }

    /// A surface installed while the shell runs is found without a restart: the directory's
    /// fingerprint moves, the rescan reads the new entry, and the listing and `open_app` have it.
    #[cfg(unix)]
    #[test]
    fn a_surface_installed_while_the_shell_runs_is_found_on_the_next_scan() {
        use std::os::unix::fs::PermissionsExt;
        let xdg = std::env::temp_dir().join(format!("yantrik-dock-xdg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&xdg);
        let apps = xdg.join("applications");
        std::fs::create_dir_all(&apps).unwrap();
        let program = xdg.join("hello-app");
        std::fs::write(&program, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let dirs = vec![apps.clone()];

        let before = crate::apps::fingerprint(&dirs);
        let scanned: Vec<DesktopEntry> =
            crate::apps::scan_in(&dirs).into_iter().filter(entry_is_launchable).collect();
        assert!(!openable_in(&scanned).iter().any(|a| a["name"] == "hello"));
        assert!(matches!(resolve("hi", &scanned), Resolved::Unknown));

        std::fs::write(
            apps.join("org.example.Hello.desktop"),
            format!(
                "[Desktop Entry]\nType=Application\nName=Hello\nExec={} --window\n\
                 X-Yantrik-Surface=hello\nX-Yantrik-Purpose=say hello to someone, by name\n\
                 X-Yantrik-Aliases=hi;greeter\n",
                program.display()
            ),
        )
        .unwrap();
        assert_ne!(crate::apps::fingerprint(&dirs), before, "the watcher would see this");

        // What `Catalogue::refresh` does: scan, keep what can run.
        let rescanned: Vec<DesktopEntry> =
            crate::apps::scan_in(&dirs).into_iter().filter(entry_is_launchable).collect();
        let listed = openable_in(&rescanned);
        let hello = listed.iter().find(|a| a["name"] == "hello").expect("hello is listed after the rescan");
        assert_eq!(hello["describe_as"], "hello");
        assert_eq!(hello["aliases"], serde_json::json!(["hi", "greeter"]));
        assert_eq!(hello["for"], "say hello to someone, by name");
        assert_eq!(availability("hi", &rescanned), Availability::Ready);
        match resolve("greeter", &rescanned) {
            Resolved::Catalogue { id, bin, args, surface, .. } => {
                assert_eq!(bin, program.display().to_string());
                assert_eq!(args, vec!["--window".to_string()]);
                assert_eq!(surface.as_deref(), Some("hello"));
                // A third-party entry keeps its own id for the window registry.
                assert_eq!(id, "org.example.Hello");
            }
            other => panic!("`greeter` resolves to {other:?}"),
        }
        assert!(launchable_app_ids(&rescanned).contains(&"hello".to_string()));
        assert_eq!(launcher_id_in("hi", &rescanned), "org.example.Hello");
        let _ = std::fs::remove_dir_all(&xdg);
    }

    #[test]
    fn punctuation_does_not_decide_whether_an_app_opens() {
        assert_eq!(canonical_id("download-manager"), "download_manager");
        assert_eq!(canonical_id("System-Monitor"), "system_monitor");
        assert_eq!(canonical_id("Download Manager"), "download_manager");
        assert_eq!(canonical_id("  notes  "), "notes");
        // Already canonical, and unchanged.
        assert_eq!(canonical_id("terminal"), "terminal");
    }

    #[test]
    fn the_guard_accepts_everything_the_dispatch_handles() {
        // These all have arms in `wire()` and were all refused by `is_known_app` as unknown,
        // which is the failure mode this list's own comment claimed to prevent.
        let shipped = shipped();
        for id in [
            "containers", "downloads", "snippets", "documents", "presentation",
            "sysmonitor", "devices", "permissions", "slides", "text_editor", "image_viewer",
        ] {
            assert!(is_known_app(id, &shipped), "the dispatch launches `{id}` but the guard refuses it");
        }
    }

    /// Every tile the launcher shows for the shell's own screens opens something.
    ///
    /// About and Skills were tiles with no route: a click logged "Unknown app", and `open_app`
    /// answered "launching" because the catalogue entry made them look known.
    #[test]
    fn every_builtin_tile_has_somewhere_to_go() {
        for entry in crate::apps::builtin_apps() {
            assert!(
                route(&entry.app_id).is_some(),
                "the launcher shows `{}` ({}), and nothing opens it",
                entry.name,
                entry.app_id
            );
            assert!(entry_is_launchable(&entry));
        }
    }

    /// A shell app whose program is not on the disk is known, and not launchable.
    /// Blender needs three things, and the third is a display. The first two were checked;
    /// the third was discovered by watching `open_app name=blender` answer "launching" and
    /// then nothing, on a session without Xwayland (#96).
    #[test]
    fn blender_without_an_x_display_is_refused_before_it_is_started() {
        assert!(blender_display(Some(":0")).is_ok());
        assert!(blender_display(Some(":1")).is_ok());
        let why = blender_display(None).unwrap_err();
        assert!(why.contains("DISPLAY"), "{why}");
        assert!(why.contains("X11"), "the refusal should say why this Blender needs X: {why}");
        assert!(why.contains("xwayland"), "and what would fix it: {why}");
        // An empty DISPLAY is what a script that copied a compositor's own environment exports.
        // It is not a display.
        assert!(blender_display(Some("")).is_err());
        assert!(blender_display(Some("   ")).is_err());
    }

    // ── What an exit means ──

    /// A window opened and closed again inside the launch grace, on purpose, is not a launch that
    /// never arrived (#200, #168).
    ///
    /// The shapes this was found in: the rc-tier browser check opens Chromium, reads a page and
    /// closes it at ~2.3 s, and `release-check` failed its last check on `{"app": "browser",
    /// "status": "exit status: 0", "lived_ms": 2342}`; and a person closing Studio two seconds
    /// after opening it left the same record, saying to any caller that Studio could not start.
    /// In both the window was gone from the compositor by the time the reaper looked, so nothing
    /// was brought forward and the launch owned its record — all that is left to go on is the
    /// exit, and status 0 is an app that ran and was closed, not one that never came up.
    #[test]
    fn a_clean_close_inside_the_grace_is_not_a_failed_launch() {
        assert_eq!(exit_verdict(2342, true, true, false), ExitVerdict::Exited);
        assert_eq!(exit_verdict(2167, true, true, false), ExitVerdict::Exited);
        assert_eq!(exit_verdict(1, true, true, false), ExitVerdict::Exited);
    }

    /// An app that dies inside the grace without ever showing a window is still a failed launch —
    /// the reason the grace exists. #200: "one that exits 1 in 2 s still is"; #168: "a launch that
    /// exits 1 at 500 ms is". A process killed by a signal has no exit code and is not a success
    /// either, so it takes the same branch.
    #[test]
    fn a_non_clean_exit_inside_the_grace_is_a_failed_launch() {
        assert_eq!(exit_verdict(500, false, true, false), ExitVerdict::Failed);
        assert_eq!(exit_verdict(2000, false, true, false), ExitVerdict::Failed);
        assert_eq!(
            exit_verdict(crate::running::LAUNCH_GRACE_MS - 1, false, true, false),
            ExitVerdict::Failed
        );
    }

    /// Past the grace, an exit is an ordinary exit whatever its status: the app showed a window
    /// and lived, so a later crash is a crash — `record_crash` writes it up — and not a launch
    /// that never was.
    #[test]
    fn an_exit_past_the_grace_is_not_a_failed_launch() {
        assert_eq!(
            exit_verdict(crate::running::LAUNCH_GRACE_MS, false, true, false),
            ExitVerdict::Exited
        );
        assert_eq!(exit_verdict(60_000, true, true, false), ExitVerdict::Exited);
    }

    /// The handover the fix must not disturb: a second copy of a single-instance app exits at once
    /// and cleanly, either with the open window raised from here or without a record of its own to
    /// keep — and a copy that could not exit cleanly is nobody's handover.
    #[test]
    fn a_second_copy_inside_the_grace_still_hands_over() {
        // The window was on screen at the moment of exit and was brought forward.
        assert_eq!(exit_verdict(120, true, true, true), ExitVerdict::SecondCopy);
        // After a shell restart the registry did not know the app was open, so the copy never
        // owned the record; its clean quick exit is the handover anyway.
        assert_eq!(exit_verdict(120, true, false, false), ExitVerdict::SecondCopy);
        // A second copy that crashed is still a failed launch.
        assert_eq!(exit_verdict(120, false, false, false), ExitVerdict::Failed);
    }

    #[test]
    fn a_missing_program_is_known_but_does_not_open() {
        let there = PathBuf::from("/definitely/not/here/yantrik-notes");
        assert!(find_program(there.to_str().unwrap()).is_none());
        assert_eq!(
            program_availability("/definitely/not/here/yantrik-notes --new"),
            Availability::Missing("/definitely/not/here/yantrik-notes".to_string())
        );
        // Screens are compiled into the shell; they are always there.
        assert_eq!(availability("files", &[]), Availability::Ready);
        assert_eq!(availability("about", &[]), Availability::Ready);
        assert_eq!(availability("skills", &[]), Availability::Ready);
    }

    /// A .desktop file whose program is gone is kept out of the launcher, and one whose program
    /// is present is kept in it.
    #[cfg(unix)]
    #[test]
    fn a_desktop_entry_is_listed_only_if_its_program_exists() {
        let entry = |exec: &str| DesktopEntry {
            name: "Thing".into(),
            exec: exec.into(),
            icon: String::new(),
            categories: String::new(),
            comment: String::new(),
            app_id: "thing".into(),
            icon_char: String::new(),
            ..Default::default()
        };
        assert!(entry_is_launchable(&entry("/bin/sh -c true")));
        assert!(entry_is_launchable(&entry("sh")), "a bare name is looked up on PATH");
        assert!(!entry_is_launchable(&entry("/usr/bin/no-such-browser-anywhere %U")));
        assert!(!entry_is_launchable(&entry("no-such-browser-anywhere")));

        let gone = [entry("no-such-browser-anywhere")];
        assert_eq!(
            availability("thing", &gone),
            Availability::Missing("no-such-browser-anywhere".to_string())
        );
        assert!(!is_launchable("thing", &gone));
        assert!(is_known_app("thing", &gone), "known, so the caller hears 'not installed'");
    }

    /// A file that exists but cannot be executed is not a program.
    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_run_is_not_a_program() {
        let dir = std::env::temp_dir().join(format!("yantrik-dock-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("not-executable");
        std::fs::write(&file, "#!/bin/sh
").unwrap();
        assert!(find_program(file.to_str().unwrap()).is_none());
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(find_program(file.to_str().unwrap()).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The browser row is the Browser pin, whichever browser answers it.
    #[test]
    fn browser_is_one_route_with_several_programs() {
        assert_eq!(route("browser"), Some(Launch::Browser));
        assert!(BROWSERS.iter().any(|(name, _)| *name == "chromium"));
        assert!(BROWSERS.iter().any(|(name, _)| name.starts_with("firefox")));
        // Only the Chromium family gets Chromium's flags; Firefox would refuse to start on them.
        for (name, flags) in BROWSERS {
            let chromium = name.contains("chrom");
            assert_eq!(!flags.is_empty(), chromium, "{name}");
            assert!(!flags.iter().any(|f| f.contains("/tmp")), "{name}: a /tmp profile forgets everything");
        }
    }

    /// No name is claimed by two routes: the first would always win, silently.
    #[test]
    fn every_name_routes_to_exactly_one_app() {
        let mut seen = std::collections::HashSet::new();
        for id in builtin_app_ids() {
            assert!(seen.insert(id), "`{id}` appears in two routes");
            assert_eq!(canonical_id(id), id, "`{id}` is not canonical, so it can never match");
        }
    }

    #[test]
    fn an_app_that_does_not_exist_is_still_refused() {
        // The guard must not have become a rubber stamp on the way to being more generous.
        assert!(!is_known_app("nonexistent-app", &[]));
        assert!(!is_known_app("", &[]));
    }

    // ── The shelf ──

    /// A shelved .desktop entry, as an installed machine still has one on disk.
    fn stale_entry(app_id: &str, name: &str, exec: &str) -> DesktopEntry {
        DesktopEntry {
            name: name.into(),
            exec: exec.into(),
            icon: String::new(),
            categories: String::new(),
            comment: String::new(),
            app_id: app_id.into(),
            icon_char: String::new(),
            ..Default::default()
        }
    }

    /// A .desktop entry the distribution ships must not shadow the shell's own route.
    ///
    /// The one that did: Debian's `blender.desktop`, `Exec=blender %f`, matched "blender" before
    /// `Launch::Blender` and opened a Blender with no addon and no surface (#96). The same shape
    /// waits for any distro app whose Name collides with one of ours — a file manager called
    /// "Files" would have opened instead of the shell's Files screen.
    #[test]
    fn a_distro_desktop_entry_does_not_shadow_the_shells_own_route() {
        let debian_blender = stale_entry("blender", "Blender", "blender %f");
        assert!(
            matches!(resolve("blender", &[debian_blender]), Resolved::Route(Launch::Blender)),
            "Debian's blender.desktop shadowed the route that carries the addon"
        );
        let a_file_manager = stale_entry("org.gnome.Nautilus", "Files", "nautilus --new-window");
        assert!(
            matches!(resolve("files", &[a_file_manager]), Resolved::Route(Launch::Screen(8))),
            "a distro file manager shadowed the shell's own Files screen"
        );
        // A program the shell has no route for still opens from its entry, args and all.
        let foreign = stale_entry("foo", "Foo", "foo --bar baz");
        match resolve("foo", &[foreign]) {
            Resolved::Catalogue { bin, args, .. } => {
                assert_eq!(bin, "foo");
                assert_eq!(args, vec!["--bar".to_string(), "baz".to_string()]);
            }
            _ => panic!("a catalogue-only program must resolve to its .desktop entry"),
        }
        // And the shelf still comes first, whatever else is installed.
        let shelved_by_name = stale_entry("music-player", "Music Player", "yantrik-music-player");
        assert!(matches!(resolve("music", &[shelved_by_name]), Resolved::Shelved(_)));
        assert!(matches!(resolve("nothing-called-this", &[]), Resolved::Unknown));
    }

    /// Every spelling a caller could arrive with is refused, and refused for the same reason.
    ///
    /// A shelf that only catches one spelling is not a shelf: `music-player` is what the binary
    /// is called, `Music Player` is the window title, `music` is what the dock says, and an agent
    /// reads whichever of those it saw last.
    #[test]
    fn every_spelling_of_a_shelved_app_reaches_the_shelf() {
        for spelling in [
            "music", "music_player", "music-player", "Music Player", "MUSIC",
            "  music  ", "yantrik-music-player",
        ] {
            let shelf = shelved(spelling).unwrap_or_else(|| panic!("`{spelling}` is not shelved"));
            assert_eq!(shelf.binary, "yantrik-music-player", "{spelling}");
        }
        for spelling in ["spreadsheet", "Spreadsheet", "ySheets", "ysheets", "yantrik-spreadsheet"] {
            let shelf = shelved(spelling).unwrap_or_else(|| panic!("`{spelling}` is not shelved"));
            assert_eq!(shelf.binary, "yantrik-spreadsheet", "{spelling}");
        }
    }

    /// Opening a shelved app is refused, and never answered "launching".
    ///
    /// Checked against a catalogue that still holds the app, because that is the state of every
    /// machine updated from a release that had it: the binary and the .desktop file are both
    /// still on the disk, and the updater removes neither.
    #[test]
    fn a_shelved_app_does_not_open_even_with_its_desktop_file_on_disk() {
        let stale = [
            stale_entry("yantrik-music-player", "Music", "/opt/yantrik/bin/yantrik-music-player"),
            stale_entry("yantrik-spreadsheet", "ySheets", "/opt/yantrik/bin/yantrik-spreadsheet"),
        ];
        for name in ["music", "music-player", "Music", "spreadsheet", "ySheets"] {
            assert!(!is_launchable(name, &stale), "`{name}` must not open");
            assert!(
                matches!(availability(name, &stale), Availability::Shelved(_)),
                "`{name}` must be refused as shelved, not as unknown or missing"
            );
        }
    }

    /// The catalogue filter drops a shelved entry, whichever way it is named.
    ///
    /// Matched on the program the Exec line runs as well as on the id, because a .desktop file
    /// left on disk by an earlier release is the case this has to survive and nothing says its
    /// basename will still be one the shelf recognises.
    #[test]
    fn the_catalogue_drops_a_shelved_desktop_entry() {
        assert!(!entry_is_launchable(&stale_entry(
            "yantrik-music-player", "Music", "/opt/yantrik/bin/yantrik-music-player"
        )));
        assert!(!entry_is_launchable(&stale_entry(
            "yantrik-spreadsheet", "ySheets", "/opt/yantrik/bin/yantrik-spreadsheet"
        )));
        // Renamed by hand, or installed somewhere else: the program is what gives it away.
        assert!(!entry_is_launchable(&stale_entry(
            "sheets-old", "Sheets", "/usr/local/bin/yantrik-spreadsheet %f"
        )));
        // And an app that is not shelved is untouched by any of it.
        assert!(entry_is_launchable(&stale_entry("shell", "Shell", "/bin/sh")));
    }

    /// No route points at a shelved binary, and no shelved name is offered as something to open.
    #[test]
    fn nothing_routes_to_a_shelved_app() {
        for (names, _) in ROUTES {
            for name in *names {
                assert!(shelved(name).is_none(), "`{name}` is both routed and shelved");
            }
        }
        // No shipped entry that declares a surface runs a shelved binary.
        for surface in crate::surfaces::declared(&shipped()) {
            assert!(shelved(&surface.id).is_none(), "`{}` is declared and shelved", surface.id);
        }
        // The list a refusal hands back must not name something that would itself be refused.
        let offered = launchable_app_ids(&shipped());
        for id in &offered {
            assert!(shelved(id).is_none(), "`{id}` is offered as launchable and is shelved");
        }
    }

    /// Every shelf entry says why, and says what would bring it back.
    ///
    /// Both strings are quoted straight into the refusal a person or a mind reads, so an empty
    /// one is a refusal that explains nothing.
    #[test]
    fn a_shelf_entry_argues_for_itself() {
        for shelf in SHELVED {
            assert!(!shelf.reason.trim().is_empty(), "{} has no reason", shelf.binary);
            assert!(
                !shelf.returns_when.trim().is_empty(),
                "{} does not say what would bring it back",
                shelf.binary
            );
            assert!(!shelf.ids.is_empty(), "{} answers to no name", shelf.binary);
            for id in shelf.ids {
                assert_eq!(canonical_id(id), *id, "`{id}` is not canonical, so it can never match");
            }
        }
    }

    /// The apps that ship are untouched by the shelf.
    #[test]
    fn un_shelved_apps_are_unaffected() {
        let shipped = shipped();
        for id in ["notes", "terminal", "files", "calendar", "email", "documents", "presentation"] {
            assert!(shelved(id).is_none(), "`{id}` is not shelved");
            assert!(!matches!(resolve(id, &shipped), Resolved::Unknown), "`{id}` still opens");
            assert!(is_known_app(id, &shipped), "`{id}` is still known");
        }
    }

    /// The release is packaged from one list of exclusions, and it is this one.
    ///
    /// build-release.sh discovers the binaries to ship by looking at what the build produced, so
    /// a shelved crate that is still a workspace member would be packaged simply because it
    /// compiled. The script therefore carries the same two binary names, and a shelf that grows
    /// an entry the script does not know about would ship the app it just refused to open.
    #[test]
    fn what_can_be_opened_is_listed_by_the_name_that_opens_it() {
        let shipped = shipped();
        let apps = openable_with(&shipped, &|_| false);
        let names: Vec<&str> = apps.iter().map(|a| a["name"].as_str().unwrap()).collect();
        for name in &names {
            assert!(
                !matches!(resolve(name, &shipped), Resolved::Unknown),
                "`{name}` is listed as openable and open_app would not open it"
            );
        }
        for shelf in SHELVED {
            for id in shelf.ids {
                assert!(!names.contains(id), "`{id}` is shelved and is listed as openable");
            }
        }
        // Listed by the name that describes it, with every other name it answers to beside it —
        // the launcher's `sysmonitor` among them — and whether it is open.
        let monitor = apps.iter().find(|a| a["name"] == "system-monitor").expect("system-monitor is openable");
        assert_eq!(monitor["describe_as"], "system-monitor");
        assert_eq!(monitor["aliases"], serde_json::json!(["sysmonitor"]));
        assert_eq!(monitor["title"], "System Monitor");
        assert_eq!(monitor["running"], false);
        // Every app this OS ships is listed while it is closed.
        for (dir, id) in published_ids() {
            assert!(names.contains(&id.as_str()), "apps/{dir} publishes `{id}` and is not listed while closed");
        }
        let notes = apps.iter().find(|a| a["name"] == "notes").expect("notes is openable");
        assert_eq!(notes["describe_as"], "notes");
        assert!(apps.iter().filter(|a| a["opens"] == "app").all(|a| a["describe_as"].is_string()));
        // Every program says what it is for: a list of names is how three models wrote a
        // "document" into Notes.
        for app in apps.iter().filter(|a| a["opens"] == "app") {
            assert!(app["for"].as_str().is_some_and(|s| s.len() > 8), "{} does not say what it is for", app["name"]);
        }
    }

    /// The launcher is listed as the launcher, not as a screen.
    ///
    /// `yos ls` put `launchpad` under "Screens of the desktop itself", because the listing
    /// called everything that was not a program a screen. A caller took it at its word: `open_app
    /// name=launchpad` opened the grid under the app window in front, and the natural next try,
    /// `show_screen screen=launchpad`, was refused with "no screen called launchpad". The listing
    /// is the one place a caller can read what a name opens, so it says that this one opens an
    /// overlay on the desktop, and what is in it.
    #[test]
    fn a_settings_section_is_listed_as_a_section_with_the_call_that_shows_it() {
        // release-check's first run on VM 520 asked `show_screen screen=skills`, as the listing
        // said to, and was refused: Skills is a section of Settings, not a screen.
        let apps = openable();
        let skills = apps.iter().find(|a| a["name"] == "skills").expect("skills is openable");
        assert_eq!(skills["opens"], "a section of Settings", "listed as {skills}");
        assert_eq!(skills["show_with"]["screen"], "settings");
        assert_eq!(skills["show_with"]["section"], "skills");
        for a in &apps {
            if a["opens"] == "a screen of the desktop itself" {
                assert!(
                    !matches!(route(a["name"].as_str().unwrap_or_default()), Some(Launch::SettingsSection(_))),
                    "{a} is a settings section listed as a screen"
                );
            }
        }
    }

    #[test]
    fn the_launcher_is_listed_as_what_it_is() {
        assert_eq!(route("launchpad"), Some(Launch::Launchpad));
        assert_eq!(availability("launchpad", &[]), Availability::Ready);
        // Part of the desktop's own surface, which is where `describe` reports it open.
        assert_eq!(surface_for("launchpad", &[]).as_deref(), Some("shell"));

        let apps = openable();
        let launcher = apps.iter().find(|a| a["name"] == "launchpad").expect("launchpad is openable");
        let opens = launcher["opens"].as_str().unwrap_or_default();
        assert!(
            opens.contains("launcher"),
            "`launchpad` is listed as opening `{opens}`, and a caller reading that will ask \
             show_screen for it"
        );
        assert!(!opens.starts_with("a screen"), "the launcher is not a screen: {opens}");
        assert_eq!(launcher["describe_as"], "shell");
        assert!(
            launcher["for"].as_str().is_some_and(|s| s.contains("app")),
            "the listing does not say what the launcher is for: {launcher}"
        );
    }

    #[test]
    fn the_release_script_excludes_every_shelved_binary() {
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../deploy/yantrik-os/build-release.sh");
        let Ok(text) = std::fs::read_to_string(&script) else {
            return; // Packaged source without the deploy tree; nothing to check against.
        };
        // The release script used to carry its own copy of the shelf and this test read the
        // names out of it. Five scripts carried such a copy and four were wrong, so the list
        // is no longer written down anywhere but here: `shelved-bins.sh` reads the table above
        // and every packaging script asks it. What can still go wrong is a script that stops
        // asking, or a reader that stops reading this file — so that is what is checked.
        assert!(
            text.contains("shelved-bins.sh"),
            "{} no longer asks shelved-bins.sh what is shelved, so a release would ship it all",
            script.display()
        );
        let reader = script.with_file_name("shelved-bins.sh");
        let reader_text = std::fs::read_to_string(&reader)
            .unwrap_or_else(|e| panic!("{} is missing: {e}", reader.display()));
        assert!(
            reader_text.contains("crates/yantrik-ui/src/wire/dock.rs"),
            "{} does not read the SHELVED table in dock.rs",
            reader.display()
        );
        // Its sed pattern takes `binary: "…"` lines that start with whitespace. Hold the table
        // to that shape, or an entry reformatted onto one line would silently leave the shelf.
        let this_file = include_str!("dock.rs");
        for shelf in SHELVED {
            let as_the_script_sees_it = this_file.lines().any(|line| {
                line.starts_with(char::is_whitespace)
                    && line.trim_start().starts_with(&format!("binary: \"{}\"", shelf.binary))
            });
            assert!(
                as_the_script_sees_it,
                "{} is shelved but not written the way shelved-bins.sh reads it",
                shelf.binary
            );
        }
    }

    /// The .desktop entries reach every machine the binaries do, and by one rule.
    ///
    /// Our apps are found by their .desktop files (`crate::surfaces`), so a deploy that ships the
    /// binaries without them leaves a machine whose apps cannot be listed while closed, answer to
    /// no alias, and — Arcade on VM 520 — cannot be opened at all. deploy-to-vm.sh shipped exactly
    /// that. Both it and the release ask shipped-desktop-files.sh which entries ship, and that
    /// asks shelved-bins.sh what is shelved, so neither can drift into its own list.
    #[test]
    fn every_packaging_path_ships_the_desktop_entries_by_the_shelfs_rule() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/yantrik-os");
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok();
        let Some(rule) = read("shipped-desktop-files.sh") else {
            return; // Packaged source without the deploy tree; nothing to check against.
        };
        assert!(rule.contains("shelved-bins.sh"), "shipped-desktop-files.sh no longer asks the shelf");
        for script in ["build-release.sh", "deploy-to-vm.sh"] {
            let text = read(script).unwrap_or_else(|| panic!("deploy/yantrik-os/{script} is missing"));
            assert!(
                text.contains("shipped-desktop-files.sh"),
                "deploy/yantrik-os/{script} does not ship the .desktop entries through \
                 shipped-desktop-files.sh, so a machine it installs finds none of our apps by name"
            );
        }
    }
}
