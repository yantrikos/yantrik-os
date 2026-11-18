//! What the reader remembers between looks, so it reads only what it has not read (#1256).
//!
//! A screen changes a little at a time: a terminal gains a line, a clock ticks, a window comes
//! to the front. Reading every line again on every look is nearly all wasted work, because
//! reading a line is the expensive half and most lines are the ones that were there before. Two
//! things are remembered, and both are exact:
//!
//! - **The last image.** A look at a display that has not changed at all is answered with the
//!   last answer, without the models running.
//! - **Each line, by its pixels.** A line whose rectangle holds exactly the pixels it held when
//!   it was last read says what it said then. That covers a line that has not moved, and a
//!   line that moved whole (a terminal that scrolled) wherever the finder puts the same
//!   rectangle around it.
//!
//! "Exactly" is the point. A remembered answer is given only for identical pixels, so memory
//! can make a reading faster and cannot make it say something the screen does not show.
//!
//! # Not a way to learn what was on another display
//!
//! Each display has a memory of its own ([`Memory::space`]). A reading of Mind View is never
//! made faster by what was read on the person's desktop, so how long it took says nothing
//! about the desktop.
//!
//! # Not a way to be told something that is not there
//!
//! Lines are matched by two independent keyed 64-bit hashes of their pixels (the rectangle's
//! size is part of both), each with a key made afresh for every process, and both must match.
//! A program that draws pixels cannot work out which pixels would be mistaken for a line that
//! was on screen earlier.
//!
//! # Bounded
//!
//! A space keeps at most [`MAX_LINES`] lines and there are at most [`MAX_SPACES`] spaces. Lines
//! not seen for [`KEPT_FOR`] looks are forgotten first.

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};

use crate::read::{Bounds, Line};

/// Lines remembered for one display at most.
pub const MAX_LINES: usize = 4096;

/// Displays remembered at most.
pub const MAX_SPACES: usize = 8;

/// Looks a line is kept for without being seen, once the space is full.
pub const KEPT_FOR: u64 = 32;

/// What pixels hash to: two independent 64-bit keyed hashes, both of which must agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key(u64, u64);

/// Two hashers fed the same bytes.
struct Twin(<RandomState as BuildHasher>::Hasher, <RandomState as BuildHasher>::Hasher);

impl Twin {
    fn write(&mut self, bytes: &[u8]) {
        self.0.write(bytes);
        self.1.write(bytes);
    }

    fn write_u32(&mut self, n: u32) {
        self.write(&n.to_le_bytes());
    }

    fn finish(self) -> Key {
        Key(self.0.finish(), self.1.finish())
    }
}

/// What a rectangle of pixels said when it was read: its text and where its characters were
/// inside the rectangle, or `None` for a rectangle that held nothing worth reporting (an icon,
/// a rule, a scroll bar). Remembering "nothing" matters as much as remembering text: a screen
/// is full of such rectangles, and each would be read again on every look.
#[derive(Debug, Clone, PartialEq)]
struct Said {
    line: Option<Inside>,
    seen: u64,
}

/// A line's text, and its characters' rectangle relative to the rectangle it was found in.
#[derive(Debug, Clone, PartialEq)]
struct Inside {
    text: String,
    dx: i32,
    dy: i32,
    w: i32,
    h: i32,
}

/// One display's memory.
#[derive(Default)]
struct Space {
    lines: HashMap<Key, Said>,
    /// The last image read here, and what it said.
    last: Option<(Key, Vec<Line>)>,
    looks: u64,
}

/// Everything the reader remembers.
pub struct Memory {
    keys: [RandomState; 2],
    spaces: HashMap<String, Space>,
}

impl Default for Memory {
    fn default() -> Self {
        Self { keys: [RandomState::new(), RandomState::new()], spaces: HashMap::new() }
    }
}

impl Memory {
    fn twin(&self) -> Twin {
        Twin(self.keys[0].build_hasher(), self.keys[1].build_hasher())
    }

    /// The key of a whole image.
    pub fn image_key(&self, rgb: &[u8], width: u32, height: u32) -> Key {
        let mut hasher = self.twin();
        hasher.write_u32(width);
        hasher.write_u32(height);
        hasher.write(rgb);
        hasher.finish()
    }

    /// The key of the pixels inside `bounds` of an RGB image `width` wide. An image too short
    /// for the rectangle is hashed as far as it goes; the rectangle's size is part of the key.
    pub fn line_key(&self, rgb: &[u8], width: u32, bounds: &Bounds) -> Key {
        let mut hasher = self.twin();
        hasher.write_u32(bounds.w);
        hasher.write_u32(bounds.h);
        let (x0, x1) = (bounds.x as usize * 3, (bounds.x + bounds.w).min(width) as usize * 3);
        for y in bounds.y..bounds.y + bounds.h {
            let row = y as usize * width as usize * 3;
            match rgb.get(row + x0..row + x1) {
                Some(pixels) => hasher.write(pixels),
                None => break,
            }
        }
        hasher.finish()
    }

    /// Begin a look at `space`: the display's memory, made if this is its first look. When
    /// there are already [`MAX_SPACES`] others, the one looked at least is forgotten.
    fn space(&mut self, space: &str) -> &mut Space {
        if !self.spaces.contains_key(space) && self.spaces.len() >= MAX_SPACES {
            let least = self.spaces.iter().min_by_key(|(_, s)| s.looks).map(|(name, _)| name.clone());
            if let Some(least) = least {
                self.spaces.remove(&least);
            }
        }
        self.spaces.entry(space.to_string()).or_default()
    }

    /// What the last look at `space` said, when `image` is the very image it looked at.
    pub fn same_image(&mut self, space: &str, image: Key) -> Option<Vec<Line>> {
        let space = self.space(space);
        space.looks += 1;
        space.last.as_ref().filter(|(last, _)| *last == image).map(|(_, lines)| lines.clone())
    }

    /// What the pixels with this key said when they were last read in `space`, placed where
    /// they are now. `None`: never read. `Some(None)`: read, and nothing worth reporting.
    pub fn recall(&mut self, space: &str, key: Key, bounds: &Bounds) -> Option<Option<Line>> {
        let space = self.space(space);
        let looks = space.looks;
        let said = space.lines.get_mut(&key)?;
        said.seen = looks;
        Some(said.line.as_ref().map(|inside| Line {
            text: inside.text.clone(),
            x: bounds.x as i32 + inside.dx,
            y: bounds.y as i32 + inside.dy,
            w: inside.w,
            h: inside.h,
        }))
    }

    /// Remember what the pixels with this key said, found at `bounds`.
    pub fn keep(&mut self, space: &str, key: Key, bounds: &Bounds, line: Option<&Line>) {
        let space = self.space(space);
        if space.lines.len() >= MAX_LINES {
            let looks = space.looks;
            space.lines.retain(|_, said| looks.saturating_sub(said.seen) < KEPT_FOR);
            if space.lines.len() >= MAX_LINES {
                // Thousands of lines all seen lately: a display that never repeats itself.
                // Nothing here is worth more than anything else, so start again.
                space.lines.clear();
            }
        }
        let line = line.map(|line| Inside { text: line.text.clone(), dx: line.x - bounds.x as i32, dy: line.y - bounds.y as i32, w: line.w, h: line.h });
        let seen = space.looks;
        space.lines.insert(key, Said { line, seen });
    }

    /// Remember the whole image a look at `space` just read, and what it said.
    pub fn keep_image(&mut self, space: &str, image: Key, lines: &[Line]) {
        self.space(space).last = Some((image, lines.to_vec()));
    }

    /// Forget everything: what was on every display is no longer held anywhere in this process.
    pub fn forget(&mut self) {
        self.spaces.clear();
    }
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
