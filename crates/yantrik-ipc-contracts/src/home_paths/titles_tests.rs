//! The cost and the budget of judging a title (#1247, reviews R6 to R9, gate 12): a page chooses its
//! title, so the judgement must cost time bounded by the work the title can hold, whatever it holds.

use super::*;

/// Review R7-H1: a title of many short words cost seconds to minutes (100,000 runs, each reading the
/// filesystem). Each is judged in under 50 ms, best of three. The judge is pure (`is_listed_place`
/// reads no filesystem and no `HOME` but the `~/` of a path), so the test sets nothing: an
/// environment variable set in a test races the tests that read it. The unfixed filter took 2 to 7
/// seconds a title with no `HOME` at all, and more with one.
#[test]
fn a_four_kilobyte_title_of_short_words_is_judged_in_no_time() {
    for title in [
        "%a ".repeat(1365), "#%a# ".repeat(819), "!a!b ".repeat(819), "%.ssh ".repeat(682), "a/b ".repeat(1024),
    ] {
        let took = (0..3)
            .map(|_| {
                let start = std::time::Instant::now();
                let hidden = names_a_secret(&title);
                assert!(!hidden, "no file has such a name: {}", &title[..12]);
                start.elapsed()
            })
            .min()
            .unwrap_or_default();
        assert!(took < std::time::Duration::from_millis(50), "{}: {took:?}", &title[..12]);
    }
}

/// Review R6-M2: a page's title of stacked markers cost cubic time per `describe shell`. Runs over
/// 255 bytes are not judged, and each name below it is judged once. The bound is the best of three
/// runs under 50 ms: the fixed filter takes microseconds, the unfixed one seconds.
#[test]
fn a_four_kilobyte_title_of_stacked_markers_is_judged_in_no_time() {
    for title in [format!("%{}", "~".repeat(4095)), format!("!{}", "~".repeat(4095)), format!("%{}", ":x".repeat(512)), format!("%{} \u{2014} Mozilla Firefox", "~".repeat(4090))] {
        let took = (0..3)
            .map(|_| {
                let start = std::time::Instant::now();
                let hidden = names_a_secret(&title);
                assert!(!hidden, "no file has such a name");
                start.elapsed()
            })
            .min()
            .unwrap_or_default();
        assert!(took < std::time::Duration::from_millis(50), "{took:?}");
    }
}

/// A title of the `pieces`, joined by spaces, filled to at most 4,096 bytes.
fn filled(pieces: impl Iterator<Item = String>) -> String {
    let mut title = String::new();
    for piece in pieces {
        if title.len() + piece.len() + 1 > 4096 {
            break;
        }
        if !title.is_empty() {
            title.push(' ');
        }
        title.push_str(&piece);
    }
    title
}

/// A fixed sequence, so the test is the same on every run.
fn lcg() -> impl Iterator<Item = u64> {
    std::iter::successors(Some(1u64), |x| Some(x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407))).map(|x| x >> 33)
}

/// Review R8-M1: a title of distinct short words defeats the memo, and had up to 254,000 runs to
/// judge (0.1 to 0.33 s in release, 2.4 s in debug). The judge stops at `RUN_BUDGET` runs and hides
/// the title from a mind. Each is hidden and judged in under 250 ms, best of three (debug here: 50 to
/// 155 ms with the budget, 1.0 to 3.5 s without; 150 ms was too near the slowest family to hold on a
/// busy runner).
#[test]
fn a_four_kilobyte_title_of_distinct_words_is_hidden_within_the_budget() {
    let tokens = ["%", "!", "#", "~", "a"];
    for (family, title) in [
        ("paths", filled((0..).map(|n| format!("~/a{n}/b/c/d/e")))),
        ("copies", filled((0..).map(|n| format!("a{n} (copy 1).bak")))),
        ("letters", filled(lcg().map(|x| char::from(b'a' + (x % 26) as u8).to_string()))),
        ("tokens", filled(lcg().map(|x| tokens[(x % 5) as usize].to_string()))),
    ] {
        assert!(title.len() > 4000 && title.len() <= 4096, "{family}: {}", title.len());
        let took = (0..3)
            .map(|_| {
                let start = std::time::Instant::now();
                let hidden = names_a_secret(&title);
                assert!(hidden, "{family}: too many runs to judge is hidden from a mind");
                start.elapsed()
            })
            .min()
            .unwrap_or_default();
        assert!(took < std::time::Duration::from_millis(250), "{family}: {took:?}");
    }
}

/// Gate 12: one `describe shell` asks about the same title on every surface that shows it, and a verdict
/// kept for a moment is the same as judging it again; the memory never grows past its bound.
#[test]
fn a_title_judged_again_gets_the_same_verdict_and_the_memory_is_bounded() {
    for n in 0..VERDICTS_MOST * 2 {
        let (secret, plain) = (format!("tokens.json - app{n}"), format!("notes.md - app{n}"));
        for _ in 0..3 {
            assert!(title_names_a_secret(&secret) && !title_names_a_secret(&plain), "{n}");
        }
        assert_eq!(title_names_a_secret(&secret), names_a_secret(&secret));
    }
    VERDICTS.with(|kept| assert!(kept.borrow().len() <= VERDICTS_MOST));
}

/// Review R9-L1: where ordinary titles stay shown. A title of distinct prose words of 6 to 8 bytes, cut at
/// 500 bytes (about 3,000 runs), is under the run budget, so a Mind sees it whole.
#[test]
fn a_five_hundred_byte_title_of_distinct_prose_words_is_shown_to_a_mind() {
    let mut title = String::new();
    for n in 0.. {
        let word = if n % 2 == 0 { format!("w{n:05}") } else { format!("w{n:06}") };
        if title.len() + word.len() + 1 > 500 {
            break;
        }
        if !title.is_empty() {
            title.push(' ');
        }
        title.push_str(&word);
    }
    assert!(title.len() > 480 && title.len() <= 500, "{}", title.len());
    assert!(!title_names_a_secret(&title), "ordinary prose under the budget stays shown");
}
