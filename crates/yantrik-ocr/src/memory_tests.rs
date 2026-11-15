//! What the reader's memory gives back, and what it must never give back.

use super::*;

const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;

/// A blank image with a patch of "ink" whose colour depends on `ink`, at `at`.
fn image(patches: &[(Bounds, u8)]) -> Vec<u8> {
    let mut rgb = vec![20u8; (WIDTH * HEIGHT * 3) as usize];
    for (bounds, ink) in patches {
        for y in bounds.y..bounds.y + bounds.h {
            for x in bounds.x..bounds.x + bounds.w {
                let at = ((y * WIDTH + x) * 3) as usize;
                // Not uniform, so a patch moved by a pixel is a different patch.
                rgb[at] = ink.wrapping_add((x - bounds.x) as u8).wrapping_mul(7);
                rgb[at + 1] = ink.wrapping_add((y - bounds.y) as u8);
                rgb[at + 2] = *ink;
            }
        }
    }
    rgb
}

fn at(x: u32, y: u32) -> Bounds {
    Bounds { x, y, w: 30, h: 8 }
}

fn line(text: &str, bounds: &Bounds) -> Line {
    // The characters sit a pixel inside the rectangle the line was found in.
    Line { text: text.into(), x: bounds.x as i32 + 1, y: bounds.y as i32 + 1, w: bounds.w as i32 - 2, h: bounds.h as i32 - 2 }
}

#[test]
fn a_line_that_has_not_changed_says_what_it_said() {
    let mut memory = Memory::default();
    let rgb = image(&[(at(4, 4), 90)]);
    let key = memory.line_key(&rgb, WIDTH, &at(4, 4));
    assert_eq!(memory.recall("desktop", key, &at(4, 4)), None, "never read");
    memory.keep("desktop", key, &at(4, 4), Some(&line("make test", &at(4, 4))));
    assert_eq!(memory.recall("desktop", key, &at(4, 4)), Some(Some(line("make test", &at(4, 4)))));
}

/// A terminal that scrolled: the same pixels, somewhere else.
#[test]
fn a_line_that_moved_whole_is_remembered_where_it_is_now() {
    let mut memory = Memory::default();
    let before = image(&[(at(4, 20), 90)]);
    let key = memory.line_key(&before, WIDTH, &at(4, 20));
    memory.keep("desktop", key, &at(4, 20), Some(&line("make test", &at(4, 20))));

    let after = image(&[(at(4, 12), 90)]);
    let moved = memory.line_key(&after, WIDTH, &at(4, 12));
    assert_eq!(moved, key, "the same pixels have the same key wherever they are");
    assert_eq!(memory.recall("desktop", moved, &at(4, 12)), Some(Some(line("make test", &at(4, 12)))), "placed where it is now");
}

#[test]
fn a_line_whose_pixels_changed_is_not_remembered() {
    let mut memory = Memory::default();
    let before = image(&[(at(4, 4), 90)]);
    let key = memory.line_key(&before, WIDTH, &at(4, 4));
    memory.keep("desktop", key, &at(4, 4), Some(&line("balance: 100", &at(4, 4))));

    let mut after = before.clone();
    after[((6 * WIDTH + 10) * 3) as usize] ^= 1;
    let changed = memory.line_key(&after, WIDTH, &at(4, 4));
    assert_ne!(changed, key, "one pixel is enough");
    assert_eq!(memory.recall("desktop", changed, &at(4, 4)), None);
    let shifted = memory.line_key(&before, WIDTH, &at(5, 4));
    assert_eq!(memory.recall("desktop", shifted, &at(5, 4)), None, "nor a rectangle one pixel over");
}

/// A screen is full of rectangles that read as nothing: icons, rules, scroll bars.
#[test]
fn a_rectangle_that_held_nothing_is_remembered_as_nothing() {
    let mut memory = Memory::default();
    let rgb = image(&[(at(4, 4), 33)]);
    let key = memory.line_key(&rgb, WIDTH, &at(4, 4));
    memory.keep("desktop", key, &at(4, 4), None);
    assert_eq!(memory.recall("desktop", key, &at(4, 4)), Some(None), "read before, and nothing: not read again");
}

/// A reading of Mind View must not be faster for what was read on the desktop.
#[test]
fn one_display_never_remembers_for_another() {
    let mut memory = Memory::default();
    let rgb = image(&[(at(4, 4), 90)]);
    let key = memory.line_key(&rgb, WIDTH, &at(4, 4));
    memory.keep("desktop", key, &at(4, 4), Some(&line("the person's own line", &at(4, 4))));
    let image_key = memory.image_key(&rgb, WIDTH, HEIGHT);
    memory.keep_image("desktop", image_key, &[line("the person's own line", &at(4, 4))]);

    assert_eq!(memory.recall("mind-view", key, &at(4, 4)), None, "the same pixels on another display are unread");
    assert_eq!(memory.same_image("mind-view", image_key), None, "and so is the same image");
    assert!(memory.same_image("desktop", image_key).is_some());
}

#[test]
fn an_image_that_has_not_changed_is_answered_from_the_last_answer() {
    let mut memory = Memory::default();
    let rgb = image(&[(at(4, 4), 90)]);
    let key = memory.image_key(&rgb, WIDTH, HEIGHT);
    assert_eq!(memory.same_image("desktop", key), None, "the first look");
    memory.keep_image("desktop", key, &[line("make test", &at(4, 4))]);
    assert_eq!(memory.same_image("desktop", key), Some(vec![line("make test", &at(4, 4))]));

    let mut changed = rgb.clone();
    changed[100] ^= 1;
    assert_eq!(memory.same_image("desktop", memory.image_key(&changed, WIDTH, HEIGHT)), None, "one pixel is a different image");
    assert_ne!(memory.image_key(&rgb, WIDTH, HEIGHT), memory.image_key(&rgb, HEIGHT, WIDTH), "and so is another shape");
}

/// A program that draws pixels cannot learn which pixels pass for another line: the keys of
/// one process are not the keys of another.
#[test]
fn two_readers_do_not_share_their_keys() {
    let rgb = image(&[(at(4, 4), 90)]);
    let (one, other) = (Memory::default(), Memory::default());
    assert_ne!(one.line_key(&rgb, WIDTH, &at(4, 4)), other.line_key(&rgb, WIDTH, &at(4, 4)));
}

/// The size of a rectangle is part of both keys: the same pixels cut to another shape are
/// another line.
#[test]
fn a_rectangle_of_another_size_has_another_key() {
    let memory = Memory::default();
    let rgb = image(&[(at(4, 4), 90)]);
    let (one, wider) = (memory.line_key(&rgb, WIDTH, &at(4, 4)), memory.line_key(&rgb, WIDTH, &Bounds { w: 31, ..at(4, 4) }));
    assert_ne!(one.0, wider.0);
    assert_ne!(one.1, wider.1);
}

/// Two hashes, with keys of their own, and a line is remembered only when both agree.
#[test]
fn a_line_needs_both_of_its_keys_to_be_remembered() {
    let mut memory = Memory::default();
    let rgb = image(&[(at(4, 4), 90)]);
    let key = memory.line_key(&rgb, WIDTH, &at(4, 4));
    assert_ne!(key.0, key.1, "two hashes, not one twice");
    memory.keep("desktop", key, &at(4, 4), None);
    assert_eq!(memory.recall("desktop", Key(key.0, key.1 ^ 1), &at(4, 4)), None, "the first half alone is not the line");
    assert_eq!(memory.recall("desktop", Key(key.0 ^ 1, key.1), &at(4, 4)), None, "nor the second");
    assert_eq!(memory.recall("desktop", key, &at(4, 4)), Some(None));
    let other = Memory::default().line_key(&rgb, WIDTH, &at(4, 4));
    assert!(other.0 != key.0 && other.1 != key.1, "and no half is shared with another process");
}

/// What one caller's app drew must not make another caller's read faster.
#[test]
fn one_caller_never_remembers_for_another() {
    let mut memory = Memory::default();
    let rgb = image(&[(at(4, 4), 90)]);
    let key = memory.line_key(&rgb, WIDTH, &at(4, 4));
    let image_key = memory.image_key(&rgb, WIDTH, HEIGHT);
    memory.keep("mind_view/aria", key, &at(4, 4), Some(&line("a token only B's app drew", &at(4, 4))));
    memory.keep_image("mind_view/aria", image_key, &[line("a token", &at(4, 4))]);
    assert_eq!(memory.recall("mind_view/bo", key, &at(4, 4)), None);
    assert_eq!(memory.same_image("mind_view/bo", image_key), None);
    assert!(memory.same_image("mind_view/aria", image_key).is_some());
}

#[test]
fn a_rectangle_past_the_end_of_the_image_is_hashed_without_a_panic() {
    let memory = Memory::default();
    let rgb = image(&[]);
    let below = Bounds { x: 0, y: HEIGHT - 2, w: WIDTH, h: 10 };
    let beside = Bounds { x: WIDTH - 4, y: 0, w: 30, h: 4 };
    let _ = (memory.line_key(&rgb, WIDTH, &below), memory.line_key(&rgb, WIDTH, &beside));
    assert_ne!(memory.line_key(&rgb, WIDTH, &below), memory.line_key(&rgb, WIDTH, &at(0, 0)));
}

#[test]
fn memory_is_bounded_and_keeps_what_was_seen_lately() {
    let mut memory = Memory::default();
    let recent = Bounds { x: 0, y: 0, w: 3, h: 3 };
    for n in 0..(MAX_LINES as u64 * 3) {
        let key = Key(n, n);
        // A look for every hundred lines, so old lines age.
        if n % 100 == 0 {
            memory.same_image("desktop", Key(u64::MAX, 0));
            memory.recall("desktop", Key(u64::MAX - 1, 0), &recent);
        }
        if n == 0 {
            memory.keep("desktop", Key(u64::MAX - 1, 0), &recent, None);
        }
        memory.keep("desktop", key, &at(0, 0), None);
    }
    let kept = memory.spaces["desktop"].lines.len();
    assert!(kept <= MAX_LINES, "{kept} lines kept");
    assert_eq!(memory.recall("desktop", Key(u64::MAX - 1, 0), &recent), Some(None), "a line seen on every look is still there");
    assert_eq!(memory.recall("desktop", Key(0, 0), &at(0, 0)), None, "one not seen since the start is gone");
}

#[test]
fn only_so_many_displays_are_remembered_and_the_least_used_goes_first() {
    let mut memory = Memory::default();
    for look in 0..10 {
        memory.same_image("desktop", Key(look, look));
    }
    for n in 0..MAX_SPACES * 2 {
        memory.same_image(&format!("display-{n}"), Key(0, 0));
    }
    assert!(memory.spaces.len() <= MAX_SPACES);
    assert!(memory.spaces.contains_key("desktop"), "the display looked at most is kept");
}

#[test]
fn forgetting_leaves_nothing() {
    let mut memory = Memory::default();
    let rgb = image(&[(at(4, 4), 90)]);
    let key = memory.line_key(&rgb, WIDTH, &at(4, 4));
    let image_key = memory.image_key(&rgb, WIDTH, HEIGHT);
    memory.keep("desktop", key, &at(4, 4), Some(&line("make test", &at(4, 4))));
    memory.keep_image("desktop", image_key, &[line("make test", &at(4, 4))]);
    memory.forget();
    assert_eq!(memory.recall("desktop", key, &at(4, 4)), None);
    assert_eq!(memory.same_image("desktop", image_key), None);
}
