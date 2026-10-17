//! Surfaces a mind can find while they are closed — read from the apps' own `.desktop` files.
//!
//! A running surface is found by its socket. A closed one used to be known only through tables
//! written into this shell: the launch routes, the purposes, the apps' alias table. So an app
//! somebody else wrote could not be listed while closed, could not declare another name, could not
//! be opened by `open_app` under its surface id, and a button on its notification reached nothing.
//! Our own apps could do all of that only because their names were typed into the shell.
//!
//! Now every app declares itself where every Debian app already describes itself — its `.desktop`
//! file — with four keys (`yantrik_shell_core::apps`, design/surface-sdk-2026-09-23.md §4):
//!
//! ```text
//! X-Yantrik-Surface=libreoffice
//! X-Yantrik-Purpose=Documents, spreadsheets and slides: open, read, edit and export them
//! X-Yantrik-Aliases=writer;calc;impress
//! X-Yantrik-Adapter=/usr/lib/yantrik/adapters/libreoffice
//! ```
//!
//! and this module is what the shell does with them:
//!
//! - [`declared`] is the catalogue of surfaces, one per id, with the names each may use: a name
//!   the desktop itself answers to (`shell`, a screen, a settings section) or that another surface
//!   already holds is not handed out twice.
//! - [`find`] resolves any name a caller holds — the id, an alias, the app's Name, its entry id —
//!   to the surface and the entry that launches it. `wire::dock` asks it for `open_app`,
//!   notification buttons and approvals.
//! - [`link_aliases_in`] links every alias at its surface's socket (`app-<alias>.sock` →
//!   `app-<id>.sock`), so `yos describe <alias>` works for an app that never heard of aliases —
//!   a Python surface, Blender's addon, anything.
//! - [`watch`] rescans when an application directory changes and relinks.
//! - [`start_adapter`] / [`app_exited`] run an `X-Yantrik-Adapter` beside the app it serves.
//!
//! This OS's own apps ship `.desktop` files with the same keys and get exactly this, nothing more.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::apps::{fold_name, Catalogue, DesktopEntry};

/// One surface this desktop knows from a `.desktop` file, whether or not it is running.
#[derive(Debug, Clone, PartialEq)]
pub struct Declared {
    /// The id it publishes as `app` and binds as `app-<id>.sock`.
    pub id: String,
    /// What a person calls it: the `Name=` of the entry that declared it first.
    pub title: String,
    /// `X-Yantrik-Purpose`, from the first entry that says one.
    pub purpose: String,
    /// Every other name it answers to that nothing else holds, across all its entries.
    pub aliases: Vec<String>,
    /// The entry that launches it when it is asked for by its id: the first to declare it.
    pub entry: String,
}

/// The names that are the desktop's own and that no `.desktop` file can take.
///
/// `shell` is the desktop's surface; `yantrik` is the name the shell sends its own notifications
/// under. A screen, a section of Settings, the launcher and the browser are routes of the shell
/// (`wire::dock::route`), and a declared surface cannot be reached under a name that opens one of
/// those instead — so the name is refused at the door rather than listed and then shadowed.
fn held_by_the_desktop(name: &str, for_surface: &str) -> bool {
    if name == "shell" || name == "yantrik" {
        return true;
    }
    match crate::wire::dock::route(name) {
        None => false,
        // A route whose surface IS this one — Blender, whose route runs the addon that serves it
        // — is the same app under the same name, not a collision.
        Some(launch) => crate::wire::dock::route_surface(launch) != Some(for_surface),
    }
}

/// Every surface the installed apps declare, one per id.
///
/// Ids are settled before aliases, so an alias can never shadow another app's id whatever order
/// the files were scanned in. Between two entries, the first in the catalogue's order wins, and
/// the loser is told so in the log with the name it lost — an author who finds their alias
/// missing from `describe shell` can read why.
pub fn declared(installed: &[DesktopEntry]) -> Vec<Declared> {
    let mut out: Vec<Declared> = Vec::new();
    for entry in installed {
        let Some(id) = entry.surface.as_deref() else { continue };
        if !counts(entry) {
            continue;
        }
        if held_by_the_desktop(id, id) {
            tracing::warn!(
                entry = %entry.app_id, surface = id,
                "a .desktop file declares a surface under a name the desktop itself answers to; \
                 it is listed as an app without a surface"
            );
            continue;
        }
        match out.iter_mut().find(|d| d.id == id) {
            // A second entry for the same surface — a suite whose programs share one adapter.
            // It lends its purpose if the first had none; its aliases are added below.
            Some(existing) => {
                if existing.purpose.is_empty() {
                    existing.purpose = entry.purpose.clone();
                }
            }
            None => out.push(Declared {
                id: id.to_string(),
                title: entry.name.clone(),
                purpose: entry.purpose.clone(),
                aliases: Vec::new(),
                entry: entry.app_id.clone(),
            }),
        }
    }
    for entry in installed {
        let Some(id) = entry.surface.as_deref() else { continue };
        if !out.iter().any(|d| d.id == id) || !counts(entry) {
            continue;
        }
        for alias in &entry.aliases {
            let taken_elsewhere = out.iter().any(|d| {
                d.id != id && (d.id == *alias || d.aliases.iter().any(|a| a == alias))
            });
            if taken_elsewhere || held_by_the_desktop(alias, id) {
                tracing::warn!(
                    entry = %entry.app_id, surface = id, alias = %alias,
                    "an alias is already a name of the desktop or of another surface; left out"
                );
                continue;
            }
            if let Some(d) = out.iter_mut().find(|d| d.id == id) {
                if !d.aliases.contains(alias) {
                    d.aliases.push(alias.clone());
                }
            }
        }
    }
    out
}

/// The `.desktop` files this OS ships, compiled in: `(entry id, contents)`.
///
/// The machine's own copies are what the shell reads; these are for the two places that must
/// know our apps' names whatever is installed. A role's reach is written in names (`text-editor`)
/// and held against the id an app publishes (`editor`), and a reach has to mean on every machine
/// what it meant when it was written — so our apps' names come from the build, not from whichever
/// entry won a name on this disk. And Blender's listing row says what it is for even where its
/// entry is not installed. `the_compiled_in_entries_are_the_shipped_files` holds this list to the
/// directory, the way the agent catalog holds `config/agents`.
pub const SHIPPED_ENTRIES: [(&str, &str); 19] = [
    ("yantrik-arcade", include_str!("../../../apps/desktop-files/yantrik-arcade.desktop")),
    ("yantrik-blender", include_str!("../../../apps/desktop-files/yantrik-blender.desktop")),
    ("yantrik-calendar", include_str!("../../../apps/desktop-files/yantrik-calendar.desktop")),
    ("yantrik-container-manager", include_str!("../../../apps/desktop-files/yantrik-container-manager.desktop")),
    ("yantrik-document-editor", include_str!("../../../apps/desktop-files/yantrik-document-editor.desktop")),
    ("yantrik-download-manager", include_str!("../../../apps/desktop-files/yantrik-download-manager.desktop")),
    ("yantrik-email", include_str!("../../../apps/desktop-files/yantrik-email.desktop")),
    ("yantrik-image-viewer", include_str!("../../../apps/desktop-files/yantrik-image-viewer.desktop")),
    ("yantrik-music-player", include_str!("../../../apps/desktop-files/yantrik-music-player.desktop")),
    ("yantrik-network-manager", include_str!("../../../apps/desktop-files/yantrik-network-manager.desktop")),
    ("yantrik-notes", include_str!("../../../apps/desktop-files/yantrik-notes.desktop")),
    ("yantrik-presentation", include_str!("../../../apps/desktop-files/yantrik-presentation.desktop")),
    ("yantrik-snippet-manager", include_str!("../../../apps/desktop-files/yantrik-snippet-manager.desktop")),
    ("yantrik-spreadsheet", include_str!("../../../apps/desktop-files/yantrik-spreadsheet.desktop")),
    ("yantrik-studio", include_str!("../../../apps/desktop-files/yantrik-studio.desktop")),
    ("yantrik-system-monitor", include_str!("../../../apps/desktop-files/yantrik-system-monitor.desktop")),
    ("yantrik-terminal", include_str!("../../../apps/desktop-files/yantrik-terminal.desktop")),
    ("yantrik-text-editor", include_str!("../../../apps/desktop-files/yantrik-text-editor.desktop")),
    ("yantrik-weather", include_str!("../../../apps/desktop-files/yantrik-weather.desktop")),
];

/// The shipped entries, parsed. The shelf still applies: [`declared`] leaves a shelved app out.
pub fn shipped() -> Vec<DesktopEntry> {
    SHIPPED_ENTRIES
        .iter()
        .filter_map(|(stem, text)| crate::apps::parse_desktop_text(stem, text))
        .collect()
}

/// The id a declared surface publishes, given any name it answers to — this OS's own apps by the
/// names their shipped files give them, then anything installed here.
///
/// For a role's reach (`agents::catalog`), which used to fold names through the apps' table in the
/// runtime. Ours first and from the build, so a third-party entry that won an alias like
/// `text-editor` on this disk cannot turn a reach written for our editor into a reach over its app.
pub fn surface_id(name: &str) -> Option<String> {
    if let Some((surface, _)) = find(name, &shipped()) {
        return Some(surface.id);
    }
    let installed = Catalogue::shared().get();
    find(name, &installed).map(|(surface, _)| surface.id)
}

/// Whether an entry's declaration counts at all: an app this build has shelved declares nothing,
/// whatever an old `.desktop` file left on the disk says.
fn counts(entry: &DesktopEntry) -> bool {
    crate::wire::dock::shelved(&entry.app_id).is_none()
        && crate::wire::dock::shelved_exec(&entry.exec).is_none()
}

/// The surface a name refers to, and the entry that opens it — for any name a caller could hold.
///
/// Asked in three rounds so a stronger match always beats a weaker one: the id, then an alias,
/// then what the app is called elsewhere — its `Name=` (`System Monitor`, `yDoc`), its entry id
/// (`yantrik-notes`), and the id the shell registers its window under (`sysmonitor`). Only the
/// first two are names on the socket bus; the third round is what lets `open_app name=yDoc` and a
/// notification sent as "Downloads" reach the app everyone else calls by those words.
///
/// An alias opens the entry that declared it, so a suite whose programs share one surface opens
/// Calc for `calc` and Writer for `writer`.
pub fn find<'a>(name: &str, installed: &'a [DesktopEntry]) -> Option<(Declared, &'a DesktopEntry)> {
    let key = fold_name(name);
    if key.is_empty() {
        return None;
    }
    let surfaces = declared(installed);
    let entry_named = |stem: &str| installed.iter().find(|e| e.app_id == stem);
    let by_id = surfaces.iter().find(|d| d.id == key);
    if let Some(d) = by_id {
        return entry_named(&d.entry).map(|e| (d.clone(), e));
    }
    if let Some(d) = surfaces.iter().find(|d| d.aliases.contains(&key)) {
        let entry = installed
            .iter()
            .find(|e| e.surface.as_deref() == Some(d.id.as_str()) && e.aliases.contains(&key))
            .or_else(|| entry_named(&d.entry));
        return entry.map(|e| (d.clone(), e));
    }
    installed
        .iter()
        .filter(|e| e.surface.is_some())
        .find(|e| {
            fold_name(&e.name) == key
                || fold_name(&e.app_id) == key
                || fold_name(&crate::wire::dock::window_id(&e.app_id)) == key
        })
        .and_then(|e| {
            let d = surfaces.iter().find(|d| Some(d.id.as_str()) == e.surface.as_deref())?;
            Some((d.clone(), e))
        })
}

// ── Aliases on the socket bus ───────────────────────────────────────

/// Link every declared alias at its surface's socket inside `dir`, and report what now reaches
/// what, as `alias → id` pairs.
///
/// A symlink, not a second listener, for the reason the protocol gives: it is one surface, so
/// every name lands in the same process and reads the same revision. Made whether or not the app
/// is running — a link to a socket that is not there yet is invisible to a client (`connect` gets
/// ENOENT and it moves on to its next candidate) and starts working the moment the app binds.
///
/// Nothing here removes a live surface: a socket file under an alias's name that answers a
/// connection belongs to a running process, and is left alone with a warning. A dead one, a
/// regular file, or a link pointing somewhere else is replaced. Relative targets, as the apps'
/// own links always were.
#[cfg(unix)]
pub fn link_aliases_in(dir: &Path, surfaces: &[Declared]) -> Vec<(String, String)> {
    let mut linked = Vec::new();
    for surface in surfaces {
        let target = format!("app-{}.sock", surface.id);
        for alias in &surface.aliases {
            let link = dir.join(format!("app-{alias}.sock"));
            match std::fs::symlink_metadata(&link) {
                Err(_) => {}
                Ok(meta) if meta.file_type().is_symlink() => {
                    if std::fs::read_link(&link).is_ok_and(|to| to == Path::new(&target)) {
                        linked.push((alias.clone(), surface.id.clone()));
                        continue;
                    }
                    let _ = std::fs::remove_file(&link);
                }
                Ok(_) => {
                    if std::os::unix::net::UnixStream::connect(&link).is_ok() {
                        tracing::warn!(
                            alias = %alias, surface = %surface.id, path = %link.display(),
                            "a running process answers under this alias's name; not linked"
                        );
                        continue;
                    }
                    let _ = std::fs::remove_file(&link);
                }
            }
            match std::os::unix::fs::symlink(&target, &link) {
                Ok(()) => linked.push((alias.clone(), surface.id.clone())),
                Err(e) => tracing::warn!(
                    alias = %alias, surface = %surface.id, error = %e,
                    "could not link an alias; callers holding it cannot reach the surface"
                ),
            }
        }
    }
    linked
}

#[cfg(not(unix))]
pub fn link_aliases_in(_dir: &Path, _surfaces: &[Declared]) -> Vec<(String, String)> {
    Vec::new()
}

/// Link the installed apps' aliases in this session's socket directory.
pub fn link_aliases(installed: &[DesktopEntry]) -> usize {
    let dir = yantrik_ipc_transport::server::socket_dir();
    let linked = link_aliases_in(&dir, &declared(installed));
    tracing::debug!(count = linked.len(), dir = %dir.display(), "surface aliases linked");
    linked.len()
}

/// How often the application directories are looked at for a change.
const RESCAN_EVERY: Duration = Duration::from_secs(3);

/// Keep the catalogue and the aliases current while the shell runs.
///
/// The catalogue was scanned at startup and again whenever the launcher opened, so an app
/// installed while the shell ran existed for the launcher and nobody else — not for `open_app`, not
/// in `describe shell`, not under its aliases — until a person happened to open the grid. This
/// looks at the directories every few seconds (`apps::fingerprint`: one `stat` per entry) and
/// rescans only when something in them moved.
///
/// A thread rather than a Slint timer: the rescan reads the disk, and the UI thread has nothing to
/// gain from doing that itself. Every reader takes the catalogue through `Catalogue::get`, which
/// swaps whole lists, so nothing sees half a scan.
pub fn watch(catalogue: Catalogue) {
    let spawned = std::thread::Builder::new().name("surface-catalogue".into()).spawn(move || {
        let dirs = crate::apps::app_dirs();
        let mut last: Option<u64> = None;
        loop {
            let now = crate::apps::fingerprint(&dirs);
            if last != Some(now) {
                if last.is_some() {
                    let count = catalogue.refresh();
                    tracing::info!(apps = count, "application directories changed; rescanned");
                }
                link_aliases(&catalogue.get());
                last = Some(now);
            }
            std::thread::sleep(RESCAN_EVERY);
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not watch the application directories; new apps appear when the launcher opens");
    }
}

// ── Adapters ────────────────────────────────────────────────────────

/// An adapter the shell started, and the launch it belongs to.
///
/// The child itself is kept here, unreaped until its watcher sees it exit under this map's lock.
/// That is what makes stopping it safe: a pid that has not been reaped cannot be reused, so the
/// signal in [`app_exited`] can only ever reach the adapter.
struct Running {
    child: std::process::Child,
    app_pid: u32,
}

/// How often an adapter's watcher checks whether it has exited.
const ADAPTER_POLL: Duration = Duration::from_millis(500);

fn adapters() -> &'static Mutex<HashMap<String, Running>> {
    static ADAPTERS: OnceLock<Mutex<HashMap<String, Running>>> = OnceLock::new();
    ADAPTERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Start the program that provides `surface` for an app that cannot host one, beside the app
/// process `app_pid` the shell has just launched.
///
/// Not started twice: if an adapter this shell started for the surface is still running, or
/// anything already answers on `app-<surface>.sock`, the surface is there and a second provider
/// would fight it for the name.
///
/// The adapter is told what it is for in its environment — `YANTRIK_SURFACE` (the id to bind as
/// `app-<id>.sock`) and `YANTRIK_APP_PID` (the process it serves) — and gets the same display
/// environment the app got. It is stopped with SIGTERM when that app process exits
/// ([`app_exited`]); an adapter should also exit on its own when the app it drives goes away,
/// because an app can be closed from somewhere the shell does not see.
pub fn start_adapter(surface: &str, command: &str, app_pid: u32) {
    let mut words = command.split_whitespace();
    let Some(program) = words.next() else { return };
    let args: Vec<&str> = words.collect();
    {
        let running = adapters().lock().unwrap_or_else(|p| p.into_inner());
        if running.contains_key(surface) {
            tracing::debug!(surface, "adapter already running");
            return;
        }
    }
    if yantrik_app_runtime::service::is_up(&format!("app-{surface}")) {
        tracing::debug!(surface, "the surface already answers; no adapter started");
        return;
    }
    let Some(path) = crate::wire::dock::find_program(program) else {
        tracing::error!(
            surface, adapter = program,
            "the app declares an adapter that is not on this machine; it opens, and nothing answers \
             for it on the socket bus"
        );
        return;
    };
    let mut command = std::process::Command::new(&path);
    command
        .args(&args)
        .env("YANTRIK_SURFACE", surface)
        .env("YANTRIK_APP_PID", app_pid.to_string())
        .env_remove("SLINT_FULLSCREEN")
        .stdin(std::process::Stdio::null());
    for (key, value) in crate::wire::dock::session_env() {
        command.env(key, value);
    }
    let child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            tracing::error!(surface, adapter = %path.display(), error = %e, "could not start the adapter");
            return;
        }
    };
    let adapter_pid = child.id();
    tracing::info!(surface, adapter = %path.display(), adapter_pid, app_pid, "adapter started");
    adapters()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(surface.to_string(), Running { child, app_pid });
    let surface = surface.to_string();
    let _ = std::thread::Builder::new().name(format!("adapter-{surface}")).spawn(move || loop {
        std::thread::sleep(ADAPTER_POLL);
        let mut running = adapters().lock().unwrap_or_else(|p| p.into_inner());
        let Some(run) = running.get_mut(&surface) else { return };
        if run.child.id() != adapter_pid {
            return;
        }
        match run.child.try_wait() {
            Ok(None) => {}
            Ok(Some(status)) => {
                running.remove(&surface);
                tracing::info!(surface = %surface, adapter_pid, %status, "adapter exited");
                return;
            }
            Err(e) => {
                running.remove(&surface);
                tracing::warn!(surface = %surface, adapter_pid, error = %e, "lost track of the adapter");
                return;
            }
        }
    });
}

/// The app process `app_pid` has exited: stop the adapters that were started for it.
///
/// Not when the exit was a hand-over — a second copy of an app passing its request to the window
/// already open and leaving at once. The window it handed over to is still there and the adapter
/// is serving it.
pub fn app_exited(app_pid: u32, handed_over: bool) {
    if handed_over {
        return;
    }
    let running = adapters().lock().unwrap_or_else(|p| p.into_inner());
    for (surface, run) in running.iter().filter(|(_, r)| r.app_pid == app_pid) {
        let adapter_pid = run.child.id();
        tracing::info!(surface = %surface, adapter_pid, app_pid, "stopping the adapter with its app");
        // SIGTERM, so it can close its socket; its watcher reaps it. The child has not been
        // reaped (that only happens under this lock), so the pid is still the adapter's.
        #[cfg(unix)]
        // SAFETY: kill(2) on a child of this process that has not been waited for.
        unsafe {
            libc::kill(adapter_pid as libc::pid_t, libc::SIGTERM);
        }
    }
}

/// Which adapters are running, as `(surface, app pid)`.
#[cfg(test)]
fn running_adapters() -> Vec<(String, u32)> {
    adapters()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .map(|(s, r)| (s.clone(), r.app_pid))
        .collect()
}

/// The catalogue as a machine that installed this build has it: the shell's own screens and every
/// `.desktop` file this OS ships, minus the shelf. Read from `apps/desktop-files`, so the tests
/// that use it hold the shipped files, not a copy of them.
#[cfg(test)]
pub(crate) fn shipped_catalogue() -> Vec<DesktopEntry> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop-files");
    let mut entries = crate::apps::scan_in(&[dir]);
    entries.retain(|e| {
        crate::wire::dock::shelved(&e.app_id).is_none()
            && crate::wire::dock::shelved_exec(&e.exec).is_none()
    });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("yantrik-surfaces-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn entry(stem: &str, name: &str, surface: &str, aliases: &str) -> DesktopEntry {
        crate::apps::parse_desktop_text(
            stem,
            &format!(
                "[Desktop Entry]\nType=Application\nName={name}\nExec=/usr/bin/{stem}\n\
                 X-Yantrik-Surface={surface}\nX-Yantrik-Aliases={aliases}\n"
            ),
        )
        .unwrap()
    }

    /// Every app this OS ships declares its surface, what it is for, and — where it is called
    /// something else anywhere — the other names, and none of them collide.
    #[test]
    fn every_shipped_app_declares_its_surface() {
        let shipped = shipped_catalogue();
        let declared = declared(&shipped);
        let ids: Vec<&str> = declared.iter().map(|d| d.id.as_str()).collect();
        for want in [
            "arcade", "blender", "calendar", "containers", "documents", "download-manager",
            "editor", "email", "image-viewer", "music", "network", "notes", "presentation", "snippets",
            "spreadsheet", "studio", "system-monitor", "terminal", "weather",
        ] {
            assert!(ids.contains(&want), "no shipped .desktop file declares `{want}`: {ids:?}");
        }
        for entry in shipped.iter().filter(|e| e.exec != "__builtin__") {
            assert!(
                entry.surface.is_some(),
                "apps/desktop-files/{}.desktop ships without X-Yantrik-Surface, so a mind cannot \
                 find it while it is closed",
                entry.app_id
            );
        }
        for d in &declared {
            assert!(d.purpose.chars().count() > 8, "`{}` does not say what it is for", d.id);
        }
        // The spellings the old tables carried, each still a name on the socket bus.
        let names = |id: &str| declared.iter().find(|d| d.id == id).unwrap().aliases.clone();
        assert_eq!(names("containers"), ["container-manager"]);
        assert_eq!(names("documents"), ["document-editor"]);
        assert_eq!(names("download-manager"), ["downloads"]);
        assert_eq!(names("editor"), ["text-editor"]);
        assert_eq!(names("image-viewer"), ["images", "image"]);
        assert_eq!(names("music"), ["music-player"]);
        assert_eq!(names("network"), ["network-manager"]);
        assert_eq!(names("presentation"), ["slides"]);
        assert_eq!(names("snippets"), ["snippet-manager"]);
        assert_eq!(names("system-monitor"), ["sysmonitor"]);
        assert!(names("studio").is_empty(), "`images` is the viewer's; the app that makes them has no alias");
        // No shipped declaration lost a name to the desktop or to another app.
        let raw: usize = shipped.iter().map(|e| e.aliases.len()).sum();
        let kept: usize = declared.iter().map(|d| d.aliases.len()).sum();
        assert_eq!(raw, kept, "a shipped alias collides with another name");
    }

    /// The compiled-in copy of the shipped entries is the directory, file for file, byte for byte.
    #[test]
    fn the_compiled_in_entries_are_the_shipped_files() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop-files");
        let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str()?.strip_suffix(".desktop").map(str::to_string))
            .collect();
        on_disk.sort();
        let mut compiled: Vec<String> = SHIPPED_ENTRIES.iter().map(|(stem, _)| stem.to_string()).collect();
        compiled.sort();
        assert_eq!(on_disk, compiled, "every file in apps/desktop-files is compiled in, and nothing else");
        for (stem, text) in SHIPPED_ENTRIES {
            let file = std::fs::read_to_string(dir.join(format!("{stem}.desktop"))).unwrap();
            assert_eq!(file, text, "{stem}.desktop is compiled in under its own name");
        }
        let by_id = |mut list: Vec<Declared>| {
            list.sort_by(|a, b| a.id.cmp(&b.id));
            list
        };
        assert_eq!(
            by_id(declared(&shipped())),
            by_id(declared(&shipped_catalogue())),
            "the compiled-in entries and the directory declare the same surfaces"
        );
    }

    /// A reach names apps the way people do, and holds them to the id each publishes — ours by the
    /// names the build gives them, whatever this machine has installed.
    #[test]
    fn a_name_folds_to_the_id_its_app_publishes() {
        assert_eq!(surface_id("text-editor").as_deref(), Some("editor"));
        assert_eq!(surface_id("sysmonitor").as_deref(), Some("system-monitor"));
        assert_eq!(surface_id("container_manager").as_deref(), Some("containers"));
        assert_eq!(surface_id("notes").as_deref(), Some("notes"));
        assert_eq!(surface_id("blender").as_deref(), Some("blender"));
        // Not an app: the desktop's own names are kept as written by the caller.
        assert_eq!(surface_id("shell"), None);
        assert_eq!(surface_id("files"), None);
        assert_eq!(surface_id("music-player").as_deref(), Some("music"));
        // Both were shelved, and declared nothing while they were; both came off it (#727, #728).
        assert_eq!(surface_id("spreadsheet").as_deref(), Some("spreadsheet"));
        assert_eq!(surface_id("ysheets").as_deref(), Some("spreadsheet"));
    }

    /// A name the desktop answers to, or that another app holds, is not handed out twice.
    #[test]
    fn names_are_given_out_once() {
        let installed = vec![
            entry("a-files", "A", "files", "x"),
            entry("b", "B", "bee", "notes;hive;shell;settings;report-problem;yantrik"),
            entry("c", "C", "cee", "hive;bee;see"),
            entry("d", "D", "bee", "buzz"),
            entry("e", "E", "blender", "b3d"),
        ];
        let declared = declared(&installed);
        let ids: Vec<&str> = declared.iter().map(|d| d.id.as_str()).collect();
        // `files` is a screen of the desktop: the declaration is refused whole.
        assert_eq!(ids, ["bee", "cee", "blender"]);
        let bee = &declared[0];
        // `notes` is free here (no Notes installed); screens, sections and `shell` are not.
        assert_eq!(bee.aliases, ["notes", "hive", "buzz"], "{bee:?}");
        assert_eq!(bee.entry, "b", "the first entry to declare an id opens it");
        let cee = &declared[1];
        // `hive` was B's first; `bee` is an id, and ids are settled before any alias.
        assert_eq!(cee.aliases, ["see"], "{cee:?}");
        // Blender's route serves the surface of the same name, so a Blender entry may declare it.
        assert_eq!(declared[2].aliases, ["b3d"]);

        // And `find` follows the same rules, strongest match first.
        let found = |name: &str| find(name, &installed).map(|(d, e)| (d.id, e.app_id.clone()));
        assert_eq!(found("hive"), Some(("bee".into(), "b".into())));
        assert_eq!(found("buzz"), Some(("bee".into(), "d".into())), "an alias opens the entry that declared it");
        assert_eq!(found("See"), Some(("cee".into(), "c".into())));
        assert_eq!(found("C"), Some(("cee".into(), "c".into())), "by Name");
        assert_eq!(found("files"), None, "a refused declaration declares nothing");
        assert_eq!(found(""), None);
    }

    /// An app this build has shelved declares nothing, whatever its old file on the disk says.
    #[test]
    fn a_shelved_app_declares_nothing() {
        // Music and ySheets were the whole shelf and both came off it (#727, #728), so this is
        // asked of whatever the shelf holds next.
        for shelf in crate::wire::dock::shelved_table() {
            let stale = entry(shelf.binary, shelf.name, "shelved-surface", "shelved-alias");
            assert!(declared(std::slice::from_ref(&stale)).is_empty(), "{}", shelf.binary);
            assert!(find("shelved-alias", &[stale]).is_none(), "{}", shelf.binary);
        }
        // And the two that came off it declare again, from the same kind of file.
        for (stem, name) in [("yantrik-spreadsheet", "ySheets"), ("yantrik-music-player", "Music")] {
            let back = entry(stem, name, "back", "back-alias");
            assert_eq!(declared(std::slice::from_ref(&back)).len(), 1, "{stem}");
            assert!(find("back-alias", &[back]).is_some(), "{stem}");
        }
    }

    /// Aliases are linked at their surface's socket, relative, whether or not it is running; a
    /// stale file under an alias's name is replaced and a live one is left alone.
    #[cfg(unix)]
    #[test]
    fn aliases_are_linked_at_the_surfaces_socket() {
        let dir = Scratch::new("links");
        let surfaces = vec![Declared {
            id: "containers".into(),
            title: "Containers".into(),
            purpose: String::new(),
            aliases: vec!["container-manager".into(), "docker-ui".into(), "busy".into(), "old".into()],
            entry: "yantrik-container-manager".into(),
        }];
        // Something else is running under `busy`; `old` is a regular file an older release left.
        let _live = std::os::unix::net::UnixListener::bind(dir.0.join("app-busy.sock")).unwrap();
        std::fs::write(dir.0.join("app-old.sock"), b"left by an older release").unwrap();
        // `docker-ui` points at somebody else's socket from an earlier catalogue.
        std::os::unix::fs::symlink("app-somebody.sock", dir.0.join("app-docker-ui.sock")).unwrap();

        let linked = link_aliases_in(&dir.0, &surfaces);
        let names: Vec<&str> = linked.iter().map(|(a, _)| a.as_str()).collect();
        assert_eq!(names, ["container-manager", "docker-ui", "old"]);
        for alias in names {
            let link = dir.0.join(format!("app-{alias}.sock"));
            assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("app-containers.sock"), "{alias}");
        }
        let busy = std::fs::symlink_metadata(dir.0.join("app-busy.sock")).unwrap();
        assert!(!busy.file_type().is_symlink(), "a live socket under an alias's name is not taken");

        // Dangling until the app binds — and then every name reaches the one socket.
        assert!(std::os::unix::net::UnixStream::connect(dir.0.join("app-container-manager.sock")).is_err());
        let _app = std::os::unix::net::UnixListener::bind(dir.0.join("app-containers.sock")).unwrap();
        assert!(std::os::unix::net::UnixStream::connect(dir.0.join("app-container-manager.sock")).is_ok());

        // Run again, as every rescan does: the same links, nothing replaced.
        assert_eq!(link_aliases_in(&dir.0, &surfaces).len(), 3);
    }

    /// An adapter is started beside its app, told what it serves, not started twice, left running
    /// when a second copy hands over, and stopped when its app exits.
    #[cfg(unix)]
    #[test]
    fn an_adapter_runs_beside_its_app_and_stops_with_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = Scratch::new("adapter");
        let report = dir.0.join("env.txt");
        let adapter = dir.0.join("adapter.sh");
        let script = format!(
            "#!/bin/sh\necho \"$YANTRIK_SURFACE $YANTRIK_APP_PID $1\" > {}\nexec sleep 30\n",
            report.display()
        );
        std::fs::write(&adapter, script).unwrap();
        std::fs::set_permissions(&adapter, std::fs::Permissions::from_mode(0o755)).unwrap();
        let surface = format!("adapter-test-{}", std::process::id());
        let app_pid = 4_000_000; // no such process; only its exit is simulated
        let command = format!("{} --flag", adapter.display());

        start_adapter(&surface, &command, app_pid);
        start_adapter(&surface, &command, app_pid);
        let mine = || running_adapters().into_iter().filter(|(s, _)| *s == surface).count();
        assert_eq!(mine(), 1, "started once");

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::fs::read_to_string(&report).map_or(true, |s| s.trim().is_empty())
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        let said = std::fs::read_to_string(&report).unwrap_or_default();
        assert_eq!(said.trim(), format!("{surface} {app_pid} --flag"), "the adapter is told what it serves");

        app_exited(app_pid, true);
        std::thread::sleep(ADAPTER_POLL * 2);
        assert_eq!(mine(), 1, "a hand-over leaves the adapter serving the window that took over");

        app_exited(app_pid, false);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while mine() > 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(mine(), 0, "the adapter stops with its app");
    }

    /// Where the guide's table of surfaces starts and ends. Between them is the shipped
    /// declarations written out, and nothing else.
    const GUIDE_TABLE_BEGIN: &str = "<!-- surfaces: generated from the X-Yantrik-* keys in \
        apps/desktop-files by `YANTRIK_WRITE_DOCS=1 cargo test -p yantrik-ui --bin yantrik-ui \
        the_guide_lists_every_surface` -->";
    const GUIDE_TABLE_END: &str = "<!-- /surfaces -->";

    fn surfaces_table() -> String {
        let mut rows: Vec<(String, Vec<String>, String)> = declared(&shipped_catalogue())
            .into_iter()
            .map(|d| (d.id, d.aliases, d.purpose))
            .collect();
        rows.push((
            "shell".into(),
            Vec::new(),
            "the desktop itself: screens, windows, files, opening apps, approvals".into(),
        ));
        rows.sort();
        let mut out = String::from(
            "\n| Surface | Socket | Also answers to | For |\n| --- | --- | --- | --- |\n",
        );
        for (id, aliases, purpose) in rows {
            let also = if aliases.is_empty() {
                "—".to_string()
            } else {
                aliases.iter().map(|a| format!("`{a}`")).collect::<Vec<_>>().join(", ")
            };
            out.push_str(&format!("| `{id}` | `app-{id}.sock` | {also} | {purpose} |\n"));
        }
        out
    }

    /// `docs/app-control.md` lists the surfaces this desktop ships, and the list is the shipped
    /// `.desktop` files rather than one somebody keeps by hand — the hand-kept one had nine rows
    /// when there were eighteen surfaces. Declaring a surface and not regenerating fails; so does
    /// editing the guide's copy.
    #[test]
    fn the_guide_lists_every_surface_this_desktop_has() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/app-control.md");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let start = text.find(GUIDE_TABLE_BEGIN).expect("the guide marks where its table starts")
            + GUIDE_TABLE_BEGIN.len();
        let end = text.find(GUIDE_TABLE_END).expect("the guide marks where its table ends");
        let table = surfaces_table();
        if std::env::var("YANTRIK_WRITE_DOCS").as_deref() == Ok("1") {
            let written = format!("{}{table}{}", &text[..start], &text[end..]);
            std::fs::write(&path, written).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            return;
        }
        assert_eq!(
            &text[start..end],
            table,
            "docs/app-control.md's table of surfaces is not what apps/desktop-files declares. \
             Regenerate it:\n  YANTRIK_WRITE_DOCS=1 cargo test -p yantrik-ui --bin yantrik-ui \
             the_guide_lists_every_surface"
        );
    }
}
