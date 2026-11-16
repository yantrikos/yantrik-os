//! The window titles `describe shell` hands a mind (#1247, review R3-6).
//!
//! Yantrik's own windows have fixed titles. A foreign app's title follows what it has open:
//! gedit says `tokens.json (~/backup) - gedit`, a terminal editor its file. That names a protected
//! file to a mind that is not allowed to know the file is there. For a mind, a title with a secret
//! name in it is replaced by the app's display name and marked `title_hidden`; the person's own
//! view is whole.

use yantrik_ipc_contracts::home_paths::{is_a_copy_name, is_listed_place, is_secret_name, names_a_protected_copy, NAME_MAX};

use crate::windows::{app_display_name, WindowEntry};

/// What `in_front` says when the front window's title is hidden and no listed window has it.
const A_WINDOW: &str = "a window";

/// What the title of a notification says when it is hidden.
const A_NOTICE: &str = "a notification";

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
/// At most [`RUN_BUDGET`] runs are judged: a title of distinct short words has up to 254,000, and a
/// page chooses its title. A title with more is taken to name a secret (hidden from a mind: it fails
/// closed). The densest title of 255 bytes has about 8,000 runs, and no real title comes near.
fn names_a_secret(title: &str) -> bool {
    let named = |span: &str| {
        let bare = span.trim_end_matches(['.', '?', '!', ':']);
        is_secret_name(span) || (bare.len() < span.len() && is_secret_name(bare))
    };
    let bounds = word_bounds(title, "");
    // A run that reads as an earlier one is not judged again, and a start whose longest run does has
    // had all of its runs judged: a title that says the same words over (a page may set one) costs
    // its distinct runs only (review R7-H1).
    let mut seen = std::collections::HashSet::new();
    let mut judged = 0;
    let by_runs = bounds.iter().enumerate().any(|(i, &(start, first_end))| {
        // The words that end within NAME_MAX bytes of the start (they are in order, so a search finds the last).
        let within = &bounds[i..i + bounds[i..].partition_point(|&(_, end)| end - start <= NAME_MAX)];
        let Some(&(_, longest)) = within.last() else { return false };
        if seen.contains(&title[start..longest]) {
            return false;
        }
        // A run that starts with a public key (`id_rsa.pub - gedit`, `id_rsa.pub.bak - gedit`) is no
        // private key that has words after it: the secret starts of names would take it for one.
        // Unless it is a whole copy name (`client_secret.pub - Copy.json`), which `is_secret_name`
        // reads the same way (review R6-L2). A run cut inside a bracket is not whole: the
        // `id_rsa.pub (copy 1` of `id_rsa.pub (copy 1) - gedit` is no copy of anything.
        let public = title[start..first_end].to_lowercase().contains(".pub");
        within.iter().enumerate().map(|(n, &(_, end))| (n, &title[start..end])).filter(|&(n, span)| n == 0 || !public || is_a_whole_copy_name(span)).any(|(_, span)| {
            if !seen.insert(span) {
                return false;
            }
            judged += 1;
            judged >= RUN_BUDGET || named(span)
        })
    });
    by_runs || names_an_editors_copy(title, &bounds) || names_a_protected_path(title)
}

/// The most runs of words [`names_a_secret`] judges in one title: about the 8,256 of the densest title
/// of [`NAME_MAX`] bytes (review R8-M1).
const RUN_BUDGET: usize = 8192;

/// Whether some word of `title` is an editor's copy ([`is_an_editors_copy`]), alone or joined with
/// everything up to the end of the next word that has the same `%` or `!` in it, when that is at most
/// [`NAME_MAX`] bytes: the place `Code - OSS` has a ` - ` in its name, so
/// `%home%ann%.config%Code - OSS%User%History%-1a2b%x.json.swp` is three words (review R8-L1). One
/// more check for each word, and each span is judged once.
fn names_an_editors_copy(title: &str, bounds: &[(usize, usize)]) -> bool {
    let joined = ['%', '!'].into_iter().flat_map(|sep| {
        let marked: Vec<(usize, usize)> = bounds.iter().copied().filter(|&(start, end)| title[start..end].contains(sep)).collect();
        marked.windows(2).map(|pair| (pair[0].0, pair[1].1)).filter(|(start, end)| end - start <= NAME_MAX).collect::<Vec<_>>()
    });
    let mut words = std::collections::HashSet::new();
    bounds.iter().copied().chain(joined).any(|(start, end)| words.insert(&title[start..end]) && is_an_editors_copy(&title[start..end]))
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

/// Whether a title is hidden from the caller: it is a mind, and the title names a secret.
pub fn hides(title: &str, mind: bool) -> bool {
    mind && names_a_secret_once(title)
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

/// [`names_a_secret`], judged once for a title in [`VERDICT_KEPT`]. The shell answers `describe` on
/// one thread, so one memory per thread is the one every surface of an answer reads.
fn names_a_secret_once(title: &str) -> bool {
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

/// The app's id and display name as a mind is shown them. A window that declares no app id gets one
/// derived from its title, so an id can name a secret as the title did: `tokens.json - foo` has the
/// id `tokens.json`, and `tokens (copy 1).json - foo` the id `tokens`: a first word of a hidden
/// title that is the id. Then both are [`A_WINDOW`]'s words, whatever the title says (reviews R6-M1,
/// R7-L1). `title` is the title the id stands beside.
pub fn shown_app(app_id: &str, title: &str, mind: bool) -> (String, String) {
    let name = app_display_name(app_id);
    let derived = title.to_lowercase().split_whitespace().next() == Some(app_id) && hides(title, mind);
    if derived || hides(app_id, mind) || hides(&name, mind) {
        ("window".to_string(), A_WINDOW.to_string())
    } else {
        (app_id.to_string(), name)
    }
}

/// One cell of the window switcher as `describe shell` reports it: its `name` (which is the window's
/// title when the app has no name of its own) and its `title`, and `title_hidden: true` in place of
/// both, with the app's display name, when `mind` is asking and either names a secret.
pub fn cell(name: &str, title: &str, app_id: &str, mind: bool) -> serde_json::Value {
    let (app, shown) = shown_app(app_id, title, mind);
    if hides(title, mind) || hides(name, mind) {
        serde_json::json!({ "name": shown, "title": shown, "app": app, "title_hidden": true })
    } else {
        serde_json::json!({ "name": name, "title": title, "app": app })
    }
}

/// The title of a notification for `describe shell`: [`A_NOTICE`] when `mind` is asking and it
/// names a secret (a download's notice names the file), and whether it was hidden.
pub fn notice_title(title: &str, mind: bool) -> (String, bool) {
    if hides(title, mind) { (A_NOTICE.to_string(), true) } else { (title.to_string(), false) }
}

/// A dock button's `app`, its label and whether the label was hidden, for `describe shell`: the
/// label is the app's display name when `mind` is asking and it names a secret (an app with no name
/// of its own is labelled by its window's title), and the app is as [`shown_app`] gives it (reviews
/// R5-M2, R6-M1).
pub fn dock_label(label: &str, app_id: &str, mind: bool) -> (String, String, bool) {
    let (app, shown) = shown_app(app_id, label, mind);
    if hides(label, mind) { (app, shown, true) } else { (app, label.to_string(), false) }
}

/// One row of `windows` for `describe shell`: `title` and `app`, and `title_hidden: true` in place
/// of the title when `mind` is asking and the title names a secret.
pub fn row(window: &WindowEntry, mind: bool) -> serde_json::Value {
    let (app, shown) = shown_app(&window.app_id, &window.title, mind);
    if hides(&window.title, mind) {
        serde_json::json!({ "title": shown, "app": app, "title_hidden": true })
    } else {
        serde_json::json!({ "title": window.title, "app": app })
    }
}

/// `in_front`, the front window's title as `windows` lists it, for `mind` asking: the app's display
/// name when the title names a secret.
pub fn in_front(title: Option<String>, windows: &[WindowEntry], mind: bool) -> Option<String> {
    let title = title?;
    if !hides(&title, mind) {
        return Some(title);
    }
    Some(windows.iter().find(|w| w.title == title).map_or_else(|| A_WINDOW.to_string(), |w| shown_app(&w.app_id, &w.title, mind).1))
}

#[cfg(test)]
#[path = "control_window_titles_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "control_window_titles_regression_tests.rs"]
mod regression_tests;
