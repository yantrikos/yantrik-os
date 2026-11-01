//! `check-sentences`: no string literal in the tree says a run of spaces (#993). [`literals`] is
//! the scanner, [`check`] the whole-tree rule.
//!
//! A long sentence split across source lines needs a `\` at the end of each; when an edit drops
//! it, the next line's indentation lands inside the words a person reads — "written and
//! your draft" came out with a gap where the `\` used to be, and it happened twice. The guard
//! was a test inside the documents layer, holding two folders
//! (`frameworks/yantrik-app/src/documents/sentences_tests.rs`); this is the whole-tree one, and
//! it reads the same way that test reads, ported here:
//!
//! Source text, not a grammar. Comments are not said; character literals and raw strings keep
//! their spaces by design; a `#[cfg(test)]` marks code no program ships — and
//! `#[cfg(all(test, unix))]` marks it too, since a conjunctive condition holds nowhere `test`
//! does not. `#[cfg(not(test))]` and `#[cfg(any(test, unix))]` both hold in a real build, so
//! they mark exactly what the rule holds.

use std::path::Path;

/// Whether a file is code only a test builds, where columns are aligned in runs of spaces on
/// purpose: `tests` folders at any depth and at the root (the repo's own holds the screen-preview
/// app), `*_tests.rs` and `*_tests` modules. These are the three terms the app-local guard it
/// replaces used to skip them; nothing else is excused by name.
pub(crate) fn is_test(file: &str) -> bool {
    file.split('/').any(|part| part == "tests")
        || file.ends_with("tests.rs")
        || file.contains("_tests")
}

/// Every string literal of `source` outside comments and outside the items a `cfg` marks as test
/// code (a `mod { }` block, or one `use`/`fn`/`mod x;` up to its `;`), with a line continuation
/// (`\` and a newline, and the indentation after it) taken out as Rust takes it out. The rest of
/// the file is read to its end: a test module near the top of a file hides nothing after it.
pub(crate) fn literals(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let (mut out, mut i) = (Vec::new(), 0);
    scan(&chars, &mut i, &mut out, false);
    out
}

/// Whether any line of what a literal says holds a hole: a run of three spaces or more with a
/// word of the line on each side of it, which is what a lost `\` leaves behind — the next line's
/// indentation joins in between two words and a screen says "written and              your draft".
///
/// A word is a letter or a digit, or the apostrophe of `don't`, and what runs up to one counts
/// with it: `window              (known: x)` is as lost a continuation as `window              x`,
/// and `done.              Next` no less. So a run counts against the mark that ends the word
/// before it (`.`, a closing `)`) and the mark that opens the word after it (`(`). A comma is in
/// neither, since a column list pads after each of its commas (`"SELECT id,   name"`) without
/// saying anything was lost. A colon counts as a word *before* a run only: a sentence breaks after
/// one (`"...hands work on: run it again"`), while a run before one is a label padded to its `:`.
/// A `{` after a run is the padded value of such a label
/// (`"  Edges:                 {}"`), never a word, so that run is not a hole.
///
/// A run at the start of a line of what is said indents it (`format!("\n    {body}")`), and a run
/// at the end of one pads it; neither has a word on the far side. A great many literals say more
/// than one line — SQL, a `usage:` screen, a document's text — and every indent in them would
/// otherwise be a hole. Hence the reader hands back `\n` as a newline and `\t` as a tab, and the
/// lines are counted one at a time. An indented line formats, as in SQL, and is read for no hole
/// at all.
///
/// A column aligned on purpose says just what a lost `\` says, so those few literals are excused
/// by name in [`ALIGNED_ON_PURPOSE`].
pub(crate) fn lost_its_continuation(said: &str) -> bool {
    said.split('\n')
        .filter(|line| !line.starts_with(' '))
        .any(|line| !hole_runs(line).is_empty())
}

/// Each hole of one line of what is said, as the column its run starts at and how wide it is;
/// see [`lost_its_continuation`].
fn hole_runs(line: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = line.chars().collect();
    // What a word ends with, and what the next one opens with. A `{` is in neither (a run before
    // one pads a label to its value), and so is a `,`, which a column list pads after.
    let ends = |c: char| {
        c.is_alphanumeric() || matches!(c, '\'' | ':' | '.' | ';' | '!' | '?' | ')' | ']' | '"')
    };
    let opens = |c: char| c.is_alphanumeric() || matches!(c, '\'' | '(' | '[');
    let (mut holes, mut k) = (Vec::new(), 0);
    while k < chars.len() {
        if chars[k] != ' ' {
            k += 1;
            continue;
        }
        let end = chars[k..]
            .iter()
            .position(|c| *c != ' ')
            .map_or(chars.len(), |d| k + d);
        let before = k > 0 && ends(chars[k - 1]);
        let after = end < chars.len() && opens(chars[end]);
        if end - k >= 3 && before && after {
            holes.push((k, end - k));
        }
        k = end;
    }
    holes
}

fn starts(chars: &[char], at: usize, text: &str) -> bool {
    text.chars().enumerate().all(|(k, c)| chars.get(at + k) == Some(&c))
}

/// Where the character literal beginning at `at` ends, if one begins there: `'x'`, `'\n'`,
/// `'\x7f'`, `'\u{1F600}'`, `'\''`, `'"'`. `None` where the quote is not one opening a character
/// literal — a lifetime (`&'a str`), an apostrophe in text the scanner has no business reading —
/// so the caller passes the quote over rather than run away with the rest of the line.
fn quote(chars: &[char], at: usize) -> Option<usize> {
    let closed = |k: usize| (chars.get(k) == Some(&'\'')).then_some(k + 1);
    if chars.get(at) != Some(&'\'') {
        return None;
    }
    match *chars.get(at + 1)? {
        '\\' => match *chars.get(at + 2)? {
            // The only escape whose length is not fixed.
            'u' => chars[at + 2..]
                .iter()
                .copied()
                .position(|c| c == '}')
                .and_then(|k| closed(at + 2 + k + 1)),
            'x' => {
                let digits = chars[at + 3..]
                    .iter()
                    .copied()
                    .take(2)
                    .take_while(char::is_ascii_hexdigit)
                    .count();
                closed(at + 3 + digits)
            }
            // Any other escape is one character, itself possibly a quote or a brace.
            c if !c.is_whitespace() => closed(at + 3),
            _ => None,
        },
        // A character literal holds one character, so the quote after it is the last one; a quote
        // with nothing between is two lifetimes, not a literal.
        c if c != '\'' && !c.is_whitespace() => closed(at + 2),
        _ => None,
    }
}

/// Where the `(...)` beginning at `at` ends, as the half-open range of its contents; `None` where
/// `at` opens none and where it is never closed.
fn parens(chars: &[char], at: usize) -> Option<(usize, usize)> {
    if chars.get(at) != Some(&'(') {
        return None;
    }
    let mut depth = 0;
    let mut k = at;
    while k < chars.len() {
        match chars[k] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((at + 1, k));
                }
            }
            _ => {}
        }
        k += 1;
    }
    None
}

/// Whether the body of a `cfg` can be satisfied only by the test build: `test` alone, or `all`
/// with `test` among its conjuncts. `not(test)` can be satisfied by a real build of course, and
/// `any(test, unix)` can through its other arm — which is why a condition naming `test` among
/// others does not by itself mark test code.
fn cfg_needs_test(chars: &[char], from: usize, to: usize) -> bool {
    let body = &chars[from..to];
    // The word run beginning at `k` of `body`, and where that run ends.
    let word = |k: usize| {
        let end = body[k..]
            .iter()
            .position(|c| !(c.is_alphanumeric() || *c == '_'))
            .map_or(body.len(), |d| k + d);
        (body[k..end].iter().collect::<String>(), end)
    };
    let (head, end) = word(0);
    match head.as_str() {
        "test" => true,
        // `all` holds only where every conjunct holds, so one of them being `test` settles it.
        "all" => body[end..].iter().enumerate().any(|(k, c)| {
            *c == '(' && parens(body, end + k).is_some_and(|(a, b)| cfg_needs_test(body, a, b))
        }),
        _ => false,
    }
}

/// The length of the `#[cfg(...)]` at `at` when it marks test code, so the item under it is
/// skipped: `#[cfg(test)]` of course, and `#[cfg(all(test, unix))]` too, which holds nowhere a
/// test is not built. Nothing else.
fn cfg_test(chars: &[char], at: usize) -> Option<usize> {
    if !starts(chars, at, "#[cfg") {
        return None;
    }
    let open = at + "#[cfg".len();
    let (from, to) = parens(chars, open)?;
    cfg_needs_test(chars, from, to).then(|| to - at + ")".len())
}

/// Reads from `*i`. With `item`, reads one item (to its closing `}` or its `;`) and stops there;
/// without, reads to the end. A `#[cfg(test)]` met on the way has the item after it skipped.
fn scan(chars: &[char], i: &mut usize, out: &mut Vec<String>, item: bool) {
    let mut depth = 0usize;
    let mut braced = false;
    while *i < chars.len() {
        let c = chars[*i];
        if let Some(len) = cfg_test(chars, *i) {
            *i += len;
            // What follows is test code: read past it, collecting nothing.
            let mut skipped = Vec::new();
            scan(chars, i, &mut skipped, true);
            continue;
        }
        match c {
            '/' if chars.get(*i + 1) == Some(&'/') => {
                while *i < chars.len() && chars[*i] != '\n' {
                    *i += 1;
                }
            }
            '/' if chars.get(*i + 1) == Some(&'*') => {
                *i += 2;
                while *i + 1 < chars.len() && !(chars[*i] == '*' && chars[*i + 1] == '/') {
                    *i += 1;
                }
                *i += 2;
            }
            // A character literal, read whole: an escape inside it can hold a brace — '\u{7}' —
            // and a brace counted there ends a `#[cfg(test)]` skip where no item ended.
            '\'' => match quote(chars, *i) {
                Some(end) => *i = end,
                None => *i += 1,
            },
            // A raw string: its newlines and indentation are what it says, not a lost
            // continuation, so nothing here is one of the literals the rule holds.
            'r' if matches!(chars.get(*i + 1), Some('#') | Some('"'))
                && !chars.get(i.wrapping_sub(1)).is_some_and(|c| c.is_alphanumeric()) =>
            {
                let mut hashes = 0;
                let mut end = *i + 1;
                while chars.get(end) == Some(&'#') {
                    hashes += 1;
                    end += 1;
                }
                if chars.get(end) != Some(&'"') {
                    *i += 1;
                    continue;
                }
                end += 1;
                let closing: String =
                    std::iter::once('"').chain(std::iter::repeat_n('#', hashes)).collect();
                let rest: String = chars[end..].iter().collect();
                *i = end
                    + rest.find(&closing).map_or_else(
                        || rest.chars().count(),
                        |k| rest[..k].chars().count() + closing.chars().count(),
                    );
            }
            '"' => {
                let mut text = String::new();
                *i += 1;
                while *i < chars.len() && chars[*i] != '"' {
                    if chars[*i] == '\\' {
                        match chars.get(*i + 1) {
                            // A line continuation: the compiler takes the newline and the
                            // indentation after it out, and so do we — the sentence is one space.
                            Some('\n') => {
                                *i += 2;
                                while chars.get(*i).is_some_and(|c| c.is_whitespace()) {
                                    *i += 1;
                                }
                                continue;
                            }
                            // An escape for a space says what the character itself would say, so
                            // it is kept as one: the line a `\n` ends is where a run of spaces
                            // stops being a hole and becomes an indent.
                            Some('n') => text.push('\n'),
                            Some('r') => text.push('\r'),
                            Some('t') => text.push('\t'),
                            Some(c) => text.push(*c),
                            None => {}
                        }
                        *i += 2;
                        continue;
                    }
                    text.push(chars[*i]);
                    *i += 1;
                }
                out.push(text);
                *i += 1;
            }
            '{' => {
                depth += 1;
                braced = true;
                *i += 1;
            }
            '}' => {
                depth = depth.saturating_sub(1);
                *i += 1;
                if item && braced && depth == 0 {
                    return;
                }
            }
            ';' if item && depth == 0 => {
                *i += 1;
                return;
            }
            _ => *i += 1,
        }
    }
}

/// Every `.rs` file under `root`, `/`-separated, skipping build output and hidden folders — the
/// list for a tree with no git.
fn walked(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut folders: Vec<String> = vec![String::new()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(root.join(&folder)) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let rel = if folder.is_empty() { name.clone() } else { format!("{folder}/{name}") };
            match entry.file_type() {
                Ok(t) if t.is_dir() && !name.starts_with('.') && name != "target" && name != "node_modules" => folders.push(rel),
                Ok(t) if t.is_file() && name.ends_with(".rs") => out.push(rel),
                _ => {}
            }
        }
    }
    out.sort();
    out
}

/// Literals aligned in runs of spaces on purpose, excused by the file they sit in and how what
/// they say begins: no reading of the characters tells these apart from a lost `\` — one row of
/// an fstab says just what one says — and collapsing them would break what they align. Each
/// entry gives its reason, the way `check-agent-files` accepts a `-` with a reason.
const ALIGNED_ON_PURPOSE: &[(&str, &str, &str)] = &[
    ("apps/arcade/src/cli.rs", "usage:", "the `usage:` screen pads each option to its column"),
    ("crates/yantrik-egress/src/main.rs", "usage:", "the `usage:` screen pads each option to its column"),
    (
        "crates/yantrik-ui/src/wire/installer_disk.rs",
        "UUID=",
        "an `/etc/fstab` row aligns each of its columns",
    ),
];

/// Whether `said` is one of [`ALIGNED_ON_PURPOSE`] excused for `file`.
fn aligned_on_purpose(file: &str, said: &str) -> bool {
    ALIGNED_ON_PURPOSE
        .iter()
        .any(|(f, begins, _)| *f == file && said.starts_with(begins))
}

/// The whole check over the tree at `root`: one sentence per literal that says a hole.
pub fn check(root: &Path) -> Result<Vec<String>, String> {
    // The files git tracks; in a copy of the tree with no git (a build host's mirror), every
    // `.rs` file under the tree.
    let files = crate::workspace::tracked(root, &["*.rs"]).or_else(|_| Ok::<_, String>(walked(root)))?;
    let mut problems = Vec::new();
    for file in files {
        if !file.ends_with(".rs") || is_test(&file) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join(&file)) else { continue };
        for said in literals(&text) {
            if lost_its_continuation(&said) && !aligned_on_purpose(&file, &said) {
                let line = said
                    .split('\n')
                    .find(|l| !l.starts_with(' ') && !hole_runs(l).is_empty())
                    .unwrap_or(&said);
                problems.push(format!("{}: {line:?} (a lost line continuation?)", file));
            }
        }
    }
    Ok(problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_continuation_is_taken_out_and_a_comment_and_a_raw_string_are_not_said() {
        let kept = "let a = \"written and \\\n    your draft\";";
        assert_eq!(literals(kept), ["written and your draft".to_string()]);
        let lost = "let a = \"written and              your draft\";";
        assert!(literals(lost)[0].contains("   "));
        // A comment says nothing; an apostrophe in text is no one's character literal.
        assert_eq!(literals("// \"in a comment    \"\nlet c = '\"'; let s = \"x\";"), ["x".to_string()]);
        assert_eq!(literals("/* \"a   b\" */ let s = \"x\";"), ["x".to_string()]);
        // A raw string keeps its own newlines and indentation by design.
        assert_eq!(literals("let r = r#\"a   b\"#; let s = \"x\";"), ["x".to_string()]);
        // Test code is skipped, whether a block or one item, and what comes after it is read.
        let early = "#[cfg(test)]\nmod t {\n    fn f() { let a = \"a  {  b   c\"; }\n}\nfn g() { let s = \"after\"; }\n";
        assert_eq!(literals(early), ["after".to_string()]);
        let single = "#[cfg(test)]\nuse x::\"y\";\nfn g() { let s = \"after\"; }\n";
        assert_eq!(literals(single), ["after".to_string()]);
        // A `cfg` that a real build can satisfy marks nothing.
        let both = "#[cfg(not(test))]\nfn g() { let s = \"said   here\"; }\n";
        assert!(literals(both).iter().any(|t| t.contains("   ")), "{both:?}");
        // A lost continuation outside a `#[cfg(test)]` item is seen whatever the test module holds.
        let after = "#[cfg(test)]\nmod t {\n    fn f() { let a = \"in a test   here\"; }\n}\nfn g() { let s = \"written and              your draft\"; }\n";
        assert!(literals(after).iter().any(|t| t.contains("   ")), "{after:?}");
    }

    #[test]
    fn every_sentence_in_the_tree_keeps_its_line_continuations() {
        let problems = check(&crate::workspace::root()).expect("the check runs");
        assert!(problems.is_empty(), "\n{}", problems.join("\n"));
    }

    #[test]
    fn a_files_own_tests_are_its_own_business() {
        assert!(is_test("apps/arcade/src/library/tests.rs"));
        assert!(is_test("apps/container-manager/src/runtime_tests.rs"));
        assert!(is_test("crates/yantrik-ui/src/wire/tests.rs"));
        // The repo's own `tests/` holds the screen-preview app, aligned in runs of spaces.
        assert!(is_test("tests/ui-preview/src/main.rs"));
        assert!(is_test("tests/calendar-core/lib.rs"));
        assert!(!is_test("crates/yantrik-ui/src/wire/dock.rs"));
        assert!(!is_test("crates/yantrik-ui/src/testing.rs"));
    }

    #[test]
    fn a_hole_is_a_lost_continuation_and_an_indent_or_a_column_is_not() {
        // What the two drafts of the bug actually said.
        assert!(lost_its_continuation("written and              your draft"));
        assert!(!lost_its_continuation("written and your draft"));
        // A run of two is one space too few to be a join of two indented source lines.
        assert!(!lost_its_continuation("written and  your draft"));
        // An indent at the start of a line, and padding at the end of one: no word beyond the run.
        assert!(!lost_its_continuation("\n    {body}\n    {other}"));
        assert!(!lost_its_continuation("Name   \nValue"));
        // A label padded to its value, and a tab-indented line.
        assert!(!lost_its_continuation("\n  Edges:                 {edges}\n  GPU:  {gpu}"));
        assert!(!lost_its_continuation("\tcode here"));
        // A sentence may break after a colon; a word is a letter, a digit, or an apostrophe.
        assert!(lost_its_continuation("needs the person's leave:   start it from the Recipes screen"));
        assert!(lost_its_continuation("don't   stop"));
        // Punctuation that ends a word, and what opens the next one, count with it: a lost `\`
        // before a `(`-ed aside (`(known: {known:?})`) is as lost as one before a plain word.
        assert!(lost_its_continuation("gives a window              (known: x)"));
        assert!(lost_its_continuation("done.              Next time"));
        // A comma is neither: a column list pads after each of its commas.
        assert!(!lost_its_continuation("id,     name,     size"));
        // A table's row does say holes — it lives in test code, or is excused by name.
        assert!(lost_its_continuation("PID     Type       RSS"));
        // Two holes are two lost continuations.
        assert!(lost_its_continuation("first line has   a hole\nsecond has   one too"));
        // SQL aligned on a keyword says nothing lost.
        assert!(!lost_its_continuation(
            "SELECT id,   name\n  FROM recipes\n WHERE id =   ?1"
        ));
    }

    #[test]
    fn a_run_aligned_on_purpose_is_excused_by_name_with_its_reason() {
        for (file, begins, reason) in ALIGNED_ON_PURPOSE.iter().copied() {
            assert!(reason.split_whitespace().count() >= 3, "{file} says {reason:?}");
            assert!(aligned_on_purpose(file, &format!("{begins}x          y")));
        }
        let fstab = "UUID={uuid}  /boot       ext4  defaults,noatime  0  2";
        assert!(lost_its_continuation(fstab), "an fstab row does say a hole");
        // A lookalike in no excused file is still found.
        assert!(!aligned_on_purpose("crates/yantrik-ui/src/wire/dock.rs", fstab));
    }
}
