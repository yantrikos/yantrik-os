//! What an HTML mail body looks like as text a person can read.
//!
//! The reading pane shows a mail body in a proportional font with word wrap of its own. The
//! default HTML-to-text conversion assumed the opposite reader: a fixed-width terminal, 80
//! columns. So a table-laid-out newsletter arrived drawn in box characters, links came with
//! `[1]` footnotes pointing at a URL list nobody asked for, every logo became `[Subreddit
//! Icon]`, and sentences were hard-wrapped at column 80 so the pane showed them broken in
//! places the sender never chose.
//!
//! This crate is the one rule for what that conversion is instead, shared by the email service
//! (which converts HTML-only mail for the reading pane) and the companion (which converts it
//! for notifications and summaries), so the two cannot drift apart:
//!
//! - no table borders — layout tables, which is what almost all marketing mail is made of,
//!   become plain paragraphs in the order they appear;
//! - links as their own text, with nothing after them and no footnote list at the end;
//! - images as nothing at all — an `alt` like "Subreddit Icon" names a picture, not content;
//! - no hard wrapping — every line stays whole and the window wraps it;
//! - runs of blank lines and trailing whitespace collapsed, so a mail padded with empty table
//!   cells reads as paragraphs, not as gaps.
//!
//! It is only ever asked about the HTML part of a mail that has no plain-text part: a
//! multipart/alternative message prefers its own `text/plain`, which its sender wrote for
//! exactly this reader, and that preference belongs to the callers, not here.

use html2text::config::with_decorator;
use html2text::render::text_renderer::{TaggedLine, TextDecorator};

/// The width the renderer is told to wrap to, i.e. effectively "do not wrap".
///
/// The renderer refuses a width of zero, and the largest possible width overflows its own
/// arithmetic when it adds positions to it, so this is a finite width no mail line will ever
/// reach rather than a limit anyone will notice.
const NO_WRAP_WIDTH: usize = 100_000;

/// Turn an HTML mail body into text for the reading pane.
///
/// Never fails and never returns markup: if the render itself is refused — which the renderer
/// only does for a width of zero, not for any input — the words of the source survive with
/// their whitespace collapsed, because a mail that reads roughly still beats a mail that does
/// not arrive.
pub fn readable_text(html: &str) -> String {
    let rendered = with_decorator(ReadingDecorator)
        // Layout tables traverse as one column of cells: no borders drawn, every cell its own
        // paragraph, in document order.
        .raw_mode(true)
        // Never answer TooNarrow; a line longer than the width is exactly what this crate is
        // for, since the window does the wrapping.
        .allow_width_overflow()
        .string_from_read(html.as_bytes(), NO_WRAP_WIDTH);
    let text = rendered
        .unwrap_or_else(|_| html.split_whitespace().collect::<Vec<_>>().join(" "));
    tidy(&text)
}

/// Trim trailing whitespace from every line, replace no-break spaces with plain ones, collapse
/// runs of blank lines to one, and trim the whole.
///
/// HTML mail is padded with empty cells and spacer elements; the renderer hands each of those
/// back as blank lines, and without this pass a newsletter reads as one sentence per screen.
fn tidy(rendered: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in rendered.split('\n') {
        let cleaned: String = line
            .chars()
            .map(|c| if c == '\u{a0}' { ' ' } else { c })
            .collect();
        let cleaned = cleaned.trim_end();
        if cleaned.is_empty() {
            // Keep at most one blank line in a row: the gap between paragraphs, not the shape
            // of the sender's layout.
            if lines.last().is_some_and(|l| !l.is_empty()) {
                lines.push(String::new());
            }
        } else {
            lines.push(cleaned.to_string());
        }
    }
    lines.join("\n").trim().to_string()
}

/// How markup is dressed for the reading pane: it is not.
///
/// html2text's own decorators write for a terminal — `PlainDecorator` brackets every link and
/// collects footnotes, `TrivialDecorator` still leaves image `alt` text inline — so this one
/// renders nothing for emphasis, links, images and headings and keeps only the markers that
/// carry meaning a proportional font cannot show: `> ` before quoted lines and list numbers.
#[derive(Clone, Copy, Debug, Default)]
struct ReadingDecorator;

impl TextDecorator for ReadingDecorator {
    type Annotation = ();

    fn decorate_link_start(&mut self, _url: &str) -> (String, ()) {
        (String::new(), ())
    }
    fn decorate_link_end(&mut self) -> String {
        String::new()
    }
    fn decorate_em_start(&self) -> (String, ()) {
        (String::new(), ())
    }
    fn decorate_em_end(&self) -> String {
        String::new()
    }
    fn decorate_strong_start(&self) -> (String, ()) {
        (String::new(), ())
    }
    fn decorate_strong_end(&self) -> String {
        String::new()
    }
    fn decorate_strikeout_start(&self) -> (String, ()) {
        (String::new(), ())
    }
    fn decorate_strikeout_end(&self) -> String {
        String::new()
    }
    fn decorate_code_start(&self) -> (String, ()) {
        (String::new(), ())
    }
    fn decorate_code_end(&self) -> String {
        String::new()
    }
    fn decorate_preformat_first(&self) -> () {}
    fn decorate_preformat_cont(&self) -> () {}
    /// Nothing for a picture, not even its `alt`: an alt is written for a screen reader at the
    /// sender's whim — "Subreddit Icon", "spacer", a logo's filename — and none of that is the
    /// mail's words. A logo that matters says so in text nearby.
    fn decorate_image(&mut self, _src: &str, _title: &str) -> (String, ()) {
        (String::new(), ())
    }
    fn header_prefix(&self, _level: usize) -> String {
        String::new()
    }
    fn quote_prefix(&self) -> String {
        "> ".to_string()
    }
    fn unordered_item_prefix(&self) -> String {
        "- ".to_string()
    }
    fn ordered_item_prefix(&self, i: i64) -> String {
        format!("{i}. ")
    }
    fn decorate_superscript_start(&self) -> (String, ()) {
        (String::new(), ())
    }
    fn decorate_superscript_end(&self) -> String {
        String::new()
    }
    fn make_subblock_decorator(&self) -> Self {
        *self
    }
    /// No footnotes: the link list at the end of the document is exactly the `[1]: <url>`
    /// block the reading pane should not show.
    fn finalise(&mut self, _links: Vec<String>) -> Vec<TaggedLine<()>> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::readable_text;

    /// A notification mail laid out the way marketing mail is actually laid out: nested tables
    /// for the frame, a logo with an alt, a linked title, a paragraph, and a footer row of
    /// linked cells. This is the shape that arrived in the reading pane drawn in box
    /// characters with `[1]` footnotes under it (#275).
    const NEWSLETTER: &str = r#"<html><body>
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0">
  <tr>
    <td align="center">
      <table width="600" cellpadding="0" cellspacing="0">
        <tr>
          <td width="24"><a href="https://reddit.example/r/yantrikdb"><img src="https://reddit.example/static/icon.png" alt="Subreddit Icon" width="24" height="24"></a></td>
          <td><a href="https://reddit.example/r/yantrikdb" style="font-weight:bold">yantrikdb</a></td>
        </tr>
      </table>
    </td>
  </tr>
  <tr>
    <td align="center">
      <table width="600" cellpadding="8" cellspacing="0">
        <tr>
          <td>
            <a href="https://reddit.example/comments/1abc"><b>New post: Yantrik OS grows a reading pane that reads mail the way people do</b></a>
            <p>Someone posted this to r/yantrikdb yesterday evening, and the discussion about the new reading pane has already grown well past twenty replies and keeps going.</p>
          </td>
        </tr>
        <tr>
          <td>
            <table cellpadding="0" cellspacing="0">
              <tr>
                <td width="120"><a href="https://reddit.example/comments/1abc/up">24 upvotes</a></td>
                <td width="120"><a href="https://reddit.example/comments/1abc">21 comments</a></td>
              </tr>
            </table>
          </td>
        </tr>
      </table>
    </td>
  </tr>
</table>
</body></html>"#;

    #[test]
    fn newsletter_has_no_box_drawing() {
        let out = readable_text(NEWSLETTER);
        for c in "─│┼┬┐└├┤┴┘━┃┏┓┗┛".chars() {
            assert!(!out.contains(c), "found table border {c:?} in:\n{out}");
        }
    }

    #[test]
    fn newsletter_has_no_link_footnotes() {
        let out = readable_text(NEWSLETTER);
        // Every bracket the old conversion produced was a footnote reference (`[1]`), a link
        // target (`][n]`) or an image alt (`[Subreddit Icon]`). Nothing in this mail's words
        // contains a bracket, so nothing in the reading should either.
        assert!(!out.contains('['), "found a bracket in:\n{out}");
        assert!(!out.contains(']'), "found a bracket in:\n{out}");
        assert!(!out.contains("https://"), "found a URL in:\n{out}");
    }

    #[test]
    fn newsletter_drops_picture_alts() {
        let out = readable_text(NEWSLETTER);
        assert!(!out.contains("Subreddit Icon"), "image alt leaked into:\n{out}");
    }

    #[test]
    fn newsletter_keeps_words_in_order() {
        let out = readable_text(NEWSLETTER);
        let title = "New post: Yantrik OS grows a reading pane that reads mail the way people do";
        let body = "Someone posted this to r/yantrikdb yesterday evening";
        let title_at = out.find(title).unwrap_or_else(|| panic!("title missing from:\n{out}"));
        let body_at = out.find(body).unwrap_or_else(|| panic!("body missing from:\n{out}"));
        assert!(title_at < body_at, "title and body out of order in:\n{out}");
        assert!(out.contains("yantrikdb"), "link text missing from:\n{out}");
        assert!(out.contains("24 upvotes"), "footer cell missing from:\n{out}");
        assert!(out.contains("21 comments"), "footer cell missing from:\n{out}");
    }

    #[test]
    fn newsletter_is_not_hard_wrapped_at_80() {
        let out = readable_text(NEWSLETTER);
        // The body sentence is well over 80 columns; if the conversion wrapped at 80 it could
        // not survive on one line, and the reading pane would show it broken mid-sentence.
        let sentence = "the discussion about the new reading pane has already grown well past twenty replies and keeps going.";
        assert!(
            out.lines().any(|line| line.contains(sentence)),
            "sentence was broken across lines in:\n{out}"
        );
    }

    #[test]
    fn blank_runs_and_trailing_space_collapse() {
        let out = readable_text("<p>a</p><br><br><br><br><p>b&nbsp;&nbsp; </p><p></p><p>c</p>");
        assert_eq!(out, "a\n\nb\n\nc");
        assert!(!out.lines().any(|l| l.ends_with(' ')), "trailing space in:\n{out:?}");
        assert!(!out.contains("\n\n\n"), "blank run survived in:\n{out:?}");
    }

    #[test]
    fn lists_keep_their_markers() {
        let out = readable_text("<ul><li>one</li><li>two</li></ul><ol><li>first</li><li>second</li></ol>");
        assert_eq!(out, "- one\n- two\n\n1. first\n2. second");
    }

    #[test]
    fn quotes_keep_their_marker() {
        let out = readable_text("<blockquote><p>quoted words</p></blockquote>");
        assert!(out.contains("> "), "quote marker missing from:\n{out:?}");
        assert!(out.contains("quoted words"), "quote text missing from:\n{out:?}");
    }

    #[test]
    fn markup_leaves_only_words() {
        let out = readable_text("<p>Hello <b>bold</b> <i>italic</i> <code>code</code> <a href=\"https://x.example\">link</a> world</p>");
        assert_eq!(out, "Hello bold italic code link world");
    }
}
