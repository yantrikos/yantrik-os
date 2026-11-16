//! Whether the title of a window, a notification or a dock button names a secret (#1247, reviews R3 to R9).
//!
//! A foreign app's title follows what it has open: gedit says `tokens.json (~/backup) - gedit`, a
//! terminal editor its file. That names a protected file to a mind that is not allowed to know the file
//! is there. The judgement is here, in the leaf crate every side uses, so the shell's `describe`, its
//! switcher, its dock and its launcher, and the companion's own window tools, ask the one function.

use super::{is_a_copy_name, is_listed_place, is_secret_name, names_a_protected_copy, secret_cores, NAME_MAX};

/// Characters that may be part of a file's name inside a title. Every other character (spaces,
/// brackets, quotes of every kind, dashes, bullets, ellipses) ends a word (review M4).
const IN_A_NAME: &str = "._-~#%!:+@$";

/// Whether `title` names a secret: a word of it, or a run of words with the spaces between them kept
/// that is at most [`NAME_MAX`] bytes (no file's name is longer, so no name is missed), is a secret
/// name (also with the full stop, `?` or `:` a sentence puts after it: `Opening tokens.json.`), or
/// an editor's copy of a protected file (`%home%ann%.kube%config.swp`, judged a word at a time, see
/// [`is_an_editors_copy`]), or a word that is a path (`~/.aws`, `/home/ann/.ssh/id_rsa`,
/// `My Stuff/.kube`), alone or with the words up to the end of the next one (`~/.config/Code - OSS/User`),
/// is a protected place. `tokens.json*` (an
/// editor's unsaved mark) is the name `tokens.json`. The runs matter because names have spaces in
/// them: `Login Data`, `tokens (copy 1).json`, `tokens - Copy.json`, and a Dropbox conflict label
/// of any length (reviews R5-M3, R6-L1).
///
/// Only a run that holds a core of a secret's name ([`secret_cores`]) is judged, and at most
/// [`RUN_BUDGET`] of them: a title of distinct short words has up to 254,000 runs, and a page chooses
/// its title. A title with more is taken to name a secret (hidden from a mind: it fails closed). A
/// title longer than [`TITLE_MOST`] is not read. So the work is linear in the title and capped.
pub(super) fn names_a_secret(title: &str) -> bool {
    // The work is bounded whatever a page sets: a title longer than a window ever has is hidden from
    // a mind, as one that names a secret is (it fails closed).
    if title.len() > TITLE_MOST {
        return true;
    }
    let named = |span: &str| {
        let bare = span.trim_end_matches(['.', '?', '!', ':']);
        is_secret_name(span) || (bare.len() < span.len() && is_secret_name(bare))
    };
    let bounds = word_bounds(title, "");
    // A run is a secret's name only if it holds a core of one (`secret_cores`): the runs that hold
    // none are skipped without being judged, which is almost every run of an ordinary title. For
    // the start `at`, `first_end[at]` is where the shortest run that holds a core ends.
    let first_end = ends_of_the_first_core(title);
    // A run that reads as an earlier one is not judged again, and a start whose longest run does has
    // had all of its runs judged: a title that says the same words over (a page may set one) costs
    // its distinct runs only (review R7-H1).
    let mut seen = std::collections::HashSet::new();
    let mut judged = 0;
    let by_runs = bounds.iter().enumerate().any(|(i, &(start, first_word_end))| {
        // The words that end within NAME_MAX bytes of the start (they are in order, so a search finds the last).
        let within = &bounds[i..i + bounds[i..].partition_point(|&(_, end)| end - start <= NAME_MAX)];
        let Some(&(_, longest)) = within.last() else { return false };
        let holds_a_core_from = first_end[start];
        if longest < holds_a_core_from || seen.contains(&title[start..longest]) {
            return false;
        }
        // A run that starts with a public key (`id_rsa.pub - gedit`, `id_rsa.pub.bak - gedit`) is no
        // private key that has words after it: the secret starts of names would take it for one.
        // Unless it is a whole copy name (`client_secret.pub - Copy.json`), which `is_secret_name`
        // reads the same way (review R6-L2). A run cut inside a bracket is not whole: the
        // `id_rsa.pub (copy 1` of `id_rsa.pub (copy 1) - gedit` is no copy of anything.
        let public = title[start..first_word_end].to_lowercase().contains(".pub");
        within
            .iter()
            .enumerate()
            .filter(|&(_, &(_, end))| end >= holds_a_core_from)
            .map(|(n, &(_, end))| (n, &title[start..end]))
            .filter(|&(n, span)| n == 0 || !public || is_a_whole_copy_name(span))
            .any(|(_, span)| {
                if !seen.insert(span) {
                    return false;
                }
                judged += 1;
                judged >= RUN_BUDGET || named(span)
            })
    });
    by_runs || names_an_editors_copy(title, &bounds) || names_a_protected_path(title)
}

/// The longest title [`names_a_secret`] reads, in bytes; a longer one is hidden from a mind. No
/// window's title comes near it, and every step of the judgement is linear in the title or capped.
const TITLE_MOST: usize = 16 * 1024;

/// The most runs of words [`names_a_secret`] judges in one title. Only a run that holds a core of a
/// secret's name is judged at all (`ends_of_the_first_core`), so an ordinary title judges none; a
/// title that sprinkles cores through thousands of runs is the one a page can build to cost time,
/// and is taken to name a secret past this many (it fails closed). 1,024 runs of 255 bytes cost 30 to
/// 55 ms in a debug build and 6 in release (gate 12 measured the 8,192 runs before it at 389 to 926 ms
/// for a whole `describe`). A title with a core in it and over about 300 bytes of words may reach it
/// (review R8-M1, R9-L1).
const RUN_BUDGET: usize = 1024;

/// For each byte of `title`, where the earliest-ending occurrence of a secret's core
/// ([`secret_cores`]) that starts at or after it ends: `usize::MAX` when there is none. A run from
/// `at` that ends before that holds no core, so it names no secret and is not judged. The cores are
/// ASCII and the rule lower-cases, so a title with a letter that lower-cases to ASCII (`K` Kelvin, `İ`)
/// is not skipped by this at all (`0` everywhere). One pass of the cores over the title: linear in it.
fn ends_of_the_first_core(title: &str) -> Vec<usize> {
    let mut first_end = vec![usize::MAX; title.len() + 1];
    if title.chars().any(|c| !c.is_ascii() && c.to_lowercase().any(|l| l.is_ascii())) {
        first_end.fill(0);
        return first_end;
    }
    let lowered = title.to_ascii_lowercase();
    for core in secret_cores() {
        let core = core.as_slice();
        for (at, window) in lowered.as_bytes().windows(core.len()).enumerate() {
            if window == core {
                first_end[at] = first_end[at].min(at + core.len());
            }
        }
    }
    for at in (0..title.len()).rev() {
        first_end[at] = first_end[at].min(first_end[at + 1]);
    }
    first_end
}

/// Whether some word of `title` is an editor's copy ([`is_an_editors_copy`]), alone or joined with
/// everything up to the end of the next word that has the same `%` or `!` in it, when that is at most
/// [`NAME_MAX`] bytes: the place `Code - OSS` has a ` - ` in its name, so
/// `%home%ann%.config%Code - OSS%User%History%-1a2b%x.json.swp` is three words (review R8-L1). One
/// more check for each word, and each span is judged once.
fn names_an_editors_copy(title: &str, bounds: &[(usize, usize)]) -> bool {
    let joined = ['%', '!'].into_iter().flat_map(|sep| {
        let marked: Vec<(usize, usize)> = bounds.iter().copied().filter(|&(start, end)| title[start..end].contains(sep)).collect();
        // Joined only when the second word is where an editor's copy ends (`.swp`, `~`, `#`): a title
        // that has two `!` words in a sentence is no encoded path (review R9-L2).
        marked
            .windows(2)
            .filter(|pair| ends_like_an_editors_copy(&title[pair[1].0..pair[1].1]))
            .map(|pair| (pair[0].0, pair[1].1))
            .filter(|(start, end)| end - start <= NAME_MAX)
            .collect::<Vec<_>>()
    });
    let mut words = std::collections::HashSet::new();
    bounds.iter().copied().chain(joined).any(|(start, end)| words.insert(&title[start..end]) && is_an_editors_copy(&title[start..end]))
}

/// Whether `word` ends as an editor's copy of a file does: Vim's swap and undo (`.swp`, `.swo`, `.s??`,
/// `.un~`, `.kate-swp`), Emacs's backup (`~`, `~1~`) and auto-save (`#`).
fn ends_like_an_editors_copy(word: &str) -> bool {
    word.ends_with(['~', '#'])
        || word.rsplit_once('.').is_some_and(|(_, end)| end == "kate-swp" || (end.len() == 3 && end.starts_with('s')))
}

/// Whether some `/` word of `title` is a protected path, or is one with the words up to the end of
/// the next `/` word joined on: `~/.config/Code - OSS/User/History/x` is the words `~/.config/Code`,
/// `-` and `OSS/User/History/x`, and a place has a ` - ` in its name (review R7-L3). One more
/// check for each word, whose span is at most [`PATH_SPAN`] bytes.
fn names_a_protected_path(title: &str) -> bool {
    let slashed: Vec<(usize, usize)> = word_bounds(title, "/").into_iter().filter(|&(start, end)| title[start..end].contains('/')).collect();
    let pairs = slashed.windows(2).map(|pair| (pair[0].0, pair[1].1)).filter(|(start, end)| end - start <= PATH_SPAN);
    let mut seen = std::collections::HashSet::new();
    slashed.iter().copied().chain(pairs).any(|(start, end)| seen.insert(&title[start..end]) && is_a_protected_path(&title[start..end]))
}

/// The longest run of words that is read as one path, in bytes.
const PATH_SPAN: usize = 4096;

/// Whether `word` is an editor's copy that spells a protected path into its name. A word is judged
/// alone, with at most one `#` taken off each end (Emacs's auto-save is `#!home!ann!.kube!config#`),
/// and with the separator put in front when it is a word's tail: the `stuff%.kube%config.swp` of
/// `%home%ann%my stuff%.kube%config.swp` is `/stuff/.kube/config.swp` (review R7-H1). Not every
/// run of words: a title of 1,365 `%a` words has 100,000 runs.
fn is_an_editors_copy(word: &str) -> bool {
    let word = word.strip_prefix('#').unwrap_or(word);
    let word = word.strip_suffix('#').unwrap_or(word);
    ['%', '!'].into_iter().filter(|sep| word.contains(*sep)).any(|sep| {
        names_a_protected_copy(word) || (!word.starts_with(sep) && names_a_protected_copy(&format!("{sep}{word}")))
    })
}

/// Whether `span` is a copy name by its marker and closes every bracket it opens.
fn is_a_whole_copy_name(span: &str) -> bool {
    is_a_copy_name(span) && span.matches('(').count() == span.matches(')').count()
}

/// The byte range of each word of `title`: a run of letters, digits, [`IN_A_NAME`] and `also`.
fn word_bounds(title: &str, also: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut start = None;
    // The end of the title closes the last word like any character that is not part of one.
    for (at, c) in title.char_indices().chain(std::iter::once((title.len(), ' '))) {
        let in_a_word = c.is_alphanumeric() || IN_A_NAME.contains(c) || also.contains(c);
        match (in_a_word, start) {
            (true, None) => start = Some(at),
            (false, Some(from)) => {
                found.push((from, at));
                start = None;
            }
            _ => {}
        }
    }
    found
}

/// Whether `word`, with a `~/` start read as the home, is a path into a protected place, judged on
/// its text alone ([`is_listed_place`]: no filesystem, so a page's title costs nothing). A word
/// that is not absolute is judged below `/`, since `PROTECTED` matches its parts anywhere: the
/// `Stuff/.kube` of `~/My Stuff/.kube` is a protected place (review R6-L3).
fn is_a_protected_path(word: &str) -> bool {
    let path = match (word.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => std::path::Path::new(&home).join(rest),
        _ => std::path::Path::new("/").join(word),
    };
    is_listed_place(&path)
}

/// How long a verdict on a title is kept: one `describe shell` asks about the same title on every
/// surface that shows it (windows, front window, switcher, dock, launcher), and the judgement of a
/// long title is the one cost a page chooses (gate 12 measured 535 and 926 ms at the 300 ms bar).
/// Short, because what makes a path a protected place is on the disk and can change.
const VERDICT_KEPT: std::time::Duration = std::time::Duration::from_secs(2);

/// The most titles whose verdict is kept; past it the memory starts over.
const VERDICTS_MOST: usize = 512;

thread_local! {
    static VERDICTS: std::cell::RefCell<std::collections::HashMap<String, (std::time::Instant, bool)>> = Default::default();
}

/// Whether a window's, a notification's or a dock label's `title` names a secret (a file name, an editor's
/// copy of a protected file, or a protected place), judged once for a title in [`VERDICT_KEPT`]. The shell answers `describe` on
/// one thread, so one memory per thread is the one every surface of an answer reads.
pub fn title_names_a_secret(title: &str) -> bool {
    let now = std::time::Instant::now();
    if let Some(verdict) = VERDICTS.with(|kept| kept.borrow().get(title).filter(|(at, _)| now.duration_since(*at) < VERDICT_KEPT).map(|&(_, hides)| hides)) {
        return verdict;
    }
    let verdict = names_a_secret(title);
    VERDICTS.with(|kept| {
        let mut kept = kept.borrow_mut();
        if kept.len() >= VERDICTS_MOST {
            kept.clear();
        }
        kept.insert(title.to_string(), (now, verdict));
    });
    verdict
}

#[cfg(test)]
#[path = "titles_tests.rs"]
mod tests;
