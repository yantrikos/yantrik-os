//! The format buttons: Markdown syntax put round the selection, or at the start of its lines.

use crate::limits::validate;

/// What a formatting button does to the text, once the document is Markdown.
///
/// Every one of these is an insertion of Markdown syntax around the selection or at the start of
/// the lines it touches, which is why they can be real: the format the buttons describe is the
/// format the file is in. Underline and highlight are missing on purpose — Markdown has neither,
/// so those two buttons have been taken off the screen rather than made to write a syntax that
/// no reader of a `.md` file would render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Bold,
    Italic,
    Strikethrough,
    InlineCode,
    Heading(u8),
    Bullet,
    Checklist,
    Quote,
    CodeBlock,
    Divider,
}

/// The text after a formatting button, and where the selection should now be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

/// Snap an offset onto a character boundary, inside the text.
///
/// The offsets come from the editing surface as byte positions, and a caret sitting in the
/// middle of a multi-byte character would panic a slice. Walking back to the boundary is the
/// only behaviour that is both safe and unsurprising.
fn boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn wrap_markers(what: Format) -> Option<&'static str> {
    match what {
        Format::Bold => Some("**"),
        Format::Italic => Some("*"),
        Format::Strikethrough => Some("~~"),
        Format::InlineCode => Some("`"),
        _ => None,
    }
}

fn line_prefix(what: Format) -> Option<String> {
    match what {
        Format::Heading(n) => Some(format!("{} ", "#".repeat(n.clamp(1, 6) as usize))),
        Format::Bullet => Some("- ".into()),
        Format::Checklist => Some("- [ ] ".into()),
        Format::Quote => Some("> ".into()),
        _ => None,
    }
}

/// Apply a formatting button to `text` over the selection `start..end`.
///
/// Each of these toggles: pressing Bold on text that is already bold takes the markers off, and
/// pressing H2 on a line that is already an H2 makes it a paragraph again. A button that can
/// only ever add is a button you cannot undo without hunting for the characters it inserted.
pub fn format(text: &str, start: usize, end: usize, what: Format) -> Result<Edit, String> {
    let (start, end) = {
        let (a, z) = if start <= end { (start, end) } else { (end, start) };
        (boundary(text, a), boundary(text, z))
    };

    if let Some(marker) = wrap_markers(what) {
        return Ok(wrap(text, start, end, marker));
    }

    if let Some(prefix) = line_prefix(what) {
        return prefixed(text, start, end, &prefix, what);
    }

    match what {
        Format::CodeBlock => {
            let (from, to) = line_span(text, start, end);
            let body = &text[from..to];
            let replacement = format!("```\n{body}\n```");
            let out = splice(text, from, to, &replacement)?;
            // Inside the fence, where the next keystroke belongs.
            Ok(Edit { start: from + 4, end: from + 4 + body.len(), text: out })
        }
        Format::Divider => {
            let (from, _) = line_span(text, start, end);
            let out = splice(text, from, from, "---\n")?;
            Ok(Edit { start: from + 4, end: from + 4, text: out })
        }
        _ => Err("That formatting is not one this document knows.".into()),
    }
}

/// A link, which is the one format that carries something of its own.
///
/// With a URL it writes `[selection](url)`. Without one it writes the placeholder and selects
/// it, so the next thing typed is the address — the ribbon button passes no URL, because there
/// is nowhere on this screen to have typed one first.
pub fn format_link(text: &str, start: usize, end: usize, url: &str) -> Result<Edit, String> {
    let (start, end) = {
        let (a, z) = if start <= end { (start, end) } else { (end, start) };
        (boundary(text, a), boundary(text, z))
    };
    let label = &text[start..end];
    let address = if url.trim().is_empty() { "url" } else { url.trim() };
    let replacement = format!("[{label}]({address})");
    let out = splice(text, start, end, &replacement)?;
    let address_at = start + 1 + label.len() + 2;
    Ok(if url.trim().is_empty() {
        Edit { start: address_at, end: address_at + address.len(), text: out }
    } else {
        let after = start + replacement.len();
        Edit { start: after, end: after, text: out }
    })
}

fn splice(text: &str, from: usize, to: usize, replacement: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len() + replacement.len());
    out.push_str(&text[..from]);
    out.push_str(replacement);
    out.push_str(&text[to..]);
    validate(&out)?;
    Ok(out)
}

/// The whole of every line the selection touches.
fn line_span(text: &str, start: usize, end: usize) -> (usize, usize) {
    let from = text[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let to = text[end..].find('\n').map(|i| end + i).unwrap_or(text.len());
    (from, to)
}

fn wrap(text: &str, start: usize, end: usize, marker: &str) -> Edit {
    let n = marker.len();
    // Already wrapped, with the markers outside the selection: take them off.
    if start >= n
        && end + n <= text.len()
        && &text[start - n..start] == marker
        && &text[end..end + n] == marker
    {
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..start - n]);
        out.push_str(&text[start..end]);
        out.push_str(&text[end + n..]);
        return Edit { start: start - n, end: end - n, text: out };
    }
    // Already wrapped, with the markers inside the selection: take them off too, so a
    // double-click that grabs `**bold**` behaves the same as a drag that grabs `bold`.
    if end - start >= 2 * n && text[start..end].starts_with(marker) && text[start..end].ends_with(marker) {
        let inner = &text[start + n..end - n];
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..start]);
        out.push_str(inner);
        out.push_str(&text[end..]);
        return Edit { start, end: start + inner.len(), text: out };
    }
    let mut out = String::with_capacity(text.len() + 2 * n);
    out.push_str(&text[..start]);
    out.push_str(marker);
    out.push_str(&text[start..end]);
    out.push_str(marker);
    out.push_str(&text[end..]);
    Edit { start: start + n, end: end + n, text: out }
}

/// Put a line prefix on every line the selection touches, or take it off every one that has it.
fn prefixed(text: &str, start: usize, end: usize, prefix: &str, what: Format) -> Result<Edit, String> {
    let (from, to) = line_span(text, start, end);
    let lines: Vec<&str> = text[from..to].split('\n').collect();

    // Off, only when every line already carries exactly this. A mixed selection gets the prefix
    // applied throughout, which is what a person pressing "bullet list" over mixed lines means.
    let all_have = lines.iter().all(|l| line_has(l, prefix, what));
    let rewritten: Vec<String> = lines
        .iter()
        .map(|l| {
            let bare = strip(l, what);
            if all_have {
                bare.to_string()
            } else {
                format!("{prefix}{bare}")
            }
        })
        .collect();
    let replacement = rewritten.join("\n");
    let out = splice(text, from, to, &replacement)?;
    Ok(Edit { start: from, end: from + replacement.len(), text: out })
}

/// Does this line already carry this exact mark?
///
/// A checklist item starts `- ` too, so a plain bullet does not count itself as one and — more
/// usefully — pressing Bullet on a checklist item converts it rather than clearing it.
fn line_has(line: &str, prefix: &str, what: Format) -> bool {
    match what {
        Format::Checklist => line.starts_with("- [ ] ") || line.starts_with("- [x] "),
        Format::Bullet => line.starts_with("- ") && !line.starts_with("- ["),
        _ => line.starts_with(prefix),
    }
}

/// The line without whichever mark of this family it carries, so switching H2 to H3 does not
/// leave `## ### `, and a bullet does not become `- - `.
fn strip(line: &str, what: Format) -> &str {
    match what {
        Format::Heading(_) => {
            let hashes = line.chars().take_while(|c| *c == '#').count();
            if (1..=6).contains(&hashes) {
                line[hashes..].trim_start_matches(' ')
            } else {
                line
            }
        }
        Format::Bullet | Format::Checklist => line
            .strip_prefix("- [ ] ")
            .or_else(|| line.strip_prefix("- [x] "))
            .or_else(|| line.strip_prefix("- "))
            .unwrap_or(line),
        Format::Quote => line.strip_prefix("> ").unwrap_or(line),
        _ => line,
    }
}
