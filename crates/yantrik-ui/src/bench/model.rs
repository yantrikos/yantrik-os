//! The Bench's state, with no Slint in it: what is on the desktop, where each icon sits, what is
//! selected. Every rule the pointer and the keys follow is a method here, so each can be tested
//! by calling it. `view` turns this into what the screen draws; `wiring` turns the screen's
//! callbacks into calls on this.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use yantrik_bench::items::{self, display_name, flow_order, is_plain_name, Item, Kind};
use yantrik_bench::layout::{self, Grid, Placed, Pos, Sort, Store, METRICS};

use super::persist::Saver;

/// Which way an arrow key moves the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("`{0}` is not a name a thing on the desktop can have")]
    BadName(String),
    #[error("there is nothing called `{0}` on the desktop")]
    NotThere(String),
    #[error("`{0}` is already on the desktop; nothing was replaced")]
    Taken(String),
    #[error("{0}")]
    Io(String),
}

pub struct Model {
    pub desktop: PathBuf,
    pub listing: Vec<Item>,
    /// Entries past the cap that were left out.
    pub more: usize,
    /// Why the desktop folder cannot be listed, if it cannot.
    pub problem: Option<String>,
    pub store: Store,
    saver: Saver,
    /// The icon area: width, height, and what the Minds panel covers at the right.
    area: Option<(i32, i32, i32)>,
    pub placed: Vec<Placed>,
    pub selection: BTreeSet<String>,
    anchor: Option<String>,
    pub renaming: Option<String>,
}

impl Model {
    pub fn new(desktop: PathBuf, store: Store, saver: Saver) -> Model {
        Model {
            desktop,
            listing: Vec::new(),
            more: 0,
            problem: None,
            store,
            saver,
            area: None,
            placed: Vec::new(),
            selection: BTreeSet::new(),
            anchor: None,
            renaming: None,
        }
    }

    pub fn grid(&self) -> Option<Grid> {
        self.area.map(|(w, h, reserve)| Grid::new(w, h, reserve, METRICS))
    }

    /// The icon area was drawn at this size. True when the icons moved because of it.
    pub fn set_area(&mut self, w: i32, h: i32, reserve: i32) -> bool {
        if self.area == Some((w, h, reserve)) || w <= 0 || h <= 0 {
            return false;
        }
        self.area = Some((w, h, reserve));
        self.relayout();
        true
    }

    /// Read the desktop folder again. True when what is listed changed.
    pub fn rescan(&mut self) -> bool {
        self.apply_listing(items::scan(&self.desktop))
    }

    /// Take a listing read elsewhere (the worker, or `rescan`). True when what is listed changed.
    pub fn apply_listing(&mut self, read: Result<items::Listing, items::ItemsError>) -> bool {
        let before = self.listing.clone();
        match read {
            Ok(l) => {
                self.listing = l.items;
                self.more = l.more;
                self.problem = None;
            }
            Err(e) => {
                self.listing.clear();
                self.more = 0;
                self.problem = Some(e.to_string());
            }
        }
        let here: HashSet<&str> = self.listing.iter().map(|i| i.name.as_str()).collect();
        self.selection.retain(|n| here.contains(n.as_str()));
        if self.renaming.as_ref().is_some_and(|n| !here.contains(n.as_str())) {
            self.renaming = None;
        }
        // A file's place is forgotten only when the folder has been read and it is not there.
        let pruned = self.problem.is_none() && self.store.drop_missing(&here);
        self.relayout();
        if pruned {
            self.save();
        }
        before != self.listing
    }

    /// Work out every icon's place from the store and the listing, and fix the place of any
    /// newcomer so the next arrival cannot move it.
    pub fn relayout(&mut self) {
        let Some(grid) = self.grid() else {
            self.placed.clear();
            return;
        };
        self.placed = self.store.resolve(&grid, &flow_order(&self.listing));
        if self.store.adopt(&grid.key(), &self.placed) {
            self.save();
        }
    }

    /// "Show desktop icons". Off hides the icons, not the files, and is remembered.
    pub fn show_icons(&self) -> bool {
        self.store.icons
    }

    pub fn set_show_icons(&mut self, on: bool) {
        if self.store.icons != on {
            self.store.icons = on;
            self.save();
        }
    }

    pub fn keep_recent(&self, record: &yantrik_bench::recent::LastOpened) {
        self.saver.keep_recent(record);
    }

    pub fn save(&mut self) {
        self.saver.save(&mut self.store);
    }

    pub fn item(&self, name: &str) -> Option<&Item> {
        self.listing.iter().find(|i| i.name == name)
    }

    pub fn placed_at(&self, name: &str) -> Option<&Placed> {
        self.placed.iter().find(|p| p.name == name)
    }

    pub fn path_of(&self, name: &str) -> PathBuf {
        self.desktop.join(name)
    }

    /// The name as a caller (a mind) may give it: the real one, or the one drawn when no other
    /// thing draws the same.
    pub fn find(&self, said: &str) -> Option<&Item> {
        self.item(said).or_else(|| {
            let mut same = self.listing.iter().filter(|i| display_name(&i.name) == said);
            match (same.next(), same.next()) {
                (Some(one), None) => Some(one),
                _ => None,
            }
        })
    }

    // ── Selection ──

    /// A press on an icon. Ctrl toggles it; without Ctrl, an icon already in the selection keeps
    /// the selection (so a group can be dragged), and any other becomes the only one.
    pub fn press(&mut self, name: &str, ctrl: bool) {
        if self.item(name).is_none() {
            return;
        }
        if ctrl {
            if !self.selection.remove(name) {
                self.selection.insert(name.to_string());
            }
        } else if !self.selection.contains(name) {
            self.selection.clear();
            self.selection.insert(name.to_string());
        }
        self.anchor = Some(name.to_string());
    }

    /// The press ended without a drag: a plain click on one of several selected icons selects
    /// just that one, as on every desktop.
    pub fn click_release(&mut self, name: &str, ctrl: bool) {
        if !ctrl && self.selection.contains(name) && self.selection.len() > 1 {
            self.selection.clear();
            self.selection.insert(name.to_string());
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    pub fn select_all(&mut self) {
        self.selection = self.listing.iter().map(|i| i.name.clone()).collect();
    }

    /// Everything the band crosses becomes the selection (Ctrl: is added to it).
    pub fn select_rect(&mut self, x: i32, y: i32, w: i32, h: i32, add: bool) {
        let hit: Vec<String> = self
            .placed
            .iter()
            .filter(|p| {
                // The icon's own box: the tile and its label, not the whole cell.
                let (ix, iy, iw, ih) = (p.x + 8, p.y + 4, METRICS.cell_w - 16, METRICS.cell_h - 8);
                x < ix + iw && x + w > ix && y < iy + ih && y + h > iy
            })
            .map(|p| p.name.clone())
            .collect();
        if !add {
            self.selection.clear();
        }
        self.selection.extend(hit);
    }

    /// An arrow key: the selection moves to the nearest icon in that direction (from the last one
    /// touched). With nothing selected, the first icon is selected. `extend` (Shift) adds it.
    pub fn arrow(&mut self, dir: Dir, extend: bool) {
        if self.placed.is_empty() {
            return;
        }
        let from = self.anchor.clone().filter(|a| self.selection.contains(a)).or_else(|| self.selection.iter().next().cloned());
        let Some(from) = from.and_then(|n| self.placed_at(&n).cloned()) else {
            let first = self.placed.iter().min_by_key(|p| (p.x, p.y)).map(|p| p.name.clone());
            if let Some(first) = first {
                self.selection.clear();
                self.selection.insert(first.clone());
                self.anchor = Some(first);
            }
            return;
        };
        let (fx, fy) = (from.x, from.y);
        let next = self
            .placed
            .iter()
            .filter(|p| p.name != from.name)
            .filter_map(|p| {
                let (dx, dy) = (p.x - fx, p.y - fy);
                // Along the direction, and how far off the line it is.
                let (along, off) = match dir {
                    Dir::Up => (-dy, dx.abs()),
                    Dir::Down => (dy, dx.abs()),
                    Dir::Left => (-dx, dy.abs()),
                    Dir::Right => (dx, dy.abs()),
                };
                (along > 0).then_some((off * 2 + along, p.name.clone()))
            })
            .min_by_key(|(score, _)| *score)
            .map(|(_, n)| n);
        if let Some(next) = next {
            if !extend {
                self.selection.clear();
            }
            self.selection.insert(next.clone());
            self.anchor = Some(next);
        }
    }

    // ── Placement ──

    /// The selected icons were dragged by `(dx, dy)` pixels. Each lands where it was let go, or on
    /// the nearest free cell when the desktop snaps; none takes a cell another icon holds.
    pub fn move_selected(&mut self, dx: i32, dy: i32) -> bool {
        let Some(grid) = self.grid() else { return false };
        let moving: Vec<Placed> = self.placed.iter().filter(|p| self.selection.contains(&p.name)).cloned().collect();
        if moving.is_empty() || (dx == 0 && dy == 0) {
            return false;
        }
        let names: HashSet<&str> = moving.iter().map(|p| p.name.as_str()).collect();
        let mut taken = layout::taken_cells(&self.placed, &names);
        let key = grid.key();
        for p in &moving {
            let pos = layout::drop_place(&grid, &taken, p.x + dx, p.y + dy, self.store.snap);
            if let Pos::Cell { col, row } = pos {
                taken.insert((col, row));
            }
            self.store.set(&key, &p.name, pos);
        }
        self.relayout();
        self.save();
        true
    }

    /// Put one icon in a cell (a mind's `desktop_place`). The cell must be on the grid and free.
    pub fn place_in_cell(&mut self, name: &str, col: u32, row: u32) -> Result<(), ModelError> {
        let grid = self.grid().ok_or_else(|| ModelError::Io("the desktop has not been drawn yet".into()))?;
        if self.item(name).is_none() {
            return Err(ModelError::NotThere(name.to_string()));
        }
        if col >= grid.cols || row >= grid.rows {
            return Err(ModelError::Io(format!(
                "cell ({col}, {row}) is off the desktop, which is {} columns by {} rows",
                grid.cols, grid.rows
            )));
        }
        if let Some(other) = self.placed.iter().find(|p| p.cell == Some((col, row)) && p.name != name) {
            return Err(ModelError::Io(format!("cell ({col}, {row}) holds `{}`; nothing was moved", display_name(&other.name))));
        }
        self.store.set(&grid.key(), name, Pos::Cell { col, row });
        self.relayout();
        self.save();
        Ok(())
    }

    /// Arrange: sort the whole desktop, column-first from the top left.
    pub fn sort(&mut self, how: Sort) {
        let Some(grid) = self.grid() else { return };
        let key = grid.key();
        for (name, pos) in layout::arrange(&self.listing, how, &grid) {
            self.store.set(&key, &name, pos);
        }
        self.relayout();
        self.save();
    }

    /// Snap to the grid on or off. Turning it on moves every free icon to the nearest free cell.
    pub fn set_snap(&mut self, on: bool) {
        if self.store.snap == on {
            return;
        }
        self.store.snap = on;
        if let (true, Some(grid)) = (on, self.grid()) {
            for (name, pos) in layout::snap_all(&grid, &self.placed) {
                self.store.set(&grid.key(), &name, pos);
            }
        }
        self.relayout();
        self.save();
    }

    // ── Names ──

    /// Rename one thing on the desktop, never onto another. Its place goes with it.
    pub fn rename(&mut self, old: &str, new: &str) -> Result<(), ModelError> {
        let new = new.trim();
        if self.item(old).is_none() {
            return Err(ModelError::NotThere(old.to_string()));
        }
        // Not a hidden name (it would leave the desktop), and no control characters (a name that
        // draws as something else).
        if !is_plain_name(new) || new.starts_with('.') || new.chars().any(char::is_control) {
            return Err(ModelError::BadName(new.to_string()));
        }
        if new == old {
            return Ok(());
        }
        if std::fs::symlink_metadata(self.path_of(new)).is_ok() {
            return Err(ModelError::Taken(new.to_string()));
        }
        crate::fileops::rename_no_replace(&self.path_of(old), &self.path_of(new)).map_err(ModelError::Io)?;
        self.store.rename(old, new);
        if self.selection.remove(old) {
            self.selection.insert(new.to_string());
        }
        self.renaming = None;
        self.rescan();
        self.save();
        Ok(())
    }

    /// The first of `base`, `base 2`, `base 3` … (with `ext` kept after the number) that is free.
    pub fn free_name(&self, base: &str, ext: &str) -> String {
        for n in 1..1000 {
            let name = if n == 1 { format!("{base}{ext}") } else { format!("{base} {n}{ext}") };
            if std::fs::symlink_metadata(self.path_of(&name)).is_err() {
                return name;
            }
        }
        format!("{base} {}{ext}", std::process::id())
    }

    /// The kinds of what is selected, for the menu.
    pub fn selected_items(&self) -> Vec<&Item> {
        self.listing.iter().filter(|i| self.selection.contains(&i.name)).collect()
    }

    pub fn is_app(&self, name: &str) -> bool {
        self.item(name).is_some_and(|i| i.kind == Kind::App)
    }
}

/// `~/Desktop` as a person reads it: under the home, `~/…`.
pub fn display_path(home: &Path, dir: &Path) -> String {
    match dir.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => dir.display().to_string(),
    }
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
