//! Files workbench. Workers perform I/O; the UI owns navigation and selection.
use crate::app_context::AppContext;
use crate::{
    filebrowser as fsview, fileops, App, BreadcrumbSegment, FileDetailData, FileEntry,
    FilePlaceData, FileRecentData, FileTabData, OpenWithItem,
};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{
    cell::RefCell,
    collections::BTreeSet,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Item {
    entry: fsview::DirEntry,
    path: PathBuf,
    trash: Option<fileops::TrashItem>,
}
#[derive(Clone)]
struct Tab {
    path: String,
    back: Vec<String>,
    forward: Vec<String>,
}
#[derive(Clone)]
struct Clipboard {
    paths: Vec<PathBuf>,
    cut: bool,
}
enum Job {
    Transfer(Clipboard, PathBuf),
    Trash(Vec<PathBuf>),
    Restore(Vec<fileops::TrashItem>),
    Undo(Vec<String>),
    Empty,
    Rename(PathBuf, String),
    Create(PathBuf, String, bool),
}
enum Event {
    Listed(
        u64,
        String,
        bool,
        bool,
        Result<(Vec<Item>, String, String), String>,
    ),
    Preview(u64, String, FileDetailData, String),
    Progress(String, f32),
    // Message, what was created (name, is a folder), undo ids, moved paths, remaining trash.
    Done(String, Option<(String, bool)>, Vec<String>, Vec<PathBuf>, Option<Vec<String>>),
    Notice(String),
}
#[derive(Clone)]
struct Sink {
    sender: mpsc::Sender<Event>,
    ui: slint::Weak<App>,
}
impl Sink {
    fn send(&self, event: Event) {
        if self.sender.send(event).is_ok() {
            let _ = self
                .ui
                .upgrade_in_event_loop(|ui| ui.invoke_file_worker_event());
        }
    }
}
struct Browser {
    path: Rc<RefCell<String>>,
    tabs: Vec<Tab>,
    active: usize,
    trash: bool,
    all: Vec<Item>,
    visible: Vec<Item>,
    selected: BTreeSet<usize>,
    anchor: usize,
    hidden: bool,
    filter: String,
    sort: String,
    ascending: bool,
    model: Rc<VecModel<FileEntry>>,
    clipboard: Option<Clipboard>,
    undo: Vec<String>,
    generation: u64,
    preview: u64,
    listing_cancel: Option<Arc<AtomicBool>>,
    job_cancel: Option<Arc<AtomicBool>>,
    sink: Sink,
}
type State = Rc<RefCell<Browser>>;
impl Browser {
    fn tabs_ui(&self, ui: &App) {
        ui.set_file_tabs(ModelRc::new(VecModel::from(
            self.tabs
                .iter()
                .enumerate()
                .map(|(i, t)| FileTabData {
                    path: t.path.clone().into(),
                    label: if t.path == "~" || t.path == "~/" {
                        "Home".into()
                    } else {
                        Path::new(&t.path)
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("/")
                            .into()
                    },
                    is_active: i == self.active,
                })
                .collect::<Vec<_>>(),
        )));
        ui.set_file_active_tab(self.active as i32);
        ui.set_file_can_go_back(!self.tabs[self.active].back.is_empty());
        ui.set_file_can_go_forward(!self.tabs[self.active].forward.is_empty());
    }
    fn paint(&mut self, ui: &App) {
        let filter = self.filter.to_lowercase();
        self.visible = self
            .all
            .iter()
            .filter(|i| {
                (self.hidden || self.trash || !i.entry.name.starts_with('.'))
                    && i.entry.name.to_lowercase().contains(&filter)
            })
            .cloned()
            .collect();
        let field = self.sort.clone();
        let asc = self.ascending;
        self.visible.sort_by(|a, b| {
            let directories = b.entry.is_dir.cmp(&a.entry.is_dir);
            if directories != std::cmp::Ordering::Equal {
                return directories;
            }
            let order = match field.as_str() {
                "size" => a.entry.size_bytes.cmp(&b.entry.size_bytes),
                "modified" => a.entry.modified.cmp(&b.entry.modified),
                _ => a
                    .entry
                    .name
                    .to_lowercase()
                    .cmp(&b.entry.name.to_lowercase()),
            };
            let order = order.then_with(|| a.entry.name.cmp(&b.entry.name));
            if asc {
                order
            } else {
                order.reverse()
            }
        });
        let hidden = self.hidden;
        self.model.set_vec(
            self.visible
                .iter()
                .map(|i| file_entry(&i.entry, hidden))
                .collect::<Vec<_>>(),
        );
        // The row under the grid: what changed last in this folder. From the whole folder, not
        // the filtered view — a search narrows the listing, it does not change what is recent
        // (the row is hidden while one is typed). Trash has no recent row: what was thrown away
        // last is not work to pick up.
        let recent = if self.trash {
            Vec::new()
        } else {
            fsview::most_recent(self.all.iter().map(|i| &i.entry), hidden, RECENT_ROW)
        };
        ui.set_file_recent(ModelRc::new(VecModel::from(
            recent
                .into_iter()
                .map(|e| FileRecentData {
                    name: e.name.clone().into(),
                    size_text: e.size_text.clone().into(),
                    changed_text: fsview::changed_text(e.modified).into(),
                    icon_char: e.icon_char.clone().into(),
                })
                .collect::<Vec<_>>(),
        )));
        self.selected.clear();
        ui.set_file_selected_index(-1);
        ui.set_file_selection_count(0);
        ui.set_file_selection_size_text("".into());
        ui.set_file_detail_data(FileDetailData::default());
        ui.set_file_ai_summary("".into());
        self.preview += 1;
    }
    fn select(&mut self, ui: &App, index: usize, ctrl: bool, shift: bool) {
        if index >= self.visible.len() {
            return;
        }
        let previous = self.selected.clone();
        if shift {
            if !ctrl {
                self.selected.clear();
            }
            self.selected.extend(
                self.anchor.min(index)..=self.anchor.max(index).min(self.visible.len() - 1),
            );
        } else if ctrl {
            if !self.selected.remove(&index) {
                self.selected.insert(index);
            }
            self.anchor = index;
        } else {
            self.selected.clear();
            self.selected.insert(index);
            self.anchor = index;
        }
        for &i in previous.symmetric_difference(&self.selected) {
            if let Some(mut row) = self.model.row_data(i) {
                row.selected = self.selected.contains(&i);
                self.model.set_row_data(i, row);
            }
        }
        ui.set_file_selected_index(if self.selected.contains(&index) {
            index as i32
        } else {
            self.selected.iter().next().map(|i| *i as i32).unwrap_or(-1)
        });
        ui.set_file_selection_count(self.selected.len() as i32);
        // Say what the selection is, not "1 selected · 0.0 KiB in files": a folder answers
        // with what it holds (issue #208), and only real files add up to a size.
        let picked: Vec<&fsview::DirEntry> = self
            .selected
            .iter()
            .filter_map(|i| self.visible.get(*i))
            .map(|i| &i.entry)
            .collect();
        ui.set_file_selection_size_text(fsview::selection_text(&picked, self.hidden).into());
        self.preview += 1;
        ui.set_file_ai_summary("".into());
        ui.set_file_detail_data(FileDetailData::default());
        if ui.get_file_detail_panel_visible() {
            self.preview_item(ui, index, false);
        }
    }
    fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected
            .iter()
            .filter_map(|i| self.visible.get(*i))
            .map(|i| i.path.clone())
            .collect()
    }
    fn clip(&mut self, ui: &App, cut: bool) {
        if self.trash {
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        set_notice(
            ui,
            &format!(
                "{} {}. Open the destination folder and paste.",
                paths.len(),
                if cut {
                    "items ready to move"
                } else {
                    "items copied"
                }
            ),
            None,
        );
        self.clipboard = Some(Clipboard { paths, cut });
        ui.set_file_has_clipboard(true);
    }
    fn load(&mut self, ui: &App, path: String, trash: bool, record: bool) {
        let path = fsview::collapse_home(&fsview::expand_home(&path));
        if record {
            set_notice(ui, "", None);
        }
        if let Some(cancel) = self.listing_cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.listing_cancel = Some(cancel.clone());
        self.generation += 1;
        let generation = self.generation;
        self.preview += 1;
        ui.set_file_browser_loading(true);
        let sink = self.sink.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let root = fileops::trash_root();
                let (items, space, badge) = if trash {
                    let mut counted = 0usize;
                    let items = fileops::trash_items(&root)?
                        .into_iter()
                        .map(|t| {
                            let meta =
                                std::fs::symlink_metadata(&t.stored).map_err(|e| e.to_string())?;
                            // A folder in Trash is counted like any other, within the same
                            // budget: "3 items" is what tells you which "Project" this was.
                            let items = meta.is_dir().then(|| {
                                counted += 1;
                                if counted <= fsview::FOLDER_COUNT_BUDGET {
                                    fsview::count_items(&t.stored)
                                } else {
                                    fsview::ItemCount::Unknown("not counted: Trash holds too many folders".into())
                                }
                            });
                            Ok(Item {
                                entry: fsview::DirEntry {
                                    name: t.name.clone(),
                                    is_dir: meta.is_dir(),
                                    size_text: if meta.is_dir() {
                                        "".into()
                                    } else {
                                        format!("{} B", meta.len())
                                    },
                                    size_bytes: meta.len(),
                                    modified: meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                                    modified_text: "In Trash".into(),
                                    icon_char: if meta.is_dir() {
                                        "folder".into()
                                    } else {
                                        "file".into()
                                    },
                                    selected: false,
                                    items,
                                },
                                path: t.stored.clone(),
                                trash: Some(t),
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    (items, String::new(), String::new())
                } else {
                    let expanded = fsview::expand_home(&path);
                    let dir = std::fs::canonicalize(&expanded)
                        .map_err(|e| format!("Could not open {}: {e}", expanded.display()))?;
                    if cancel.load(Ordering::Acquire) {
                        return Err("Canceled".into());
                    }
                    let mut entries = fsview::list_dir_checked(
                        dir.to_str().ok_or("Folder path is not UTF-8")?,
                        true,
                        "",
                        "name",
                        true,
                    )?;
                    // Each folder's item count, on this worker thread: the tiles' "12 items".
                    fsview::count_folders(&dir, &mut entries, &|| cancel.load(Ordering::Acquire));
                    let items = entries
                    .into_iter()
                    .map(|entry| Item {
                        path: dir.join(&entry.name),
                        entry,
                        trash: None,
                    })
                    .collect();
                    (items, free_space(&dir), fsview::detect_project_type(&path))
                };
                Ok((items, space, badge))
            })();
            if !cancel.load(Ordering::Acquire) {
                sink.send(Event::Listed(generation, path, trash, record, result));
            }
        });
    }
    fn refresh(&mut self, ui: &App) {
        self.load(ui, self.tabs[self.active].path.clone(), self.trash, false);
    }
    fn preview_item(&mut self, ui: &App, index: usize, quick: bool) {
        let Some(item) = self.visible.get(index).cloned() else {
            return;
        };
        if self.trash {
            return;
        }
        self.preview += 1;
        let generation = self.preview;
        let sink = self.sink.clone();
        let name = item.entry.name.clone();
        if quick {
            ui.set_file_quick_look_open(true);
            ui.set_file_quick_look_name(name.clone().into());
            ui.set_file_quick_look_content("Reading preview…".into());
        }
        std::thread::spawn(move || {
            let detail = fsview::get_file_details(
                item.path
                    .parent()
                    .unwrap_or(Path::new("/"))
                    .to_str()
                    .unwrap_or("/"),
                &name,
            );
            let content = if detail.is_text_file {
                detail.preview_text.clone()
            } else {
                format!(
                    "{}\n{}\n{}\n{}",
                    if item.entry.is_dir {
                        "Folder"
                    } else {
                        &detail.file_type
                    },
                    detail.size_text,
                    detail.modified_text,
                    item.path.display()
                )
            };
            sink.send(Event::Preview(
                generation,
                name,
                FileDetailData {
                    name: detail.name.into(),
                    file_type: if item.entry.is_dir {
                        "Folder".into()
                    } else {
                        detail.file_type.into()
                    },
                    size_text: detail.size_text.into(),
                    modified_text: detail.modified_text.into(),
                    path_text: item.path.to_string_lossy().into_owned().into(),
                    permissions: detail.permissions.into(),
                    preview_text: detail.preview_text.into(),
                    is_text_file: detail.is_text_file,
                    icon_char: detail.icon_char.into(),
                },
                content,
            ));
        });
    }
    fn job(&mut self, ui: &App, job: Job) {
        if ui.get_file_browser_loading() {
            set_notice(ui, "Wait for this folder to finish loading.", None);
            return;
        }
        if self.job_cancel.is_some() {
            set_notice(ui, "A file operation is already running. Wait or cancel it first.", None);
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.job_cancel = Some(cancel.clone());
        let sink = self.sink.clone();
        ui.set_file_operation_busy(true);
        ui.set_file_operation_text("Preparing…".into());
        ui.set_file_operation_progress(0.);
        set_notice(ui, "", None);
        std::thread::spawn(move || run_job(job, cancel, sink));
    }
}
/// How many files the recent row under the grid shows.
const RECENT_ROW: usize = 6;

/// One row of the listing, as the screen and `describe` read it.
///
/// A folder carries its item count only when the count was read: `count_known` false with a
/// reason is "we could not look", and the tile says so instead of drawing a zero.
fn file_entry(entry: &fsview::DirEntry, show_hidden: bool) -> FileEntry {
    let (item_count, count_known, count_reason) = match &entry.items {
        Some(count @ fsview::ItemCount::Known { .. }) => {
            (count.shown(show_hidden).unwrap_or(0) as i32, true, String::new())
        }
        Some(fsview::ItemCount::Unknown(reason)) => (0, false, reason.clone()),
        None if entry.is_dir => (0, false, "not counted".to_string()),
        None => (0, false, String::new()),
    };
    FileEntry {
        name: entry.name.clone().into(),
        is_dir: entry.is_dir,
        size_text: entry.size_text.clone().into(),
        modified_text: entry.modified_text.clone().into(),
        icon_char: entry.icon_char.clone().into(),
        selected: false,
        item_count,
        count_known,
        count_reason: count_reason.into(),
        changed_text: fsview::changed_text(entry.modified).into(),
    }
}

/// The sidebar's places for this home, as the screen reads them.
fn set_places(ui: &App) {
    let home = fsview::expand_home("~");
    ui.set_file_places(ModelRc::new(VecModel::from(
        fsview::places(&home)
            .into_iter()
            .map(|p| FilePlaceData { id: p.id.into(), label: p.label.into(), path: p.path.into() })
            .collect::<Vec<_>>(),
    )));
}

fn free_space(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return String::new();
    };
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } == 0 {
        let s = unsafe { stat.assume_init() };
        format!(
            "{:.1} GiB available",
            s.f_bavail as f64 * s.f_frsize as f64 / (1024. * 1024. * 1024.)
        )
    } else {
        String::new()
    }
}
/// The notice a finished job leaves behind. An error or a cancel outranks everything; a
/// creation names the thing it made so the footer can offer Open and Rename for it — the
/// old "Created folder · 1 item" did not say which folder (issue #208); counted jobs keep
/// their summary.
fn done_message(
    label: &str,
    done: usize,
    total: usize,
    error: Option<&String>,
    canceled: bool,
    created: Option<&(String, bool)>,
) -> String {
    if let Some(error) = error {
        format!("{label}: {done}/{total}. {error}")
    } else if canceled {
        format!("Canceled after {done}/{total} items. Completed items are kept.")
    } else if let Some((name, folder)) = created {
        format!("Created {} \"{name}\"", if *folder { "folder" } else { "file" })
    } else {
        format!("{label} · {done} {}", if done == 1 { "item" } else { "items" })
    }
}

/// Put a notice in the footer. `created` carries the name of the thing just made, and
/// whether it is a folder, when the notice is about a creation — the footer's Open and
/// Rename buttons act on that name. Every other notice clears them.
pub(super) fn set_notice(ui: &App, text: &str, created: Option<(String, bool)>) {
    ui.set_file_notice(text.into());
    ui.set_file_notice_created(
        created
            .as_ref()
            .map(|(name, _)| name.as_str())
            .unwrap_or("")
            .into(),
    );
    ui.set_file_notice_created_dir(created.is_some_and(|(_, folder)| folder));
}

fn run_job(job: Job, cancel: Arc<AtomicBool>, sink: Sink) {
    let root = fileops::trash_root();
    let mut undo = vec![];
    let mut moved = vec![];
    let mut errors = vec![];
    let mut created = None;
    let mut done = 0usize;
    let mut total = 1usize;
    let mut label = "Completed";
    match job {
        Job::Transfer(clip, dst) => {
            total = clip.paths.len();
            label = if clip.cut { "Moved" } else { "Copied" };
            for src in clip.paths {
                if cancel.load(Ordering::Acquire) {
                    break;
                }
                let title = src
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let mut bytes = 0u64;
                let mut last = Instant::now();
                sink.send(Event::Progress(
                    format!(
                        "{} · {} of {} · {}",
                        if clip.cut { "Moving" } else { "Copying" },
                        done + 1,
                        total,
                        title
                    ),
                    done as f32 / total.max(1) as f32,
                ));
                let result = fileops::transfer(&src, &dst, clip.cut, &cancel, &mut |n| {
                    bytes += n;
                    if last.elapsed() > Duration::from_millis(100) {
                        last = Instant::now();
                        sink.send(Event::Progress(
                            format!("Copying {title} · {:.1} MiB", bytes as f64 / 1048576.),
                            done as f32 / total.max(1) as f32,
                        ));
                    }
                });
                match result {
                    Ok(()) => {
                        done += 1;
                        if clip.cut {
                            moved.push(src);
                        }
                    }
                    Err(e) => {
                        errors.push(e);
                        break;
                    }
                }
            }
        }
        Job::Trash(paths) => {
            total = paths.len();
            label = "Moved to Trash";
            for p in paths {
                if cancel.load(Ordering::Acquire) {
                    break;
                }
                match fileops::trash(&p, &root) {
                    Ok(id) => {
                        undo.push(id);
                        done += 1;
                    }
                    Err(e) => {
                        errors.push(e);
                        break;
                    }
                }
                sink.send(Event::Progress(
                    format!("Moving to Trash · {done} of {total}"),
                    done as f32 / total.max(1) as f32,
                ));
            }
        }
        Job::Restore(items) => {
            total = items.len();
            label = "Restored";
            for item in items {
                if cancel.load(Ordering::Acquire) {
                    break;
                }
                match fileops::restore(&item, &root) {
                    Ok(()) => done += 1,
                    Err(e) => {
                        errors.push(e);
                        break;
                    }
                }
            }
        }
        Job::Undo(ids) => {
            label = "Restored";
            total = ids.len();
            match fileops::trash_items(&root) {
                Ok(items) => {
                    for item in items.into_iter().filter(|i| ids.contains(&i.id)) {
                        if cancel.load(Ordering::Acquire) {
                            break;
                        }
                        match fileops::restore(&item, &root) {
                            Ok(()) => done += 1,
                            Err(e) => {
                                errors.push(e);
                                break;
                            }
                        }
                    }
                }
                Err(e) => errors.push(e),
            }
        }
        Job::Empty => {
            label = "Emptied Trash";
            match fileops::empty_trash(&root, &cancel) {
                Ok(()) => done = 1,
                Err(e) => errors.push(e),
            }
        }
        Job::Rename(src, new) => {
            label = "Renamed";
            let result = fileops::name(&new).and_then(|_| {
                fileops::rename_no_replace(
                    &src,
                    &src.parent().ok_or("No parent folder")?.join(&new),
                )
            });
            match result {
                Ok(()) => done = 1,
                Err(e) => errors.push(e),
            }
        }
        Job::Create(dir, name, folder) => {
            label = if folder {
                "Created folder"
            } else {
                "Created file"
            };
            match if folder {
                fileops::create_folder(&dir, &name)
            } else {
                fileops::create_file(&dir, &name)
            } {
                Ok(()) => {
                    done = 1;
                    created = Some((name, folder));
                }
                Err(e) => errors.push(e),
            }
        }
    }
    let message = done_message(
        label,
        done,
        total,
        errors.first(),
        cancel.load(Ordering::Acquire),
        created.as_ref(),
    );
    let remaining = fileops::trash_items(&root)
        .ok()
        .map(|items| items.into_iter().map(|i| i.id).collect());
    sink.send(Event::Done(message, created, undo, moved, remaining));
}

pub fn wire(ui: &App, ctx: &AppContext) {
    let (sender, receiver) = mpsc::channel();
    let sink = Sink {
        sender,
        ui: ui.as_weak(),
    };
    let model = Rc::new(VecModel::from(vec![]));
    ui.set_file_browser_entries(ModelRc::from(model.clone()));
    let state = Rc::new(RefCell::new(Browser {
        path: ctx.browser_path.clone(),
        tabs: vec![Tab {
            path: ctx.browser_path.borrow().clone(),
            back: vec![],
            forward: vec![],
        }],
        active: 0,
        trash: false,
        all: vec![],
        visible: vec![],
        selected: BTreeSet::new(),
        anchor: 0,
        hidden: false,
        filter: String::new(),
        sort: "name".into(),
        ascending: true,
        model,
        clipboard: None,
        undo: vec![],
        generation: 0,
        preview: 0,
        listing_cancel: None,
        job_cancel: None,
        sink,
    }));
    macro_rules! bind {($callback:ident, |$u:ident,$s:ident $(,$arg:ident)*| $body:block)=>{{let weak=ui.as_weak();let state=state.clone();ui.$callback(move|$($arg),*|{if let Some($u)=weak.upgrade(){let mut $s=state.borrow_mut();$body}});}};}
    bind!(on_file_worker_event, |u, s| {
        while let Ok(event) = receiver.try_recv() {
            match event {
                Event::Listed(generation, path, trash, record, result) => {
                    if generation != s.generation {
                        continue;
                    }
                    u.set_file_browser_loading(false);
                    match result {
                        Ok((items, space, badge)) => {
                            if record && !s.trash && s.tabs[s.active].path != path {
                                let active = s.active;
                                let old = s.tabs[active].path.clone();
                                s.tabs[active].back.push(old);
                                s.tabs[active].forward.clear();
                            }
                            s.trash = trash;
                            u.set_file_trash_mode(trash);
                            if !trash {
                                let active = s.active;
                                s.tabs[active].path = path.clone();
                                *s.path.borrow_mut() = path.clone();
                            }
                            let display = if trash { "Trash" } else { &path };
                            u.set_file_browser_path(display.into());
                            u.set_file_breadcrumbs(ModelRc::new(VecModel::from(if trash {
                                vec![BreadcrumbSegment {
                                    label: "Trash".into(),
                                    full_path: "Trash".into(),
                                }]
                            } else {
                                fsview::breadcrumb_segments(&path)
                                    .into_iter()
                                    .map(|(label, full_path)| BreadcrumbSegment {
                                        label: label.into(),
                                        full_path: full_path.into(),
                                    })
                                    .collect()
                            })));
                            u.set_file_free_space_text(space.into());
                            u.set_file_dir_type_badge(badge.into());
                            // Places are re-read with each listing: a ~/Projects made a minute
                            // ago appears the next time any folder is opened. Eight `stat`s.
                            set_places(&u);
                            if trash {
                                u.set_file_trash_count(items.len() as i32);
                            }
                            s.all = items;
                            s.paint(&u);
                            s.tabs_ui(&u);
                        }
                        Err(e) => set_notice(&u, &e, None),
                    }
                }
                Event::Preview(generation, name, detail, content) => {
                    if generation == s.preview {
                        u.set_file_detail_data(detail);
                        u.set_file_quick_look_name(name.into());
                        u.set_file_quick_look_content(content.into());
                    }
                }
                Event::Progress(text, progress) => {
                    u.set_file_operation_text(text.into());
                    u.set_file_operation_progress(progress);
                }
                Event::Done(message, created, undo, moved, remaining) => {
                    s.job_cancel = None;
                    u.set_file_operation_busy(false);
                    u.set_file_operation_text(message.clone().into());
                    set_notice(&u, &message, created);
                    if !undo.is_empty() {
                        s.undo = undo;
                    }
                    if let Some(ids) = remaining {
                        s.undo.retain(|id| ids.contains(id));
                    }
                    if let Some(clip) = s.clipboard.as_mut() {
                        if clip.cut {
                            clip.paths.retain(|p| !moved.contains(p));
                            if clip.paths.is_empty() {
                                s.clipboard = None;
                            }
                        }
                    }
                    u.set_file_has_clipboard(s.clipboard.is_some());
                    u.set_file_can_undo(!s.undo.is_empty());
                    s.refresh(&u);
                }
                Event::Notice(text) => set_notice(&u, &text, None),
            }
        }
    });
    bind!(on_file_refresh, |u, s| {
        s.refresh(&u);
    });
    bind!(on_file_navigate_to_path, |u, s, path| {
        s.filter.clear();
        s.load(&u, path.to_string(), false, true);
    });
    bind!(on_file_navigate_dir, |u, s, name| {
        if !s.trash && fileops::name(name.as_str()).is_ok() {
            let path = fsview::child_path(&s.tabs[s.active].path, &name);
            s.filter.clear();
            s.load(&u, path, false, true);
        }
    });
    bind!(on_file_go_up, |u, s| {
        if !s.trash {
            let path = fsview::parent_path(&s.tabs[s.active].path);
            s.filter.clear();
            s.load(&u, path, false, true);
        }
    });
    bind!(on_file_go_back, |u, s| {
        let i = s.active;
        if let Some(path) = s.tabs[i].back.pop() {
            let current = s.tabs[i].path.clone();
            s.tabs[i].forward.push(current);
            s.filter.clear();
            s.load(&u, path, false, false);
        }
    });
    bind!(on_file_go_forward, |u, s| {
        let i = s.active;
        if let Some(path) = s.tabs[i].forward.pop() {
            let current = s.tabs[i].path.clone();
            s.tabs[i].back.push(current);
            s.filter.clear();
            s.load(&u, path, false, false);
        }
    });
    bind!(on_file_toggle_hidden, |u, s| {
        s.hidden = !s.hidden;
        u.set_file_show_hidden(s.hidden);
        s.paint(&u);
    });
    bind!(on_file_filter_changed, |u, s, text| {
        s.filter = text.to_string();
        s.paint(&u);
    });
    bind!(on_file_sort_changed, |u, s, field, asc| {
        s.sort = field.to_string();
        s.ascending = asc;
        s.paint(&u);
    });
    bind!(on_file_multi_select_clicked, |u, s, index, ctrl, shift| {
        if index >= 0 {
            s.select(&u, index as usize, ctrl, shift);
        }
    });
    bind!(on_file_select_all, |u, s| {
        if !s.visible.is_empty() {
            s.anchor = 0;
            let last = s.visible.len() - 1;
            s.select(&u, last, false, true);
        }
    });
    bind!(on_file_clear_selection, |u, s| {
        for &i in &s.selected {
            if let Some(mut row) = s.model.row_data(i) {
                row.selected = false;
                s.model.set_row_data(i, row);
            }
        }
        s.selected.clear();
        s.preview += 1;
        u.set_file_detail_data(FileDetailData::default());
        u.set_file_selected_index(-1);
        u.set_file_selection_count(0);
        u.set_file_selection_size_text("".into());
    });
    bind!(on_file_copy_selected, |u, s| {
        s.clip(&u, false);
    });
    bind!(on_file_cut_selected, |u, s| {
        s.clip(&u, true);
    });
    bind!(on_file_copy, |u, s, name| {
        if let Some(index) = s.visible.iter().position(|i| i.entry.name == name.as_str()) {
            s.select(&u, index, false, false);
            s.clip(&u, false);
        }
    });
    bind!(on_file_cut, |u, s, name| {
        if let Some(index) = s.visible.iter().position(|i| i.entry.name == name.as_str()) {
            s.select(&u, index, false, false);
            s.clip(&u, true);
        }
    });
    bind!(on_file_paste, |u, s| {
        if !s.trash {
            if let Some(clip) = s.clipboard.clone() {
                let dir = fsview::expand_home(&s.tabs[s.active].path);
                s.job(&u, Job::Transfer(clip, dir));
            }
        }
    });
    bind!(on_file_delete_selected, |u, s| {
        if !s.trash {
            let paths = s.selected_paths();
            if !paths.is_empty() {
                s.job(&u, Job::Trash(paths));
            }
        }
    });
    bind!(on_file_delete, |u, s, name| {
        if !s.trash {
            if let Some(item) = s
                .visible
                .iter()
                .find(|i| i.entry.name == name.as_str())
                .cloned()
            {
                s.job(&u, Job::Trash(vec![item.path]));
            }
        }
    });
    bind!(on_file_rename, |u, s, old, new| {
        if !s.trash {
            if let Some(item) = s
                .visible
                .iter()
                .find(|i| i.entry.name == old.as_str())
                .cloned()
            {
                if old != new {
                    s.job(&u, Job::Rename(item.path, new.to_string()));
                }
            }
        }
    });
    bind!(on_file_create_file, |u, s, name| {
        if !s.trash {
            let dir = fsview::expand_home(&s.tabs[s.active].path);
            s.job(&u, Job::Create(dir, name.to_string(), false));
        }
    });
    bind!(on_file_create_folder, |u, s, name| {
        if !s.trash {
            let dir = fsview::expand_home(&s.tabs[s.active].path);
            s.job(&u, Job::Create(dir, name.to_string(), true));
        }
    });
    bind!(on_file_cancel_operation, |u, s| {
        if let Some(cancel) = &s.job_cancel {
            cancel.store(true, Ordering::Release);
            u.set_file_operation_text("Canceling…".into());
        }
    });
    bind!(on_file_dismiss_notice, |u, s| {
        set_notice(&u, "", None);
    });
    bind!(on_file_toggle_trash, |u, s| {
        let path = s.tabs[s.active].path.clone();
        let trash = !s.trash;
        s.filter.clear();
        s.load(&u, path, trash, false);
    });
    bind!(on_file_restore_from_trash, |u, s, index| {
        if s.trash {
            if let Some(item) = s.visible.get(index as usize).and_then(|i| i.trash.clone()) {
                s.job(&u, Job::Restore(vec![item]));
            }
        }
    });
    bind!(on_file_empty_trash, |u, s| {
        if s.trash {
            s.job(&u, Job::Empty);
        }
    });
    bind!(on_file_undo_trash, |u, s| {
        let ids = s.undo.clone();
        if !ids.is_empty() {
            s.job(&u, Job::Undo(ids));
        }
    });
    bind!(on_file_new_tab, |u, s| {
        if s.tabs.len() < 8 {
            let path = s.tabs[s.active].path.clone();
            s.tabs.push(Tab {
                path: path.clone(),
                back: vec![],
                forward: vec![],
            });
            s.active = s.tabs.len() - 1;
            s.filter.clear();
            s.load(&u, path, false, false);
            s.tabs_ui(&u);
        }
    });
    bind!(on_file_switch_tab, |u, s, index| {
        if index >= 0 && (index as usize) < s.tabs.len() {
            s.active = index as usize;
            let path = s.tabs[s.active].path.clone();
            s.filter.clear();
            s.load(&u, path, false, false);
            s.tabs_ui(&u);
        }
    });
    bind!(on_file_close_tab, |u, s, index| {
        let index = index as usize;
        if s.tabs.len() > 1 && index < s.tabs.len() {
            s.tabs.remove(index);
            if index < s.active {
                s.active -= 1;
            }
            s.active = s.active.min(s.tabs.len() - 1);
            let path = s.tabs[s.active].path.clone();
            s.filter.clear();
            s.load(&u, path, false, false);
            s.tabs_ui(&u);
        }
    });
    bind!(on_file_selection_changed, |u, s, name| {
        if let Some(index) = s.visible.iter().position(|i| i.entry.name == name.as_str()) {
            s.preview_item(&u, index, false);
        }
    });
    bind!(on_file_quick_look, |u, s, index| {
        if index >= 0 {
            s.preview_item(&u, index as usize, true);
        }
    });
    bind!(on_file_close_quick_look, |u, s| {
        s.preview += 1;
        u.set_file_quick_look_open(false);
    });
    bind!(on_file_context_copy_path, |u, s, name| {
        if let Some(item) = s.visible.iter().find(|i| i.entry.name == name.as_str()) {
            let path = item.path.to_string_lossy().into_owned();
            let sink = s.sink.clone();
            std::thread::spawn(move || {
                use std::io::Write;
                let result = std::process::Command::new("wl-copy")
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                    .and_then(|mut child| {
                        child.stdin.take().unwrap().write_all(path.as_bytes())?;
                        let status = child.wait()?;
                        if status.success() {
                            Ok(())
                        } else {
                            Err(std::io::Error::other("Clipboard unavailable"))
                        }
                    });
                if let Err(e) = result {
                    sink.send(Event::Notice(e.to_string()));
                }
            });
        }
    });
    // Opening an existing Terminal must create a new tab at the requested path.
    bind!(on_file_context_open_terminal, |u, s| {
        if !s.trash {
            let dir = fsview::expand_home(&s.tabs[s.active].path);
            let sink = s.sink.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let client = yantrik_ipc_transport::client::RpcClient::for_service("app-terminal");
                let result=rt.block_on(async {tokio::time::timeout(Duration::from_secs(4),client.call("app.act",serde_json::json!({"action":"open_directory","args":{"directory":dir.to_string_lossy()}}))).await});
                match result {
                    Ok(Ok(reply))
                        if reply.get("accepted").and_then(|v| v.as_bool()) == Some(true) =>
                    {
                        let _ = std::process::Command::new("wlrctl")
                            .args(["toplevel", "focus", "title:Terminal"])
                            .status();
                    }
                    Ok(Err(e)) if e.message.starts_with("Connection failed") => {
                        super::dock::spawn_app_in("terminal", "yantrik-terminal", &[], Some(&dir))
                    }
                    other => sink.send(Event::Notice(format!(
                        "Could not open Terminal here: {other:?}"
                    ))),
                }
            });
        }
    });
    // The "Open with" list, rebuilt every time the menu asks: what is installed and which app
    // is the default can both change while Files stays open, and a stale row would promise a
    // launch that cannot happen (#233). The list comes from the same rule the double-click
    // follows, so its first row IS the default.
    bind!(on_file_open_with_requested, |u, s| {
        let rows = match s.selected.iter().next().and_then(|i| s.visible.get(*i)) {
            Some(item) if !s.trash && !item.entry.is_dir => {
                let defaults = crate::mime_dispatch::MimeDefaults::read();
                let installed = crate::apps::Catalogue::shared().get();
                // The machine's browser by the desktop id the defaults file would name, so a
                // browser row and a written default speak the same language.
                let browser =
                    super::dock::find_browser().map(|(bin, _)| format!("{bin}.desktop"));
                crate::mime_dispatch::open_with(
                    &item.entry.name,
                    &defaults,
                    &installed,
                    browser.as_deref(),
                )
            }
            // A folder or the Trash has nothing to open with; the menu shows the empty state.
            _ => vec![],
        };
        u.set_file_open_with_apps(ModelRc::new(VecModel::from(
            rows.into_iter()
                .map(|r| OpenWithItem {
                    name: r.name.into(),
                    id: r.id.into(),
                    is_default: r.is_default,
                })
                .collect::<Vec<_>>(),
        )));
    });
    // One row of the list was clicked: open the file in the app the row names, through the
    // one launcher, so a row and a double-click cannot start an app differently (#233).
    bind!(on_file_open_with, |u, s, app| {
        if s.trash {
            return;
        }
        if let Some(item) = s.selected.iter().next().and_then(|i| s.visible.get(*i)) {
            if !item.entry.is_dir {
                super::open_with::launch_desktop_id(
                    app.as_str(),
                    &item.entry.name,
                    &item.path,
                    Some(&u),
                );
            }
        }
    });
    // "Always use this app": write the default into the person's own mimeapps.list, say on
    // the screen that it was remembered, and open the file they pointed at in the app they
    // chose — the click that asked for the change should see the change (#233).
    bind!(on_file_open_with_always, |u, s, app, name| {
        if s.trash {
            return;
        }
        if let Some(item) = s.selected.iter().next().and_then(|i| s.visible.get(*i)) {
            if item.entry.is_dir {
                return;
            }
            let app = app.to_string();
            let display = name.to_string();
            match crate::mime_dispatch::mime_for(&item.entry.name) {
                Some(mime) => {
                    match crate::mime_dispatch::MimeDefaults::set_default(mime, &app) {
                        Ok(()) => {
                            let ext = Path::new(&item.entry.name)
                                .extension()
                                .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
                                .unwrap_or_else(|| item.entry.name.clone());
                            set_notice(
                                &u,
                                &format!("From now on, {ext} files open in {display}."),
                                None,
                            );
                        }
                        Err(e) => set_notice(
                            &u,
                            &format!("Could not remember the choice: {e}"),
                            None,
                        ),
                    }
                }
                None => set_notice(
                    &u,
                    &format!(
                        "{} has no file type the shell knows, so the choice cannot be remembered.",
                        item.entry.name
                    ),
                    None,
                ),
            }
            super::open_with::launch_desktop_id(&app, &item.entry.name, &item.path, Some(&u));
        }
    });
    set_places(ui);
    state.borrow().tabs_ui(ui);
}

#[cfg(test)]
mod tests {
    use super::done_message;

    #[test]
    fn a_creation_names_the_thing_it_made() {
        let made = ("Tour 23 Sep".to_string(), true);
        assert_eq!(
            done_message("Created folder", 1, 1, None, false, Some(&made)),
            "Created folder \"Tour 23 Sep\""
        );
        let made = ("notes.md".to_string(), false);
        assert_eq!(
            done_message("Created file", 1, 1, None, false, Some(&made)),
            "Created file \"notes.md\""
        );
    }

    #[test]
    fn counted_jobs_keep_their_summary() {
        assert_eq!(done_message("Copied", 1, 3, None, false, None), "Copied · 1 item");
        assert_eq!(
            done_message("Moved to Trash", 4, 4, None, false, None),
            "Moved to Trash · 4 items"
        );
    }

    #[test]
    fn an_error_or_a_cancel_outranks_the_name() {
        let made = ("Tour 23 Sep".to_string(), true);
        let error = "File exists".to_string();
        assert_eq!(
            done_message("Created folder", 0, 1, Some(&error), false, Some(&made)),
            "Created folder: 0/1. File exists"
        );
        assert_eq!(
            done_message("Copied", 2, 5, None, true, None),
            "Canceled after 2/5 items. Completed items are kept."
        );
    }
}
