//! A deck written out as text a person or a mind can read without this app, and the name a deck
//! is given when nobody has chosen one.

use super::Deck;

/// A file name made out of a deck's title: lower case, words joined by hyphens, nothing that
/// needs quoting in a shell.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('-');
            }
            gap = false;
            out.extend(c.to_lowercase());
        } else {
            gap = true;
        }
        if out.len() >= 60 {
            break;
        }
    }
    if out.is_empty() {
        "untitled".to_string()
    } else {
        out
    }
}

impl Deck {
    /// The deck as Markdown: one `#` heading per slide, `---` between them, speaker notes as a
    /// blockquote.
    ///
    /// Cheap, diffable, and a thing a mind can read and write without this app. The deck's own
    /// name goes in an HTML comment because Markdown has nowhere else to put it that is not
    /// also a slide.
    pub fn to_markdown(&self) -> String {
        let mut out = format!("<!-- yPresent deck: {} -->\n\n", self.content().title);
        for (i, slide) in self.slides().iter().enumerate() {
            if i > 0 {
                out.push_str("\n---\n\n");
            }
            out.push_str(&format!("# {}\n", slide.title));
            let body = slide.body.trim_end();
            if !body.trim().is_empty() {
                out.push('\n');
                out.push_str(body);
                out.push('\n');
            }
            let notes = slide.notes.trim_end();
            if !notes.trim().is_empty() {
                out.push('\n');
                for line in notes.lines() {
                    out.push_str("> ");
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        out
    }

    /// Titles only, numbered. What a person wants when they are checking the shape of a talk
    /// rather than reading it.
    pub fn to_outline(&self) -> String {
        let mut out = format!("# {}\n\n", self.content().title);
        for (i, slide) in self.slides().iter().enumerate() {
            out.push_str(&format!("{}. {}\n", i + 1, slide.title));
        }
        out
    }
}
