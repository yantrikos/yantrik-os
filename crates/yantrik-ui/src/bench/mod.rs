//! The Bench: the desktop's icons (issue #1030, phase 1).
//!
//! An ordinary desktop that remembers. `~/Desktop` is listed and kept live from the observer's
//! file events; each icon sits where the person put it, per screen size, in
//! `~/.yantrik/desktop_layout.json`; pictures are cut once and cached; the menus, the keys and the
//! control surface all do the same few things through the same few functions here.
//!
//! The pieces, from the data outward:
//!
//! - `yantrik_bench` (a crate of its own): what the folder holds, the layout file, thumbnails, the
//!   archive and the Continue record. No Slint, no shell.
//! - [`model`]: this desktop's state and the rules the pointer and keys follow. Still no Slint.
//! - [`menu`]: what each right-click offers. Pure.
//! - [`view`]: the model as `BenchState` for `components/bench.slint`.
//! - [`pictures`]: thumbnails made on a worker.
//! - [`ops`]: opening, creating, pasting, archiving; [`keys`]: what a key means; [`wiring`]: the
//!   screen's callbacks.
//!
//! The state lives on the UI thread, in one place ([`with`]); the control surface reads it there
//! too, so `describe` and the screen cannot disagree.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use slint::ComponentHandle;
use yantrik_bench::archive::Archived;
use yantrik_bench::layout::Store;
use yantrik_bench::recent::{self, LastOpened, Target};

use crate::app_context::AppContext;
use crate::apps::Catalogue;
use crate::App;

pub mod describe;
mod keys;
pub mod menu;
pub mod model;
pub(crate) mod ops;
mod persist;
mod pictures;
mod shortcuts;
mod view;
mod wiring;

use model::Model;
use persist::Saver;
use pictures::Pictures;
use shortcuts::Shortcuts;

/// The toast that offers Undo after something is archived. Its id starts `local:` so the
/// notification store never hears of it (wire/toast.rs).
pub const UNDO_TOAST: &str = "local:bench-undo";

/// Which menu is up, so Back can put the first one back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Root {
    Desktop,
    Item,
}

pub struct Bench {
    pub model: Model,
    pictures: Pictures,
    shortcuts: Shortcuts,
    pub home: PathBuf,
    catalogue: Catalogue,
    pub last_opened: Option<LastOpened>,
    /// What each recent Archive moved, by the id of the toast that offers its Undo.
    archived: Vec<(String, Vec<Archived>)>,
    root: Option<Root>,
}

thread_local! {
    static BENCH: RefCell<Option<Rc<RefCell<Bench>>>> = const { RefCell::new(None) };
}

/// Run `f` on the Bench, from the UI thread. `None` when it is not up, or is being used (a
/// callback that opens a file reaches here again through the Lens or Files; skipping that one
/// note is better than a panic).
pub fn with<R>(f: impl FnOnce(&mut Bench) -> R) -> Option<R> {
    let shared = BENCH.with(|b| b.borrow().clone())?;
    let mut bench = shared.try_borrow_mut().ok()?;
    Some(f(&mut bench))
}

/// Make the desktop folder if it is missing (the XDG one when the person has set it), and say
/// where it is. Called before the file watcher starts, which cannot watch a folder that is not
/// there, and again here; making it twice is harmless.
pub fn ensure_desktop(home: &std::path::Path) -> PathBuf {
    let dir = crate::filebrowser::desktop_dir(home);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(dir = %dir.display(), error = %e, "The desktop folder could not be made");
    }
    dir
}

/// Bring the Bench up: read the layout, list the desktop, start the workers and wire the screen.
pub fn wire(ui: &App, ctx: &AppContext) {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let desktop = ensure_desktop(&home);
    let layout = persist::layout_path(&home);
    let record = persist::recent_path(&home);
    let store = Store::load(&layout);
    let saver = Saver::worker(layout, record.clone());
    let last_opened = recent::load(&record);
    let cache = home.join(".cache/yantrik/thumbs");
    let bench = Bench {
        model: Model::new(desktop, store, saver),
        pictures: Pictures::start(cache, ui.as_weak()),
        shortcuts: Shortcuts::default(),
        home,
        catalogue: ctx.installed_apps.clone(),
        last_opened,
        archived: Vec::new(),
        root: None,
    };
    let shared = Rc::new(RefCell::new(bench));
    BENCH.with(|b| *b.borrow_mut() = Some(shared.clone()));
    wiring::wire(ui, shared);
    refresh(ui);
}

/// The observer reported file changes: list the desktop again if one was in it.
pub fn on_events(ui: &App, events: &[yantrik_os::SystemEvent]) {
    let Some(dir) = with(|b| b.model.desktop.clone()) else { return };
    let touched = events.iter().any(|e| match e {
        yantrik_os::SystemEvent::FileChanged { path, kind } => {
            std::path::Path::new(path).starts_with(&dir)
                || matches!(kind, yantrik_os::FileChangeKind::Renamed { to } if std::path::Path::new(to).starts_with(&dir))
        }
        _ => false,
    });
    if touched {
        refresh(ui);
    }
}

/// Scans are numbered, and an answer older than one already applied is dropped, so two scans
/// that finish out of order cannot put an older listing over a newer one.
static SCANS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static APPLIED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// List the desktop on a worker and apply the answer where the UI lives.
pub fn refresh(ui: &App) {
    use std::sync::atomic::Ordering;
    let Some(dir) = with(|b| b.model.desktop.clone()) else { return };
    let weak = ui.as_weak();
    let number = SCANS.fetch_add(1, Ordering::SeqCst) + 1;
    let spawned = std::thread::Builder::new().name("bench-scan".into()).spawn(move || {
        let listing = yantrik_bench::items::scan(&dir);
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if APPLIED.fetch_max(number, Ordering::SeqCst) > number {
                return;
            }
            with(|b| {
                b.model.apply_listing(listing);
                let names: std::collections::HashSet<&str> = b.model.listing.iter().map(|i| i.name.as_str()).collect();
                b.pictures.retain(&names);
                let items = b.model.listing.clone();
                b.shortcuts.retain(&items);
                view::publish(&ui, b);
            });
        });
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "The desktop could not be listed");
    }
}

/// A picture the worker made (or could not make) is ready.
pub(crate) fn arrived(ui: &App, name: String, key: String, pixels: Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>>) {
    with(|b| {
        if b.pictures.arrived(name, key, pixels) {
            view::publish(ui, b);
        }
    });
}

/// A file was opened from Files, the Lens or a notification: it is what Continue offers next.
/// The Bench's own opens record themselves (and reach here only while it is busy, so are skipped).
pub fn note_file_opened(ui: Option<&App>, path: &std::path::Path) {
    let title = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let place = path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    with(|b| {
        b.remember(Target::File { path: path.to_string_lossy().into_owned() }, &title, &place);
        if let Some(ui) = ui {
            view::publish_continue(&ui.global::<crate::BenchState>(), b);
        }
    });
}

/// An app was opened from the Lens: it is what Continue offers next.
pub fn note_app_opened(ui: &App, app_id: &str, title: &str) {
    with(|b| {
        b.remember(Target::App { id: app_id.to_string() }, title, "App");
        view::publish_continue(&ui.global::<crate::BenchState>(), b);
    });
}

/// The Undo button on the toast Archive raised.
pub fn toast_action(ui: &App, id: &str, action: &str) -> bool {
    if !id.starts_with(UNDO_TOAST) {
        return false;
    }
    if action == "undo" {
        ops::undo_archive(ui, id);
    }
    crate::wire::toast::remove(ui, id);
    true
}

impl Bench {
    /// Keep what Continue should offer.
    pub(crate) fn remember(&mut self, target: Target, title: &str, subtitle: &str) {
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let record = LastOpened::new(target, title, subtitle, at);
        self.model.keep_recent(&record);
        self.last_opened = Some(record);
    }
}
