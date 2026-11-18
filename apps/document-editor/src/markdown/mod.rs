//! The Markdown the document is: its outline, its counts, and the pure string work behind find,
//! replace, the format buttons and the HTML export. Nothing here knows about Slint or a file, so
//! each function is callable from a test with no desktop.

mod export;
mod find;
mod format;
#[cfg(test)]
mod tests;

pub use export::to_html;
pub use find::{matches, replace, replace_all};
pub use format::{format, format_link, Format};

/// One ATX heading, and where it starts in the text.
///
/// `offset` is a byte offset into the document, which is what the editing surface's
/// `set-selection-offsets` takes — so clicking a heading in the outline moves the real caret
/// rather than scrolling to an approximation of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub title: String,
    pub level: i32,
    pub offset: usize,
    pub line: usize,
}

/// The headings of a Markdown document, in document order.
///
/// Fenced code blocks are skipped: a `# comment` inside ``` is a comment in some language, not a
/// section of the document, and an outline that lists it is describing the wrong thing.
pub fn outline(text: &str) -> Vec<Heading> {
    let mut headings = Vec::new();
    let mut offset = 0usize;
    let mut fenced = false;
    for (line_number, line) in text.split('\n').enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
        } else if !fenced {
            let hashes = trimmed.chars().take_while(|c| *c == '#').count();
            let rest = &trimmed[hashes..];
            if (1..=6).contains(&hashes) && (rest.is_empty() || rest.starts_with(' ')) {
                headings.push(Heading {
                    title: rest.trim().trim_end_matches('#').trim_end().to_string(),
                    level: hashes as i32,
                    offset,
                    line: line_number + 1,
                });
            }
        }
        offset += line.len() + 1;
    }
    headings
}

pub fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Characters, not bytes. The old status bar reported `text.len()`, which is the byte length and
/// overstates every document with an accent in it.
pub fn char_count(text: &str) -> usize {
    text.chars().count()
}

/// How many characters of `text` the file does not have.
///
/// The characters between the longest common prefix and the longest common suffix of the two,
/// which for an ordinary edit is the edit itself rather than the whole document. Characters and
/// not bytes, for the same reason the status bar counts characters.
pub fn unsaved_chars(text: &str, baseline: &str) -> usize {
    let a: Vec<char> = text.chars().collect();
    let b: Vec<char> = baseline.chars().collect();
    let mut head = 0;
    while head < a.len() && head < b.len() && a[head] == b[head] {
        head += 1;
    }
    let mut tail = 0;
    while tail < a.len() - head
        && tail < b.len() - head
        && a[a.len() - 1 - tail] == b[b.len() - 1 - tail]
    {
        tail += 1;
    }
    a.len() - head - tail
}
