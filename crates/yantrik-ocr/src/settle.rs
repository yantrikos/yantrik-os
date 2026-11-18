//! Where a line really is: the rectangle its own ink says, not the one the finder guessed.
//!
//! The finder puts a rectangle around each line, and it is never quite the same rectangle
//! twice. The same line of a terminal, one row higher after a scroll, comes back two pixels
//! wider or a pixel lower. Measured on a 40-line terminal that scrolled by one line: 3 of the
//! 39 lines that had only moved were given the rectangle they had before.
//!
//! That matters twice. Memory matches a line by the pixels in its rectangle (`memory`), so a
//! rectangle that wobbles is a line read again for nothing. And what the engine reads depends
//! on the rectangle it is handed, so the same line could read differently from one look to the
//! next.
//!
//! So each found rectangle is settled onto the ink before anything else is done with it: the
//! tallest run of rows that have ink in them (with the specks that belong to it: the dot of an
//! i, an accent, an underline), and the columns from the first glyph to the last, with a margin
//! of blank pixels. That rectangle depends on the picture and not on the finder's mood, and it
//! is the one both the memory and the engine are given. A line that has only moved settles onto
//! the same pixels wherever it is.
//!
//! When the ink does not say (text over a photograph, a line that runs into its neighbours, two
//! lines in one rectangle, a rectangle taller than a line), the found rectangle is kept as it is.
//! A rule under a line, or a field's border, is a run of its own and stays with the line.

use std::collections::HashMap;
use std::ops::Range;

use crate::read::Bounds;

/// How far past the found rectangle the ink is followed, in pixels. A finder that cut the
/// tails off a line's letters is put right; a line that goes on further than this has no edge
/// of its own.
const GROW: u32 = 6;

/// Blank pixels kept around the ink. The engine reads a line with a little air around it
/// better than one cut to the glyphs.
const MARGIN: u32 = 2;

/// The least a speck may be, in rows: the dot of an i, an accent, an underline. It is also how
/// far it may stand off from its line. Both grow with the line, to a third of its height, so
/// the dots of large type are not cut off.
const SPECK: u32 = 3;

/// A found rectangle taller than this is no line: it is kept as it is, and not searched.
const MOST_ROWS: u32 = 200;

/// The picture, and which of its pixels are ink for the line being settled.
struct Ink<'a> {
    rgb: &'a [u8],
    width: u32,
    background: [u8; 3],
}

impl Ink<'_> {
    fn at(&self, x: u32, y: u32) -> bool {
        let at = (y as usize * self.width as usize + x as usize) * 3;
        self.rgb.get(at..at + 3).is_some_and(|pixel| pixel != self.background)
    }

    fn in_row(&self, y: u32, x0: u32, x1: u32) -> u32 {
        (x0..x1).filter(|x| self.at(*x, y)).count() as u32
    }

    fn in_column(&self, x: u32, y0: u32, y1: u32) -> bool {
        (y0..y1).any(|y| self.at(x, y))
    }
}

/// The colour most of the rectangle is: what the line is written on. `around` is the found
/// rectangle with the space about it, where a line's own paper shows: inside the finder's
/// rectangle alone, heavy type can be more ink than paper.
fn background(rgb: &[u8], width: u32, around: &Bounds) -> Option<[u8; 3]> {
    let mut seen: HashMap<[u8; 3], u32> = HashMap::new();
    for y in around.y..around.y + around.h {
        let row = (y as usize * width as usize + around.x as usize) * 3;
        for pixel in rgb.get(row..row + around.w as usize * 3)?.chunks_exact(3) {
            *seen.entry([pixel[0], pixel[1], pixel[2]]).or_default() += 1;
        }
    }
    seen.into_iter().max_by_key(|(colour, count)| (*count, *colour)).map(|(colour, _)| colour)
}

/// The rows of the line, given how much ink each row from `above` has: `None` when the ink
/// does not say where the line is. `open_above` and `open_below` say whether the rows looked at
/// end short of the picture's own edge.
fn line_rows(rows: &[u32], above: u32, found: Range<u32>, open_above: bool, open_below: bool) -> Option<(u32, u32)> {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    let mut start = None;
    for (n, count) in rows.iter().copied().chain(std::iter::once(0)).enumerate() {
        let y = above + n as u32;
        match (start, count > 0) {
            (None, true) => start = Some(y),
            (Some(from), false) => {
                runs.push((from, y));
                start = None;
            }
            _ => {}
        }
    }
    let inside = |run: &(u32, u32)| run.1.min(found.end).saturating_sub(run.0.max(found.start));
    let (chosen, &(top, bottom)) = runs.iter().enumerate().filter(|(_, run)| inside(run) > 0).max_by_key(|(_, run)| (inside(run), std::cmp::Reverse(run.0)))?;
    // Ink all the way to the edge of where we looked: the line has no edge of its own there,
    // unless that edge is the edge of the picture.
    if (top == above && open_above) || (bottom == above + rows.len() as u32 && open_below) {
        return None;
    }
    // A speck (the dot of an i, an accent, an underline) is as tall, and may stand as far off,
    // as a third of the line; never less than SPECK rows.
    let speck = SPECK.max((bottom - top) / 3);
    let (mut line_top, mut line_bottom) = (top, bottom);
    let (mut nearest_above, mut nearest_below) = (None, None);
    for run in runs.iter().enumerate().filter(|(n, _)| *n != chosen).map(|(_, run)| run) {
        // Another run as tall as a speck inside the found rectangle is another line: the ink
        // does not say which of them this rectangle is for.
        if inside(run) > speck {
            return None;
        }
        // A speck belongs to the line only if it lies wholly inside what the finder found, so
        // the tips of the next line's letters are not taken for one.
        if run.0 < found.start || run.1 > found.end || run.1 - run.0 > speck {
            continue;
        }
        if run.1 <= top && top - run.1 <= speck && nearest_above.is_none_or(|(_, end)| run.1 > end) {
            nearest_above = Some((run.0, run.1));
        }
        if run.0 >= bottom && run.0 - bottom <= speck && nearest_below.is_none_or(|(start, _)| run.0 < start) {
            nearest_below = Some((run.0, run.1));
        }
    }
    if let Some((from, _)) = nearest_above {
        line_top = from;
    }
    if let Some((_, to)) = nearest_below {
        line_bottom = to;
    }
    Some((line_top, line_bottom))
}

/// Settle a found rectangle onto the line's own ink, in an RGB image `width` × `height`.
pub fn settle(rgb: &[u8], width: u32, height: u32, found: &Bounds) -> Bounds {
    if found.w == 0 || found.h == 0 || found.h > MOST_ROWS || found.x + found.w > width || found.y + found.h > height {
        return *found;
    }
    let (x0, x1) = (found.x, found.x + found.w);
    let (leftmost, rightmost) = (found.x.saturating_sub(GROW), (x1 + GROW).min(width));
    let (above, below) = (found.y.saturating_sub(GROW), (found.y + found.h + GROW).min(height));
    let around = Bounds { x: leftmost, y: above, w: rightmost - leftmost, h: below - above };
    let Some(background) = background(rgb, width, &around) else { return *found };
    let ink = Ink { rgb, width, background };

    // The line is the tallest run of inked rows inside the found rectangle, with the specks
    // that belong to it. A rule under the text, or a field's border, is a run of its own.
    let rows: Vec<u32> = (above..below).map(|y| ink.in_row(y, x0, x1)).collect();
    let Some((mut top, mut bottom)) = line_rows(&rows, above, found.y..found.y + found.h, above > 0, below < height) else {
        return *found;
    };

    // Left and right: from the finder's edge to the nearest edge of ink, outwards if the edge
    // fell inside a glyph and inwards if it fell in the air.
    let column = |x: u32| ink.in_column(x, top, bottom);
    let mut left = x0;
    if column(left) {
        while left > leftmost && column(left - 1) {
            left -= 1;
        }
        if left > 0 && column(left - 1) {
            // Still ink where we stopped looking: the line runs on into something else.
            return *found;
        }
    } else {
        while left < x1 && !column(left) {
            left += 1;
        }
    }
    let mut right = x1;
    if column(right - 1) {
        while right < rightmost && column(right) {
            right += 1;
        }
        if right < width && column(right) {
            return *found;
        }
    } else {
        while right > left && !column(right - 1) {
            right -= 1;
        }
    }
    if left >= right {
        return *found;
    }

    // A little air around the ink, as far as it really is air.
    let blank_row = |y: u32| ink.in_row(y, left, right) == 0;
    for _ in 0..MARGIN {
        if top > 0 && blank_row(top - 1) {
            top -= 1;
        }
        if bottom < height && blank_row(bottom) {
            bottom += 1;
        }
    }
    for _ in 0..MARGIN {
        if left > 0 && !ink.in_column(left - 1, top, bottom) {
            left -= 1;
        }
        if right < width && !ink.in_column(right, top, bottom) {
            right += 1;
        }
    }
    Bounds { x: left, y: top, w: right - left, h: bottom - top }
}

#[cfg(test)]
#[path = "settle_tests.rs"]
mod tests;
