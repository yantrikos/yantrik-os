//! What a person reads says "mind", not "harness", and never counts "tokens".
//!
//! design/experience-final-2026-10-10.md, "One vocabulary for everyone": both audiences read the
//! same plain words, and internal terms appear only inside Details, beside the plain word. The
//! punch list of 12 Oct 2026 found "Harnesses" in the Settings sidebar and "0 tokens" on the
//! Accounts page, so this keeps them from coming back.
//!
//! What counts as visible, narrowly so that the check is reliable rather than clever: a string
//! literal assigned to a property a person reads (`text`, `label`, `title`, `description`, ...),
//! on the property's own line or on the lines that continue its value (a `? "..." : "..."`
//! conditional). Identifiers, ids compared in code (`h.state == "attached"`), struct fields named
//! after the internal word and comments are not looked at. Allowed: anything inside an element
//! whose type name contains `Detail` (`YDetails`, the Agents screen's `DetailRow`), because that is
//! where the design puts the internal words for the expert, and anything inside a block opened on
//! a line that names a details flag (`if root.details-open : VerticalLayout {`), which is the
//! same Details area drawn by hand.

use std::path::{Path, PathBuf};

fn slint_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.unwrap().path();
        if path.is_dir() {
            slint_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "slint") {
            out.push(path);
        }
    }
}

/// The properties whose string values end up on screen or read aloud.
const VISIBLE: &[&str] = &[
    "text",
    "label",
    "title",
    "description",
    "placeholder-text",
    "placeholder",
    "tooltip",
    "accessible-label",
    "accessible-description",
    "subtitle",
    "hint",
    "message",
    "value",
    "heading",
];

/// A line split into its code (comments removed) and the string literals in it, each with the
/// byte offset in the code where it starts. A `//` inside a string is not a comment.
fn lex(line: &str) -> (String, Vec<(usize, String)>) {
    let mut code = String::new();
    let mut strings = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'/') {
            break;
        }
        if c == '"' {
            let start = code.len();
            let mut s = String::new();
            code.push('"');
            while let Some(c) = chars.next() {
                code.push(c);
                match c {
                    '\\' => {
                        if let Some(n) = chars.next() {
                            code.push(n);
                            s.push(n);
                        }
                    }
                    '"' => break,
                    _ => s.push(c),
                }
            }
            strings.push((start, s));
        } else {
            code.push(c);
        }
    }
    (code, strings)
}

/// The property a string at `at` in `code` is assigned to, if the line names one before it:
/// the last `name:` ahead of the string, skipping `?`/`:` of a conditional.
fn property_before(code: &str, at: usize) -> Option<String> {
    let head = &code[..at];
    let bytes = head.as_bytes();
    let mut i = head.len();
    while i > 0 {
        i -= 1;
        if bytes[i] == b':' {
            let name: String = head[..i]
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            // `a ? "x" : "y"` — a colon with no name before it is the conditional's.
            if !name.is_empty() && !head[..i].ends_with(' ') {
                return Some(name);
            }
        }
    }
    None
}

/// Whether the code ahead of a `{` opens a Details area: an element whose type name has `Detail`
/// in it (`YDetails`, `DetailRow`), or a block shown only while a `details-open` flag is set.
fn opens_details(before: &str) -> bool {
    let element = before.trim_end().rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')).next().unwrap_or("");
    element.contains("Detail") || before.contains("details-open")
}

fn is_internal(word_source: &str) -> bool {
    let lower = word_source.to_lowercase();
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| matches!(w, "harness" | "harnesses" | "token" | "tokens"))
}

/// Every (line, string) in `source` that a person reads and that uses an internal word.
fn leaks(source: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    // Brace depth at which each open `...Detail... {` element started.
    let mut depth = 0usize;
    let mut details: Vec<usize> = Vec::new();
    // The property whose value this line continues, when the previous line did not end it.
    let mut continuing: Option<String> = None;
    for (n, line) in source.lines().enumerate() {
        let (code, strings) = lex(line);
        let trimmed = code.trim();
        let starts_property = trimmed
            .split_once(':')
            .map(|(name, _)| !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(false);
        if starts_property {
            continuing = None;
        }
        // Walk the line once, outside string literals: track the Details blocks it opens and
        // closes, and note for each string whether it sits inside one, so a one-line
        // `YDetails { text: "..."; }` counts as inside.
        let mut inside_at: Vec<bool> = Vec::new();
        let mut in_string = false;
        let mut escaped = false;
        let mut before = String::new();
        for c in code.chars() {
            if in_string {
                match (escaped, c) {
                    (true, _) => escaped = false,
                    (false, '\\') => escaped = true,
                    (false, '"') => in_string = false,
                    _ => {}
                }
                continue;
            }
            match c {
                '"' => {
                    in_string = true;
                    inside_at.push(!details.is_empty());
                }
                '{' => {
                    if opens_details(&before) {
                        details.push(depth);
                    }
                    depth += 1;
                }
                '}' => {
                    depth = depth.saturating_sub(1);
                    if details.last() == Some(&depth) {
                        details.pop();
                    }
                }
                _ => {}
            }
            before.push(c);
        }
        for (i, (at, s)) in strings.iter().enumerate() {
            let property = property_before(&code, *at).or_else(|| continuing.clone());
            let visible = property.as_deref().is_some_and(|p| VISIBLE.contains(&p));
            let inside_details = inside_at.get(i).copied().unwrap_or(false);
            if visible && !inside_details && is_internal(s) {
                found.push((n + 1, s.clone()));
            }
        }
        // A property value that runs on to the next line.
        if starts_property {
            let name = trimmed.split_once(':').unwrap().0.to_string();
            if !trimmed.ends_with(';') && !trimmed.ends_with('{') {
                continuing = Some(name);
            }
        } else if continuing.is_some() && trimmed.ends_with(';') {
            continuing = None;
        }
    }
    found
}

#[test]
fn no_visible_string_says_harness_or_token() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    slint_files(&root.join("ui"), &mut files);
    assert!(files.len() > 20, "found only {} .slint files; is the path right?", files.len());

    let mut problems = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap();
        for (line, s) in leaks(&source) {
            problems.push(format!("{}:{line}: \"{s}\"", file.strip_prefix(root).unwrap_or(file).display()));
        }
    }
    assert!(
        problems.is_empty(),
        "visible text uses an internal word; say \"mind\" for a harness and say what was used \
         in plain words for tokens, or put the internal word inside a YDetails \
         (design/experience-final-2026-10-10.md, \"One vocabulary for everyone\"):\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn the_scanner_sees_visible_internal_words_and_nothing_else() {
    let sidebar = "{ icon: Icons.companion, label: \"Harnesses\", id: 8 },\n";
    assert_eq!(leaks(sidebar), [(1, "Harnesses".to_string())]);

    let conditional = "Text {\n    text: root.ok\n        ? \"fine\"\n        : \"0 tokens\";\n}\n";
    assert_eq!(leaks(conditional), [(4, "0 tokens".to_string())]);

    // Inside Details, an id compared in code, a comment and a struct field are all fine.
    let details = "YDetails {\n    text: \"harness manifest in /opt\";\n}\n";
    assert!(leaks(details).is_empty());
    let row = "DetailRow { label: \"Tokens\"; value: x; }\n";
    assert!(leaks(row).is_empty());
    let compared = "visible: root.kind == \"harness\";\n";
    assert!(leaks(compared).is_empty());
    let comment = "// a harness says \"tokens\"\ntext: \"Minds\";\n";
    assert!(leaks(comment).is_empty());
    let by_hand = "if root.details-open : VerticalLayout {\n    Text {\n        text: \"named by the token\";\n    }\n}\n";
    assert!(leaks(by_hand).is_empty());
    // A field that happens to be called `detail` is not a Details area.
    let field = "if root.step.detail != \"\" : Text {\n    text: \"0 tokens\";\n}\n";
    assert_eq!(leaks(field), [(2, "0 tokens".to_string())]);
    let after = "YDetails { text: \"harness\"; }\nText { text: \"harness\"; }\n";
    assert_eq!(leaks(after), [(2, "harness".to_string())]);
}
