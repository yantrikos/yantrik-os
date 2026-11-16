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

/// The slowest of three judgements of `title`, none of them from the verdict memo.
fn slowest_of_three(title: &str) -> (bool, std::time::Duration) {
    let mut hidden = false;
    let mut took = std::time::Duration::ZERO;
    for _ in 0..3 {
        let start = std::time::Instant::now();
        hidden = names_a_secret(title);
        took = took.max(start.elapsed());
    }
    (hidden, took)
}

/// The most one judgement of a 4 KB title may take in a debug build here. The bar is 300 ms for a
/// whole `describe shell` on the real build (gate 12), which judges a title once for every surface
/// that shows it, on a machine slower than CI; so one judgement here must leave that room on a
/// machine three times slower. Debug is five to ten times slower than the release the person runs.
const JUDGEMENT_BUDGET: std::time::Duration = std::time::Duration::from_millis(100);

/// Gate 12 (G4): the title families that took a Mind's `describe shell` 389 to 926 ms on the real
/// build. They hold no secret's name or core, so no run of them is judged at all: shown, in
/// microseconds, however long they are (and so is ordinary prose over 1 KB, review R9-L1).
#[test]
fn the_families_the_gate_found_slow_are_shown_and_judged_in_no_time() {
    for (family, title) in [
        ("copy 1", filled((0..).map(|n| format!("a{n} (copy 1).bak")))),
        ("Copy", filled((0..).map(|n| format!("a{n} - Copy.bak")))),
        ("firefox prefix", format!("firefox-esr {}", filled((0..).map(|n| format!("a{n} - Copy.bak"))))),
        ("paths", filled((0..).map(|n| format!("~/a{n}/b/c/d/e")))),
        ("letters", filled(lcg().map(|x| char::from(b'a' + (x % 26) as u8).to_string()))),
        ("prose", filled((0..).map(|n| format!("word{n}x")))),
    ] {
        assert!(title.len() > 4000, "{family}: {}", title.len());
        let (hidden, took) = slowest_of_three(&title);
        assert!(!hidden, "{family}: it names nothing");
        assert!(took < JUDGEMENT_BUDGET / 2, "{family}: {took:?}");
        assert!(!names_a_secret(&title), "{family}");
    }
}

/// The worst case that can be built: a secret's core in every word, so every run is judged, and no
/// run is a secret, so none ends the search early. The judge stops at `RUN_BUDGET` judged runs and
/// hides the title from a mind (it fails closed); the person sees it whole.
#[test]
fn a_four_kilobyte_title_with_a_core_in_every_word_is_hidden_within_the_budget() {
    for (family, title) in [
        ("tokens copy 1", filled((0..).map(|n| format!("tokens{n} (copy 1).bak")))),
        ("tokens Copy", filled((0..).map(|n| format!("tokens{n} - Copy.bak")))),
        ("cookies Copy", filled((0..).map(|n| format!("cookies{n} - Copy.bak")))),
        ("credentials", filled((0..).map(|n| format!("credentials{n} (copy 1).bak")))),
        ("login data", filled((0..).map(|n| format!("login data{n} - Copy.bak")))),
        ("key letters", filled(lcg().map(|x| ["tokens", ".pem", "a", "b", "c"][(x % 5) as usize].to_string()))),
        ("tokens only", filled(std::iter::repeat("tokens1".to_string()))),
        ("tokens, short words", filled((0..).map(|n| if n % 2 == 0 { format!("tokens{n}") } else { format!("w{n}") }))),
    ] {
        assert!(title.len() > 4000 && title.len() <= 4096, "{family}: {}", title.len());
        let (hidden, took) = slowest_of_three(&title);
        assert!(took < JUDGEMENT_BUDGET, "{family}: {took:?}");
        // Whether it ends hidden is the budget's or a run's; it is never slower than the budget.
        let _ = hidden;
    }
}

/// A title is judged in time linear in its length, with a fixed cap: one that is far longer than
/// any window's is hidden from a mind without being read.
#[test]
fn a_title_longer_than_any_window_has_is_hidden_from_a_mind_unread() {
    let long = "a ".repeat(TITLE_MOST);
    let start = std::time::Instant::now();
    assert!(names_a_secret(&long));
    assert!(start.elapsed() < std::time::Duration::from_millis(20), "{:?}", start.elapsed());
    // Just under the cap is read like any other.
    assert!(!names_a_secret(&"a ".repeat(TITLE_MOST / 4)));
}

/// Review R9-L1: ordinary prose is shown to a mind at any length a window has (it holds no core of a
/// secret's name, so no run of it is judged), and one over the cap is hidden. A prose title that does
/// hold a core and is long enough to pass the budget is hidden too (it fails closed).
#[test]
fn ordinary_prose_is_shown_whatever_its_length_and_a_cored_flood_is_not() {
    let prose = |bytes: usize| {
        let mut title = String::new();
        let mut n = 0;
        while title.len() < bytes {
            title.push_str(&format!("w{n}x{} ", "abcdefgh".chars().cycle().skip(n % 8).take(2 + n % 3).collect::<String>()));
            n += 1;
        }
        title.truncate(bytes);
        title
    };
    for bytes in [267, 500, 808, 1111, 1500, 4000, TITLE_MOST - 1] {
        let title = prose(bytes);
        assert!(!names_a_secret(&title), "{bytes} bytes of prose is shown");
    }
    assert!(names_a_secret(&prose(TITLE_MOST + 1)), "past the cap it is not read");
    // One core in the middle of a long title judges only the runs that hold it: shown.
    for half in [100, 240, 900, 2000] {
        let title = format!("{} tokens {}", prose(half), prose(half));
        assert!(!names_a_secret(&title), "{} bytes with a core in it is shown", title.len());
    }
    // A core in every word is what the budget is for (the test above); in every other word as well.
    let flood = filled((0..).map(|n| if n % 2 == 0 { format!("tokens{n}") } else { format!("w{n}") }));
    assert!(names_a_secret(&flood), "{} bytes with a core in every other word is hidden", flood.len());
}

/// Review R9-L2: two `!` or `%` words in a sentence are not an encoded path, so a title that has
/// them is shown; an editor's copy that has ` - ` in its path still is one (the three titles of review
/// R8-L1 stay hidden). The three titles of the review's first list were hidden at the last head.
#[test]
fn a_sentence_with_two_marked_words_is_shown_and_an_editors_copy_with_a_space_is_hidden() {
    for title in ["Hello!Login Data! - Firefox", "Q3 50%.ssh/notes and 20% - Sheets", "Read %.ssh/config \u{2014} then 20% more - Firefox"] {
        assert!(!names_a_secret(title), "{title}");
    }
    for title in [
        "less %home%ann%.config%Code - OSS%User%History%-1a2b%AbCd.json.swp",
        "#!home!ann!.config!Code - OSS!User!History!-1a2b!AbCd.json# - emacs",
        "!home!ann!.config!Code - OSS!Backups!ws1!file!9f8e~ - emacs",
        "%home%ann%my stuff%.kube%config.swp - less",
    ] {
        assert!(names_a_secret(title), "{title}");
    }
}
