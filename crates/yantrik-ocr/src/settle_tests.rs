//! A line settles onto its own ink, whatever rectangle the finder gave it.

use super::*;
use crate::memory::Memory;

const WIDTH: u32 = 200;
const HEIGHT: u32 = 120;
const PAPER: [u8; 3] = [29, 31, 33];
const PEN: [u8; 3] = [216, 222, 233];

fn paper() -> Vec<u8> {
    PAPER.iter().copied().cycle().take((WIDTH * HEIGHT * 3) as usize).collect()
}

fn fill(rgb: &mut [u8], x: u32, y: u32, w: u32, h: u32, colour: [u8; 3]) {
    for row in y..y + h {
        for column in x..x + w {
            let at = ((row * WIDTH + column) * 3) as usize;
            rgb[at..at + 3].copy_from_slice(&colour);
        }
    }
}

/// A line of `glyphs` letters, each 6 wide with 2 of air between, `tall` high, with its top
/// left at `x`, `y`. Every letter is a little different, so a line is not its own neighbour.
fn write(rgb: &mut [u8], x: u32, y: u32, glyphs: u32, tall: u32, seed: u8) {
    for n in 0..glyphs {
        fill(rgb, x + n * 8, y, 6, tall, PEN);
        // A notch that depends on the letter and the line.
        let notch = (n as u8).wrapping_mul(3).wrapping_add(seed) % 4;
        fill(rgb, x + n * 8 + 2, y + 2 + notch as u32, 2, 2, PAPER);
    }
}

/// The ink of a 20-letter line at `x`, `y`, 10 high: where it should settle, with its margin.
fn settled(x: u32, y: u32) -> Bounds {
    Bounds { x: x - 2, y: y - 2, w: 20 * 8 - 2 + 4, h: 10 + 4 }
}

#[test]
fn a_line_settles_to_the_same_rectangle_however_the_finder_wobbled() {
    let mut rgb = paper();
    write(&mut rgb, 10, 40, 20, 10, 1);
    for found in [
        Bounds { x: 7, y: 37, w: 164, h: 16 },
        Bounds { x: 8, y: 38, w: 166, h: 15 },
        Bounds { x: 6, y: 36, w: 163, h: 18 },
        // Cut into the letters at the top and the left, and short at the bottom.
        Bounds { x: 12, y: 42, w: 158, h: 7 },
        Bounds { x: 9, y: 35, w: 160, h: 12 },
    ] {
        assert_eq!(settle(&rgb, WIDTH, HEIGHT, &found), settled(10, 40), "from {found:?}");
    }
}

/// The reason for all of this: a terminal that scrolled. The line is the same pixels one row
/// of text higher, and the finder's rectangle around it is not the one it drew before.
#[test]
fn a_line_that_moved_settles_onto_the_same_pixels_and_is_remembered() {
    let (mut before, mut after) = (paper(), paper());
    for (n, seed) in [(0u32, 11u8), (1, 22), (2, 33)] {
        write(&mut before, 10, 30 + n * 17, 20, 10, seed);
        write(&mut after, 10, 13 + n * 17, 20, 10, seed);
    }
    let memory = Memory::default();
    let middle_before = settle(&before, WIDTH, HEIGHT, &Bounds { x: 8, y: 45, w: 163, h: 14 });
    let middle_after = settle(&after, WIDTH, HEIGHT, &Bounds { x: 7, y: 27, w: 166, h: 17 });
    assert_eq!(middle_before, settled(10, 47));
    assert_eq!(middle_after, settled(10, 30));
    assert_eq!(memory.line_key(&before, WIDTH, &middle_before), memory.line_key(&after, WIDTH, &middle_after), "the same pixels, one line higher");
    let other_line = settle(&after, WIDTH, HEIGHT, &Bounds { x: 8, y: 45, w: 163, h: 14 });
    assert_ne!(memory.line_key(&after, WIDTH, &other_line), memory.line_key(&after, WIDTH, &middle_after), "the line below is another line");
}

#[test]
fn the_dot_of_an_i_stays_with_its_line() {
    let mut rgb = paper();
    write(&mut rgb, 10, 44, 20, 6, 1);
    // A dot two rows tall, two blank rows above the letters.
    fill(&mut rgb, 30, 40, 2, 2, PEN);
    let found = Bounds { x: 8, y: 38, w: 163, h: 14 };
    let line = settle(&rgb, WIDTH, HEIGHT, &found);
    assert_eq!((line.y, line.y + line.h), (38, 52), "from two above the dot to two below the letters");
}

#[test]
fn the_tips_of_the_next_lines_letters_are_not_taken_for_a_speck() {
    let mut rgb = paper();
    write(&mut rgb, 10, 40, 20, 10, 1);
    // The next line starts three rows below, and the finder's rectangle reaches two rows into it.
    write(&mut rgb, 10, 53, 20, 10, 2);
    let found = Bounds { x: 8, y: 38, w: 163, h: 17 };
    let line = settle(&rgb, WIDTH, HEIGHT, &found);
    assert_eq!((line.y, line.y + line.h), (38, 52), "the line's own ten rows and the air around them");
}

#[test]
fn the_margin_is_only_ever_air() {
    let mut rgb = paper();
    write(&mut rgb, 10, 40, 20, 10, 1);
    // Another line one row below: there is one row of air under this one, not two.
    write(&mut rgb, 10, 51, 20, 10, 2);
    let line = settle(&rgb, WIDTH, HEIGHT, &Bounds { x: 8, y: 38, w: 163, h: 13 });
    assert_eq!(line.y + line.h, 51, "the margin stops at the next line's ink");
    assert_eq!(line.y, 38);
}

#[test]
fn text_over_a_picture_keeps_the_rectangle_the_finder_gave_it() {
    // No two neighbouring pixels alike: nothing here is paper, so the ink has no edge.
    let rgb: Vec<u8> = (0..WIDTH * HEIGHT * 3).map(|n| (n.wrapping_mul(2654435761) >> 13) as u8).collect();
    let found = Bounds { x: 20, y: 30, w: 100, h: 16 };
    assert_eq!(settle(&rgb, WIDTH, HEIGHT, &found), found);
}

#[test]
fn a_rectangle_with_nothing_in_it_is_left_alone() {
    let rgb = paper();
    let found = Bounds { x: 20, y: 30, w: 100, h: 16 };
    assert_eq!(settle(&rgb, WIDTH, HEIGHT, &found), found);
    let empty = Bounds { x: 20, y: 30, w: 0, h: 0 };
    assert_eq!(settle(&rgb, WIDTH, HEIGHT, &empty), empty);
    let outside = Bounds { x: WIDTH - 10, y: 30, w: 40, h: 16 };
    assert_eq!(settle(&rgb, WIDTH, HEIGHT, &outside), outside, "a rectangle that is not in the picture is not read from it");
}

#[test]
fn a_line_at_the_edge_of_the_picture_settles_without_reaching_outside() {
    let mut rgb = paper();
    write(&mut rgb, 0, 0, 20, 10, 1);
    let line = settle(&rgb, WIDTH, HEIGHT, &Bounds { x: 0, y: 0, w: 162, h: 13 });
    assert_eq!(line, Bounds { x: 0, y: 0, w: 20 * 8 - 2 + 2, h: 12 }, "air only on the sides that have any");
    let mut last = paper();
    write(&mut last, 10, HEIGHT - 10, 20, 10, 1);
    let line = settle(&last, WIDTH, HEIGHT, &Bounds { x: 8, y: HEIGHT - 13, w: 163, h: 13 });
    assert_eq!((line.y, line.y + line.h), (HEIGHT - 12, HEIGHT));
}

#[test]
fn a_line_that_runs_into_something_wider_keeps_the_found_rectangle() {
    let mut rgb = paper();
    // A rule right across the picture through the middle of the letters: the line has no left
    // or right edge of its own.
    write(&mut rgb, 30, 40, 10, 10, 1);
    fill(&mut rgb, 0, 44, WIDTH, 2, PEN);
    let found = Bounds { x: 28, y: 38, w: 82, h: 14 };
    assert_eq!(settle(&rgb, WIDTH, HEIGHT, &found), found);
}
