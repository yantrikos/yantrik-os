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
//! run of rows that have ink in them around the line's densest row, and the columns from the
//! first glyph to the last, with a margin of blank pixels. That rectangle depends on the
//! picture and not on the finder's mood, and it is the one both the memory and the engine are
//! given. A line that has only moved settles onto the same pixels wherever it is.
//!
//! When the ink does not say (text over a photograph, a line that runs into its neighbours),
//! the found rectangle is kept as it is. Nothing is lost but the remembering.

use std::collections::HashMap;

use crate::read::Bounds;

/// How far past the found rectangle the ink is followed, in pixels. A finder that cut the
/// tails off a line's letters is put right; a line that goes on further than this has no edge
/// of its own.
const GROW: u32 = 6;

/// Blank pixels kept around the ink. The engine reads a line with a little air around it
/// better than one cut to the glyphs.
const MARGIN: u32 = 2;

/// The tallest run of rows that counts as a speck (the dot of an i, an accent, an underscore),
/// and the widest gap it may stand off from its line by.
const SPECK: u32 = 3;

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

/// Settle a found rectangle onto the line's own ink, in an RGB image `width` × `height`.
pub fn settle(rgb: &[u8], width: u32, height: u32, found: &Bounds) -> Bounds {
    if found.w == 0 || found.h == 0 || found.x + found.w > width || found.y + found.h > height {
        return *found;
    }
    let (x0, x1) = (found.x, found.x + found.w);
    let (leftmost, rightmost) = (found.x.saturating_sub(GROW), (x1 + GROW).min(width));
    let (above, below) = (found.y.saturating_sub(GROW), (found.y + found.h + GROW).min(height));
    let around = Bounds { x: leftmost, y: above, w: rightmost - leftmost, h: below - above };
    let Some(background) = background(rgb, width, &around) else { return *found };
    let ink = Ink { rgb, width, background };

    // The row with the most ink is in the body of the letters, wherever the finder put its
    // edges. The line is the run of inked rows around it.
    let rows: Vec<u32> = (above..below).map(|y| ink.in_row(y, x0, x1)).collect();
    let row = |y: u32| rows[(y - above) as usize];
    let Some(densest) = (found.y..found.y + found.h).max_by_key(|y| (row(*y), std::cmp::Reverse(*y))).filter(|y| row(*y) > 0) else {
        return *found;
    };
    let (mut top, mut bottom) = (densest, densest + 1);
    while top > above && row(top - 1) > 0 {
        top -= 1;
    }
    while bottom < below && row(bottom) > 0 {
        bottom += 1;
    }
    // Ink all the way to the edge of where we looked: the line has no edge of its own there,
    // unless that edge is the edge of the picture.
    if (top == above && above > 0 && row(top) > 0) || (bottom == below && below < height && row(bottom - 1) > 0) {
        return *found;
    }
    // A speck just above or below, inside what the finder found, belongs to the line: the dot
    // of an i on a line with no tall letter stands off from it by a blank row or two. It is
    // measured as far as it goes, not only as far as the finder's edge, so the tips of the
    // next line's letters are not mistaken for one.
    let speck = |from: u32, towards_top: bool| -> Option<u32> {
        let step = |y: u32| if towards_top { y.checked_sub(1).filter(|y| *y >= above) } else { Some(y + 1).filter(|y| *y < below) };
        let within = |y: &u32| (found.y..found.y + found.h).contains(y);
        let mut y = from;
        for _ in 0..=SPECK {
            y = step(y).filter(within)?;
            if row(y) > 0 {
                let mut end = y;
                let mut tall = 1;
                while let Some(next) = step(end).filter(|next| row(*next) > 0) {
                    end = next;
                    tall += 1;
                }
                return (tall <= SPECK && within(&end)).then_some(end);
            }
        }
        None
    };
    if let Some(up) = speck(top, true) {
        top = up;
    }
    if let Some(down) = speck(bottom - 1, false) {
        bottom = down + 1;
    }

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
