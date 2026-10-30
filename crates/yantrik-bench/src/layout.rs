//! Where each icon sits, and the file that remembers it.
//!
//! The desktop is a grid of cells, filled column by column from the top left, as every desktop
//! the person has used fills it. An icon the person has moved stays where they put it: in a cell
//! when snapping is on (the default), at a pixel position when it is off. The positions are kept
//! in `~/.yantrik/desktop_layout.json`, one map per screen size, because a cell that is on a
//! 1920x1080 desktop may be off a 1280x800 one.
//!
//! The rules the file is held to:
//!
//! - **Written atomically.** A temporary file beside it, synced, then renamed over it: a crash
//!   leaves the old layout or the new one, never half of either.
//! - **Never written over what could not be read.** A file that is there but unreadable (a
//!   permission, a disk error) is left alone, and this run's layout is kept in memory only. A
//!   file that does not parse is moved aside to `.corrupt` before the new one is written, so
//!   nothing the person arranged is destroyed by a bug of ours.
//! - **New files flow in, gone files free their cell.** A name with no saved place takes the
//!   next free cell; a saved place for a file that is no longer there is dropped when the layout
//!   is next written, so the cell is free for whatever arrives.
//! - **A new screen size starts from the last one.** Cells that still fit stay; the rest flow.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::items::Item;

/// The layout file's own version, so a later phase can add to it without guessing.
pub const VERSION: u32 = 1;
/// More than this and the file is not a layout.
const MAX_FILE_BYTES: u64 = 1 << 20;
/// Screen sizes remembered. A laptop that docks and undocks uses two or three.
const MAX_LAYOUTS: usize = 8;
/// Places remembered per screen size. The listing itself is capped lower (`items::MAX_ITEMS`).
const MAX_ENTRIES: usize = 4000;

/// The size of a cell and the margin around the grid, in logical pixels. The shell hands these
/// to the Slint side, so the picture and the arithmetic cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metrics {
    pub cell_w: i32,
    pub cell_h: i32,
    pub pad_x: i32,
    pub pad_y: i32,
}

pub const METRICS: Metrics = Metrics { cell_w: 104, cell_h: 116, pad_x: 20, pad_y: 8 };

/// The icon area at one screen size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    pub m: Metrics,
    pub w: i32,
    pub h: i32,
    /// The width not covered by the panel.
    pub usable_w: i32,
    pub cols: u32,
    pub rows: u32,
}

impl Grid {
    /// The grid for an area `w` by `h` of which the right `reserve` is covered (by the Minds
    /// panel). The size is what the layout is kept under; the reserve only decides how many
    /// columns there is room for, so opening the panel reflows the icons without rewriting where
    /// the person put them.
    pub fn new(w: i32, h: i32, reserve: i32, m: Metrics) -> Grid {
        let usable = (w - reserve.max(0)).max(m.cell_w);
        let cols = ((usable - 2 * m.pad_x) / m.cell_w).max(1) as u32;
        let rows = ((h - 2 * m.pad_y) / m.cell_h).max(1) as u32;
        Grid { m, w: w.max(m.cell_w), h: h.max(m.cell_h), usable_w: usable, cols, rows }
    }

    /// The key this size's layout is kept under.
    pub fn key(&self) -> String {
        format!("{}x{}", self.w, self.h)
    }

    pub fn origin(&self, col: u32, row: u32) -> (i32, i32) {
        (self.m.pad_x + col as i32 * self.m.cell_w, self.m.pad_y + row as i32 * self.m.cell_h)
    }

    /// The cell whose top left is nearest `(x, y)`, kept inside the grid.
    pub fn nearest_cell(&self, x: i32, y: i32) -> (u32, u32) {
        let round = |v: i32, size: i32, pad: i32, max: u32| -> u32 {
            let c = ((v - pad) as f32 / size as f32).round();
            c.clamp(0.0, (max - 1) as f32) as u32
        };
        (round(x, self.m.cell_w, self.m.pad_x, self.cols), round(y, self.m.cell_h, self.m.pad_y, self.rows))
    }

    /// A free pixel position kept wholly on the screen.
    pub fn clamp_free(&self, x: i32, y: i32) -> (i32, i32) {
        (x.clamp(0, (self.usable_w - self.m.cell_w).max(0)), y.clamp(0, (self.h - self.m.cell_h).max(0)))
    }

    /// How many icons fit before the grid is full.
    pub fn capacity(&self) -> usize {
        self.cols as usize * self.rows as usize
    }
}

/// One icon's place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Pos {
    Cell { col: u32, row: u32 },
    Free { x: i32, y: i32 },
}

/// An icon with its place worked out for this grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub name: String,
    pub x: i32,
    pub y: i32,
    /// The cell, when it is in one.
    pub cell: Option<(u32, u32)>,
}

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("the layout file could not be read earlier, so it is not written over: {0}")]
    Unreadable(String),
    #[error("the layout could not be written: {0}")]
    Write(#[from] std::io::Error),
    #[error("the layout could not be encoded: {0}")]
    Encode(#[from] serde_json::Error),
}

fn yes() -> bool {
    true
}

/// Every remembered place, for every screen size.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub version: u32,
    /// Snap to the grid. On unless the person turned it off.
    #[serde(default = "yes")]
    pub snap: bool,
    /// "Show desktop icons": on unless the person turned it off.
    #[serde(default = "yes")]
    pub icons: bool,
    /// The screen size most recently used: where a new size starts from.
    #[serde(default)]
    pub last: Option<String>,
    #[serde(default)]
    layouts: BTreeMap<String, BTreeMap<String, Pos>>,
    /// The file was there and could not be read: do not write over it.
    #[serde(skip)]
    unreadable: Option<String>,
    /// The file was there and was not a layout: move it aside before writing.
    #[serde(skip)]
    corrupt: bool,
}

impl Default for Store {
    fn default() -> Self {
        Store { version: VERSION, snap: true, icons: true, last: None, layouts: BTreeMap::new(), unreadable: None, corrupt: false }
    }
}

impl Store {
    /// Read the layout file. Never fails: a missing file is a new desktop, and one that cannot
    /// be read or parsed is flagged (see the module notes) and starts empty.
    pub fn load(path: &Path) -> Store {
        let mut text = String::new();
        let read = std::fs::File::open(path).and_then(|f| f.take(MAX_FILE_BYTES + 1).read_to_string(&mut text));
        match read {
            Ok(_) if text.len() as u64 > MAX_FILE_BYTES => Store { corrupt: true, ..Store::default() },
            Ok(_) => match serde_json::from_str::<Store>(&text) {
                Ok(mut s) => {
                    s.version = VERSION;
                    s
                }
                Err(_) => Store { corrupt: true, ..Store::default() },
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Store::default(),
            // Not UTF-8 is a file that is not a layout; any other error is the disk's or the
            // permission's, and the file may be fine.
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => Store { corrupt: true, ..Store::default() },
            Err(e) => Store { unreadable: Some(e.to_string()), ..Store::default() },
        }
    }

    /// The layout is being written by another thread from a copy of this one: the old file that
    /// did not parse is that write's to set aside, not the next one's.
    pub fn handed_over(&mut self) {
        self.corrupt = false;
    }

    /// Write the layout, atomically. See the module notes for what is refused.
    pub fn save(&mut self, path: &Path) -> Result<(), LayoutError> {
        if let Some(why) = &self.unreadable {
            return Err(LayoutError::Unreadable(why.clone()));
        }
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let json = serde_json::to_vec_pretty(&*self)?;
        if self.corrupt {
            let mut aside = path.as_os_str().to_owned();
            aside.push(".corrupt");
            match std::fs::rename(path, PathBuf::from(aside)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            self.corrupt = false;
        }
        Ok(crate::atomic::write(path, &json)?)
    }

    /// Where `name` is remembered at the grid's size, if it is.
    pub fn get(&self, grid_key: &str, name: &str) -> Option<Pos> {
        self.layouts.get(grid_key)?.get(name).copied()
    }

    /// Remember a place, and that this is the screen size most recently used.
    pub fn set(&mut self, grid_key: &str, name: &str, pos: Pos) {
        self.last = Some(grid_key.to_string());
        if !self.layouts.contains_key(grid_key) && self.layouts.len() >= MAX_LAYOUTS {
            if let Some(old) = self.layouts.keys().find(|k| Some(k.as_str()) != self.last.as_deref()).cloned() {
                self.layouts.remove(&old);
            }
        }
        let layout = self.layouts.entry(grid_key.to_string()).or_default();
        if layout.len() < MAX_ENTRIES || layout.contains_key(name) {
            layout.insert(name.to_string(), pos);
        }
    }

    /// Forget every place for a file that is not in `existing`. Returns whether anything went.
    pub fn drop_missing(&mut self, existing: &HashSet<&str>) -> bool {
        let mut changed = false;
        for layout in self.layouts.values_mut() {
            let before = layout.len();
            layout.retain(|name, _| existing.contains(name.as_str()));
            changed |= layout.len() != before;
        }
        changed
    }

    /// A file was renamed on the desktop: it keeps its place at every screen size.
    pub fn rename(&mut self, old: &str, new: &str) {
        for layout in self.layouts.values_mut() {
            if let Some(pos) = layout.remove(old) {
                layout.insert(new.to_string(), pos);
            }
        }
    }

    /// Remember where a new arrival flowed to, so it stays there. Without this the next file to
    /// go would pull every later icon up a cell: an icon is only where the person can expect it
    /// to be if it was fixed the moment it arrived. Only cells are kept (a cascaded icon has no
    /// cell and is placed again next time). Returns whether anything was added.
    pub fn adopt(&mut self, grid_key: &str, placed: &[Placed]) -> bool {
        let mut added = false;
        for p in placed {
            if let Some((col, row)) = p.cell {
                if self.layouts.get(grid_key).map_or(true, |l| !l.contains_key(&p.name)) {
                    self.set(grid_key, &p.name, Pos::Cell { col, row });
                    added = true;
                }
            }
        }
        added
    }

    /// The places to start from at this grid: its own, or the most recent other size's.
    fn base(&self, grid_key: &str) -> Option<&BTreeMap<String, Pos>> {
        self.layouts
            .get(grid_key)
            .or_else(|| self.last.as_ref().and_then(|k| self.layouts.get(k)))
            .or_else(|| self.layouts.values().next())
    }

    /// Every icon's place on `grid`, in `order` (the order new files flow in).
    pub fn resolve(&self, grid: &Grid, order: &[String]) -> Vec<Placed> {
        let saved = self.base(&grid.key());
        let mut taken: HashSet<(u32, u32)> = HashSet::new();
        let mut placed: HashMap<&str, Placed> = HashMap::new();
        let mut unplaced: Vec<&str> = Vec::new();
        for name in order {
            match saved.and_then(|s| s.get(name)).copied() {
                Some(Pos::Cell { col, row }) if col < grid.cols && row < grid.rows && taken.insert((col, row)) => {
                    let (x, y) = grid.origin(col, row);
                    placed.insert(name, Placed { name: name.clone(), x, y, cell: Some((col, row)) });
                }
                Some(Pos::Free { x, y }) => {
                    let (x, y) = grid.clamp_free(x, y);
                    placed.insert(name, Placed { name: name.clone(), x, y, cell: None });
                }
                // No place, a cell that is off this grid, or one two icons both claim.
                _ => unplaced.push(name),
            }
        }
        let mut flow = FlowCursor::new(grid);
        for name in unplaced {
            let p = flow.next(grid, &taken);
            let placed_at = match p {
                Pos::Cell { col, row } => {
                    taken.insert((col, row));
                    let (x, y) = grid.origin(col, row);
                    Placed { name: name.to_string(), x, y, cell: Some((col, row)) }
                }
                Pos::Free { x, y } => Placed { name: name.to_string(), x, y, cell: None },
            };
            placed.insert(name, placed_at);
        }
        order.iter().filter_map(|n| placed.remove(n.as_str())).collect()
    }
}

/// Walks the free cells column-first from the top left; when the grid is full, hands out places
/// cascaded from the bottom right, so nothing is lost off screen and every icon is still
/// reachable (the person can drag it to where there is room).
struct FlowCursor {
    next: usize,
    overflow: i32,
}

impl FlowCursor {
    fn new(_grid: &Grid) -> FlowCursor {
        FlowCursor { next: 0, overflow: 0 }
    }

    fn next(&mut self, grid: &Grid, taken: &HashSet<(u32, u32)>) -> Pos {
        while self.next < grid.capacity() {
            let i = self.next as u32;
            self.next += 1;
            let (col, row) = (i / grid.rows, i % grid.rows);
            if !taken.contains(&(col, row)) {
                return Pos::Cell { col, row };
            }
        }
        let (x, y) = grid.origin(grid.cols - 1, grid.rows - 1);
        self.overflow += 1;
        let (x, y) = grid.clamp_free(x - 10 * self.overflow, y - 10 * self.overflow);
        Pos::Free { x, y }
    }
}

/// How Arrange orders the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Name,
    Type,
    /// Newest first.
    Date,
}

/// Every icon's place when the desktop is sorted: column-first from the top left, one after the
/// other. A listing too big for the grid cascades, as a fresh one does.
pub fn arrange(items: &[Item], sort: Sort, grid: &Grid) -> Vec<(String, Pos)> {
    let mut sorted: Vec<&Item> = items.iter().collect();
    let by_name = |a: &&Item, b: &&Item| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.name.cmp(&b.name));
    match sort {
        Sort::Name => sorted.sort_by(by_name),
        Sort::Type => sorted.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| by_name(a, b))),
        Sort::Date => sorted.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| by_name(a, b))),
    }
    let none = HashSet::new();
    let mut flow = FlowCursor::new(grid);
    sorted.into_iter().map(|i| (i.name.clone(), flow.next(grid, &none))).collect()
}

/// Where an icon dropped at `(x, y)` lands. With snapping, the nearest cell that is free (the
/// one under it when nothing else is there); without, exactly where it was let go, kept on the
/// screen.
pub fn drop_place(grid: &Grid, taken: &HashSet<(u32, u32)>, x: i32, y: i32, snap: bool) -> Pos {
    if !snap {
        let (x, y) = grid.clamp_free(x, y);
        return Pos::Free { x, y };
    }
    let (c0, r0) = grid.nearest_cell(x, y);
    let mut best: Option<(u32, u32, u32)> = None;
    for col in 0..grid.cols {
        for row in 0..grid.rows {
            if taken.contains(&(col, row)) {
                continue;
            }
            let d = col.abs_diff(c0) + row.abs_diff(r0);
            if best.map_or(true, |(bd, bc, br)| (d, col, row) < (bd, bc, br)) {
                best = Some((d, col, row));
            }
        }
    }
    match best {
        Some((_, col, row)) => Pos::Cell { col, row },
        // A full grid: keep it where it was let go rather than refuse the move.
        None => {
            let (x, y) = grid.clamp_free(x, y);
            Pos::Free { x, y }
        }
    }
}

/// The cells in use, by the icons that sit in one.
pub fn taken_cells(placed: &[Placed], except: &HashSet<&str>) -> HashSet<(u32, u32)> {
    placed.iter().filter(|p| !except.contains(p.name.as_str())).filter_map(|p| p.cell).collect()
}

/// Where each free-placed icon goes when snapping is turned on: the nearest free cell.
pub fn snap_all(grid: &Grid, placed: &[Placed]) -> Vec<(String, Pos)> {
    let mut taken: HashSet<(u32, u32)> = placed.iter().filter_map(|p| p.cell).collect();
    let mut out = Vec::new();
    for p in placed.iter().filter(|p| p.cell.is_none()) {
        let pos = drop_place(grid, &taken, p.x, p.y, true);
        if let Pos::Cell { col, row } = pos {
            taken.insert((col, row));
        }
        out.push((p.name.clone(), pos));
    }
    out
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
