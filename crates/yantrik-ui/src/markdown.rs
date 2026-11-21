//! Simple block-level markdown parser for chat messages.
//!
//! Converts LLM markdown output into a flat list of styled blocks
//! that Slint can render with per-block formatting.
//!
//! Handles: code fences (with their language), headings (six levels), bullet and numbered lists
//! (nested, with wrapped continuation lines), GFM tables, block quotes, rules, the tool trail (`⚙️ …`, see
//! `trail`). Each block carries its text twice: plain, with the inline markers stripped (what a
//! view that draws one `Text` per block shows — the Lens's bubble), and as the markdown it was
//! written in, which [`styled`] turns into Slint's `StyledText` for a view that draws bold,
//! italic, inline code and links (the Agents pane).

use crate::markdown_lines::{self as line_of, ListItem};

/// A parsed content block with its type and text.
#[derive(Default)]
pub struct ParsedBlock {
    /// "text", "code", "heading", "bullet", "quote", "rule", "table" or "tool".
    pub block_type: &'static str,
    /// The block as plain text, inline markers stripped.
    pub text: String,
    /// The same block with its inline markdown kept — `**bold**`, `*italic*`, `` `code` ``, links.
    /// A code block's and a tool block's is its text: nothing in either is markup.
    pub markdown: String,
    /// A `tool` block's call. `None` for every other kind of block.
    pub call: Option<crate::trail::ToolCall>,
    /// A heading's level, 1 to 6. 0 for every other block.
    pub level: u8,
    /// A code block's language, as its fence named it. Empty when it named none.
    pub lang: String,
    /// A `bullet` block's items, one by one, so a view can hang each under its marker.
    pub items: Vec<ListItem>,
    /// A `table` block's rows of inline markdown, the header row first, every row as wide as the widest.
    pub rows: Vec<Vec<String>>,
}

impl ParsedBlock {
    fn of(block_type: &'static str, text: String, markdown: String) -> Self {
        ParsedBlock { block_type, text, markdown, ..Default::default() }
    }
}

/// The block being gathered, line by line: its plain text and its markdown side by side.
#[derive(Default)]
struct Gathering {
    text: String,
    markdown: String,
    items: Vec<ListItem>,
    lang: String,
}

impl Gathering {
    fn push(&mut self, separator: char, text: &str, markdown: &str) {
        if !self.text.is_empty() {
            self.text.push(separator);
        }
        self.text.push_str(text);
        if !self.markdown.is_empty() {
            self.markdown.push(separator);
        }
        self.markdown.push_str(markdown);
    }

    /// A list item's wrapped line, indented under its marker: more of the same item.
    fn continue_last(&mut self, text: &str, markdown: &str) {
        self.text.push(' ');
        self.text.push_str(text);
        self.markdown.push(' ');
        self.markdown.push_str(markdown);
        if let Some(item) = self.items.last_mut() {
            item.markdown.push(' ');
            item.markdown.push_str(markdown);
        }
    }
}

/// Parse markdown content into renderable blocks.
pub fn parse_blocks(content: &str) -> Vec<ParsedBlock> {
    let mut blocks = Vec::new();
    let mut current_type = "text";
    let mut current = Gathering::default();
    let mut in_code_block = false;

    let mut lines = content.lines().peekable();
    while let Some(line) = lines.next() {
        // Code fence toggle
        if line.trim_start().starts_with("```") {
            if in_code_block {
                // End code block
                flush(&mut blocks, &mut current, "code");
                in_code_block = false;
            } else {
                // Flush current text, start code block
                flush(&mut blocks, &mut current, current_type);
                current_type = "text";
                in_code_block = true;
                current.lang = line.trim_start()[3..].split_whitespace().next().unwrap_or("").to_string();
            }
            continue;
        }

        if in_code_block {
            current.push('\n', line, line);
            continue;
        }

        // A tool call, as the trail writes it: its own block, never folded into the paragraph
        // around it. The panel used to show `⚙️ mcp_yantrik_os_os_act...` as a line of prose and
        // the arguments, when a harness sent them, as raw JSON (#125).
        if crate::trail::is_trail(line) {
            if let Some((call, took_next)) = crate::trail::parse(line, lines.peek().copied()) {
                if took_next {
                    lines.next();
                }
                flush(&mut blocks, &mut current, current_type);
                current_type = "text";
                let summary = call.summary();
                blocks.push(ParsedBlock { call: Some(call), ..ParsedBlock::of("tool", summary.clone(), summary) });
                continue;
            }
        }

        let trimmed = line.trim();

        // Empty line → paragraph break
        if trimmed.is_empty() {
            flush(&mut blocks, &mut current, current_type);
            current_type = "text";
            continue;
        }

        // A rule: a line of its own, a thin line when drawn.
        if line_of::is_rule(trimmed) {
            flush(&mut blocks, &mut current, current_type);
            current_type = "text";
            blocks.push(ParsedBlock::of("rule", "\u{2500}\u{2500}\u{2500}".into(), String::new()));
            continue;
        }

        // Heading (# to ######)
        if let Some((level, heading_text)) = line_of::heading(trimmed) {
            flush(&mut blocks, &mut current, current_type);
            current_type = "text";
            blocks.push(ParsedBlock { level, ..ParsedBlock::of("heading", strip_inline(&heading_text), heading_text) });
            continue;
        }

        // A GFM table: a header row, a `|---|` row under it, then the body rows.
        if line_of::opens_table(trimmed, lines.peek().copied()) {
            flush(&mut blocks, &mut current, current_type);
            current_type = "text";
            lines.next();
            blocks.push(table(trimmed, &mut lines));
            continue;
        }

        // Block quote: the lines after `>` run together as one paragraph.
        if let Some(inner) = line_of::quote(line) {
            if current_type != "quote" {
                flush(&mut blocks, &mut current, current_type);
            }
            current_type = "quote";
            if !inner.is_empty() {
                current.push(' ', &strip_inline(inner), inner);
            }
            continue;
        }

        // List item (- a, * a, • a, 1. a): one entry in the block's items, nested by indent.
        if let Some(item) = line_of::list_item(line) {
            if current_type != "bullet" {
                flush(&mut blocks, &mut current, current_type);
            }
            current_type = "bullet";
            let marker = item.number.map_or("\u{2022}".to_string(), |n| format!("{n}."));
            current.push('\n', &format!("{marker} {}", strip_inline(&item.markdown)), &format!("{marker} {}", item.markdown));
            current.items.push(item);
            continue;
        }

        // A wrapped line of the item above, indented under its text.
        if current_type == "bullet" && line_of::indent_of(line) >= 2 {
            current.continue_last(&strip_inline(trimmed), trimmed);
            continue;
        }

        // Normal text paragraph
        if current_type != "text" {
            flush(&mut blocks, &mut current, current_type);
        }
        current_type = "text";
        current.push(' ', &strip_inline(trimmed), trimmed);
    }

    // Flush remaining content
    flush(&mut blocks, &mut current, if in_code_block { "code" } else { current_type });

    // If no blocks were parsed, return content as single text block
    if blocks.is_empty() && !content.trim().is_empty() {
        blocks.push(ParsedBlock::of("text", strip_inline(content.trim()), content.trim().to_string()));
    }

    blocks
}

/// A table whose header line and separator are read: its body rows follow while they have a `|`.
fn table<'a>(header: &str, lines: &mut std::iter::Peekable<impl Iterator<Item = &'a str>>) -> ParsedBlock {
    let mut rows = vec![line_of::cells(header)];
    while let Some(next) = lines.next_if(|l| !l.trim().is_empty() && l.contains('|')) {
        rows.push(line_of::cells(next));
    }
    // As wide as the widest row: a longer row widens the table (the header gets empty cells)
    // rather than losing a cell.
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    for row in &mut rows {
        row.resize(width, String::new());
    }
    let join = |cell: fn(&str) -> String| {
        rows.iter().map(|r| r.iter().map(|c| cell(c)).collect::<Vec<_>>().join(" | ")).collect::<Vec<_>>().join("\n")
    };
    ParsedBlock { rows: rows.clone(), ..ParsedBlock::of("table", join(strip_inline), join(str::to_string)) }
}

fn flush(blocks: &mut Vec<ParsedBlock>, current: &mut Gathering, block_type: &'static str) {
    let Gathering { text, markdown, items, lang } = std::mem::take(current);
    if text.is_empty() {
        return;
    }
    // A code block is its lines exactly; nothing else has trailing space worth keeping.
    let keep = |s: String| if block_type == "code" { s } else { s.trim_end().to_string() };
    blocks.push(ParsedBlock { items, lang, ..ParsedBlock::of(block_type, keep(text), keep(markdown)) });
}

/// A block's inline markdown as Slint's `StyledText`: bold, italic, inline code, strikethrough and
/// links drawn inside one wrapped text element, which a plain `Text` cannot do.
///
/// A paragraph's and a list's markdown is read; any other block is its plain text, since a
/// heading is drawn in a heading's own weight and nothing in a code block is markup. What
/// `StyledText` does not take — a raw HTML tag, a block quote, a rule — falls back to the block's
/// plain text with the markers stripped, the Lens's reading, so no block is lost to a parse error.
///
/// An unclosed marker, which is what a paragraph looks like mid-stream (`it is **very`), stays
/// the characters it is — CommonMark leaves an unmatched `**` as text — and the chunk that closes
/// it turns the run bold. Nothing is held back, and the block is as tall as its text.
pub fn styled(block: &ParsedBlock) -> slint::StyledText {
    match block.block_type {
        "text" | "bullet" | "quote" => slint::StyledText::from_markdown(&show_link_targets(&block.markdown))
            .unwrap_or_else(|_| slint::StyledText::from_plain_text(&block.text)),
        _ => slint::StyledText::from_plain_text(&block.text),
    }
}

#[cfg(test)]
#[path = "markdown_elements_tests.rs"]
mod elements;

#[cfg(test)]
mod tests {
    use super::*;

    /// A call in the trail is a block of its own kind, so the bubble can draw it as the call —
    /// name, what it touched, arguments — rather than as a sentence that happens to start with
    /// a gear. On the version this fixes, it came out as a "text" block reading
    /// `⚙️ os_act studio.generate {"args":{…}}`.
    #[test]
    fn a_tool_call_in_an_answer_is_its_own_block_with_its_arguments_on_it() {
        let blocks = parse_blocks(
            "Making it now.\n⚙️ os_act studio.generate {\"args\":{\"prompt\":\"a red kite\"}}\n\nDone: one picture.",
        );
        let kinds: Vec<&str> = blocks.iter().map(|b| b.block_type).collect();
        assert_eq!(kinds, vec!["text", "tool", "text"], "{kinds:?}");
        assert_eq!(blocks[1].text, "os_act studio.generate prompt=\"a red kite\"");
        assert_eq!(blocks[2].text, "Done: one picture.");
    }

    #[test]
    fn a_tool_calls_whole_arguments_ride_with_the_block_and_prose_has_none() {
        let blocks = parse_blocks("⚙️ os_act notes.new_note {\"args\":{\"body\":\"the whole body\"}}\n\nSaved.");
        let call = blocks[0].call.as_ref().expect("a tool block carries its call");
        assert!(call.detail().contains("\"body\": \"the whole body\""));
        assert!(blocks[1].call.is_none());
    }

    /// Hermes' verbose form puts the arguments on the line after the call. They belong to the
    /// call, not to the paragraph.
    #[test]
    fn arguments_on_the_next_line_are_not_shown_as_a_paragraph_of_json() {
        let blocks = parse_blocks(
            "⚙️ mcp_yantrik_os_os_act(['app', 'action', 'args'])\n{\"app\": \"studio\", \"action\": \"generate\", \"args\": {\"prompt\": \"a red kite\"}}\nDone.",
        );
        let kinds: Vec<&str> = blocks.iter().map(|b| b.block_type).collect();
        assert_eq!(kinds, vec!["tool", "text"], "{kinds:?}");
        assert_eq!(blocks[0].text, "mcp_yantrik_os_os_act studio.generate prompt=\"a red kite\"");
    }

    #[test]
    fn a_gear_inside_a_code_fence_stays_code() {
        let blocks = parse_blocks("```\n⚙️ os_apps\n```");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].block_type, "code");
    }

    /// The shape every catalog role's answer takes (#190): headings, bold labels, italics,
    /// backticks, a list, a fence. Each block keeps the markdown it was written in beside the
    /// plain text, and a code block's text is its markdown, markers and all.
    #[test]
    fn each_block_keeps_its_markdown_beside_its_plain_text() {
        let blocks = parse_blocks(
            "## Strongest **point**\n\nThe **How:** is *slow* and `cargo test`\nruns it.\n\n\
             - one **bold** item\n- two\n1. first\n\n```\nlet x = **y**;\n```",
        );
        let kinds: Vec<&str> = blocks.iter().map(|b| b.block_type).collect();
        assert_eq!(kinds, vec!["heading", "text", "bullet", "code"], "{kinds:?}");
        assert_eq!((blocks[0].text.as_str(), blocks[0].markdown.as_str()), ("Strongest point", "Strongest **point**"));
        assert_eq!(blocks[1].text, "The How: is slow and cargo test runs it.");
        assert_eq!(blocks[1].markdown, "The **How:** is *slow* and `cargo test` runs it.");
        assert_eq!(blocks[2].text, "\u{2022} one bold item\n\u{2022} two\n1. first");
        assert_eq!(blocks[2].markdown, "\u{2022} one **bold** item\n\u{2022} two\n1. first");
        assert_eq!((blocks[3].text.as_str(), blocks[3].markdown.as_str()), ("let x = **y**;", "let x = **y**;"));
    }

    /// What `StyledText` holds, as far as a test can see it: its Debug form names each span's style.
    fn styles(text: &slint::StyledText) -> String {
        format!("{text:?}")
    }

    #[test]
    fn a_paragraph_and_a_list_are_drawn_with_their_bold_italic_and_code() {
        let blocks = parse_blocks("The **How:** is *slow* and `cargo` runs.\n\n- one **bold**\n- two");
        let prose = styled(&blocks[0]);
        for style in ["Strong", "Emphasis", "Code"] {
            assert!(styles(&prose).contains(style), "{style} missing: {prose:?}");
        }
        assert_ne!(prose, slint::StyledText::from_plain_text(&blocks[0].text), "it is not the plain text");
        let list = styled(&blocks[1]);
        assert!(styles(&list).contains("Strong"), "{list:?}");
        assert!(styles(&list).contains("\u{2022} two"), "each item is its own line: {list:?}");
    }

    /// Mid-stream a paragraph can end inside a marker. The unclosed `**` is text — the layout is the
    /// text's, nothing is held back — and the chunk that closes it makes the run bold.
    #[test]
    fn an_unclosed_marker_mid_stream_is_text_until_it_closes() {
        let partial = parse_blocks("It is **very");
        assert_eq!(partial.len(), 1);
        let drawn = styled(&partial[0]);
        assert_eq!(drawn, slint::StyledText::from_plain_text("It is **very"), "{drawn:?}");
        let also = styled(&parse_blocks("Run `cargo")[0]);
        assert_eq!(also, slint::StyledText::from_plain_text("Run `cargo"), "{also:?}");
        let closed = styled(&parse_blocks("It is **very** slow")[0]);
        assert!(styles(&closed).contains("Strong"), "{closed:?}");
    }

    /// What `StyledText` refuses — raw HTML, a block quote — is still drawn: as the plain text the
    /// Lens shows. And a heading or a code block is never read as markup.
    #[test]
    fn what_styled_text_cannot_take_falls_back_to_the_plain_text() {
        for source in ["a <div>b</div> **c**"] {
            let block = &parse_blocks(source)[0];
            assert_eq!(styled(block), slint::StyledText::from_plain_text(&block.text), "{source}");
        }
        let code = &parse_blocks("```\n**not bold**\n```")[0];
        assert_eq!(styled(code), slint::StyledText::from_plain_text("**not bold**"));
        let heading = &parse_blocks("# A `title`")[0];
        assert_eq!(styled(heading), slint::StyledText::from_plain_text("A title"));
    }
}

/// Writes a link's real target after its text, muted (italic): `[Guide](https://docs.example.com/x)`
/// reads "Guide *(docs.example.com)*", so a link that says one thing and opens another shows where
/// it goes before it is pressed. A link whose text is its own address (compared without a trailing
/// `/`), an autolink and a bare URL are left as they are. Images and code spans are untouched.
pub(crate) fn show_link_targets(markdown: &str) -> String {
    let chars: Vec<(usize, char)> = markdown.char_indices().collect();
    let mut out = String::with_capacity(markdown.len());
    let (mut i, mut copied, mut in_code) = (0, 0, false);
    while i < chars.len() {
        let c = chars[i].1;
        if c == '`' {
            in_code = !in_code;
        } else if c == '[' && !in_code && (i == 0 || chars[i - 1].1 != '!') {
            // The `]` that closes this `[`, then `(target)` straight after it.
            let mut depth = 0;
            let close = chars[i..].iter().position(|&(_, ch)| {
                depth += (ch == '[') as i32 - (ch == ']') as i32;
                depth == 0
            });
            if let Some(close) = close.map(|n| i + n) {
                let after = chars.get(close + 1).map(|&(_, ch)| ch);
                let end = chars[close..].iter().position(|&(_, ch)| ch == ')').map(|n| close + n);
                if let (Some('('), Some(end)) = (after, end) {
                    let text = &markdown[chars[i + 1].0..chars[close].0];
                    let target = markdown[chars[close + 2].0..chars[end].0].trim();
                    if !same_address(text, target) {
                        out.push_str(&markdown[copied..chars.get(end + 1).map_or(markdown.len(), |c| c.0)]);
                        out.push_str(&format!(" *({})*", target_host(target)));
                        copied = chars.get(end + 1).map_or(markdown.len(), |c| c.0);
                    }
                    i = end + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    out.push_str(&markdown[copied..]);
    out
}

fn same_address(text: &str, target: &str) -> bool {
    text.trim().trim_end_matches('/') == target.trim_end_matches('/')
}

/// The host a link goes to: no scheme, user, port, path or query. A target with no `scheme://`
/// (`javascript:...`, a bare path) is shown as written, cut short.
fn target_host(target: &str) -> String {
    let Some((_, rest)) = target.split_once("://") else {
        return target.chars().take(40).collect();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    authority.split(':').next().unwrap_or(authority).to_string()
}

/// Strip inline markdown markers: **bold**, *italic*, `code`.
pub(crate) fn strip_inline(text: &str) -> String {
    let mut result = text.to_string();

    // Bold: **text**
    while let Some(start) = result.find("**") {
        if let Some(end) = result[start + 2..].find("**") {
            let inner = result[start + 2..start + 2 + end].to_string();
            result = format!("{}{}{}", &result[..start], inner, &result[start + 2 + end + 2..]);
        } else {
            break;
        }
    }

    // Italic: *text* (single asterisks remaining after bold removal)
    while let Some(start) = result.find('*') {
        if let Some(end) = result[start + 1..].find('*') {
            if end > 0 {
                let inner = result[start + 1..start + 1 + end].to_string();
                result =
                    format!("{}{}{}", &result[..start], inner, &result[start + 1 + end + 1..]);
            } else {
                break;
            }
        } else {
            break;
        }
    }

    // Inline code: `text`
    while let Some(start) = result.find('`') {
        if let Some(end) = result[start + 1..].find('`') {
            let inner = result[start + 1..start + 1 + end].to_string();
            result = format!("{}{}{}", &result[..start], inner, &result[start + 1 + end + 1..]);
        } else {
            break;
        }
    }

    result
}
