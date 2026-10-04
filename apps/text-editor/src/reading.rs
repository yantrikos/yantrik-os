//! Reading a document to a mind one page at a time.
//!
//! VM 520, 4 October: a mind asked to read ~/mdg/MDG-spec.md (9,557 bytes) could not. `describe`
//! handed back the open tab's text cut at 4,000 characters with nothing saying so, and there was
//! no action that read a document at all, so the mind concluded the editor could not show it a
//! file's contents and gave up. Minds keep about 4,000 characters of a `safe` read in their work
//! log (and 900 of anything graded higher), so `read` is `safe` and every page is sized, as
//! serialized JSON, to be kept whole; the answer names the exact call for the next page. The
//! shape follows Notes' `read_notes`.

/// The most a whole `read` answer may be, as serialized JSON: under a mind's 4,000-character
/// keep for a safe read, with room for the call's own wrapping. The same figure as Notes'.
pub const READ_ANSWER: usize = 3_400;

/// What `describe` hands back of the tab in front, in characters.
pub const DESCRIBE_CHARS: usize = 4_000;

/// Room kept for a `line_cut` note on top of the measured envelope.
const CUT_NOTE: usize = 140;

/// The least text a page carries even when a long file path eats the envelope; a page that could hold
/// nothing would page forever.
const MIN_TEXT: usize = 1_000;

/// How long `s` is once written inside a JSON string, quotes left off.
fn json_len(s: &str) -> usize {
    serde_json::to_string(s).map(|j| j.len() - 2).unwrap_or(s.len())
}

/// The longest leading part of `line` whose JSON form fits in `budget` bytes.
fn cut_to(line: &str, budget: usize) -> &str {
    let mut used = 0;
    let mut buf = [0u8; 4];
    for (at, c) in line.char_indices() {
        used += json_len(c.encode_utf8(&mut buf));
        if used > budget {
            return &line[..at];
        }
    }
    line
}

/// One page of `text`, starting at 1-based `from_line`.
///
/// `path` is the document as this caller may see it, and `call_args` what the next call has to
/// repeat before `from_line` (`tab 2 and `), so `how_to_see_the_rest` can be
/// followed verbatim. Lines are counted as `describe.lines` counts them: one more than the
/// newlines. A page always carries at least one line; a line longer than a page is cut, and
/// `line_cut` says so rather than let the cut pass for the line's end.
pub fn page(text: &str, path: serde_json::Value, from_line: usize, call_args: &str) -> serde_json::Value {
    let lines: Vec<&str> = text.split('\n').collect();
    let total = lines.len();
    let from = from_line.max(1);
    if from > total {
        return serde_json::json!({
            "path": path,
            "lines_total": total,
            "from_line": from,
            "to_line": total,
            "text": "",
            "how_to_see_the_rest": format!(
                "from_line {from} is past the end of these {total} lines: call read with \
                 {call_args}from_line 1 to start again"
            ),
        });
    }
    // The envelope measured with the longest hint this document could need.
    let worst_hint = format!("lines {total}–{total} of {total}: call read with {call_args}from_line {total}");
    let envelope = serde_json::json!({
        "path": path, "lines_total": total, "from_line": total, "to_line": total,
        "text": "", "how_to_see_the_rest": worst_hint,
    });
    let budget = READ_ANSWER.saturating_sub(envelope.to_string().len() + CUT_NOTE).max(MIN_TEXT);

    let mut out = String::new();
    let mut used = 0;
    let mut to = from - 1;
    let mut cut = None;
    for (i, line) in lines.iter().enumerate().skip(from - 1) {
        let n = i + 1;
        let first = to < from;
        // A newline inside a JSON string is the two bytes `\n`.
        let size = json_len(line) + if first { 0 } else { 2 };
        if used + size > budget {
            if first {
                let kept = cut_to(line, budget);
                cut = Some(format!(
                    "line {n} is {} characters long, more than one page holds; only its first {} \
                     are here",
                    line.chars().count(),
                    kept.chars().count()
                ));
                out.push_str(kept);
                to = n;
            }
            break;
        }
        if !first {
            out.push('\n');
        }
        out.push_str(line);
        used += size;
        to = n;
    }
    let mut answer = serde_json::json!({
        "path": path,
        "lines_total": total,
        "from_line": from,
        "to_line": to,
        "text": out,
        "how_to_see_the_rest": if to < total {
            format!("lines {from}–{to} of {total}: call read with {call_args}from_line {}", to + 1)
        } else {
            String::new()
        },
    });
    if let Some(note) = cut {
        answer["line_cut"] = serde_json::json!(note);
    }
    answer
}

/// What `describe.content` holds: the first [`DESCRIBE_CHARS`] characters.
pub fn describe_content(text: &str) -> String {
    text.chars().take(DESCRIBE_CHARS).collect()
}

/// When `describe.content` is cut, the `read` call that carries on from the line it stops in.
///
/// A cut that said nothing is half of why the mind on VM 520 gave up: it was shown 42% of the
/// spec and nothing told it there was more, or how to get it.
pub fn describe_cut(text: &str) -> Option<String> {
    let (at, _) = text.char_indices().nth(DESCRIBE_CHARS)?;
    let line = text[..at].bytes().filter(|b| *b == b'\n').count() + 1;
    Some(format!("read from_line {line}"))
}
