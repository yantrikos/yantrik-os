//! The rendered Markdown preview, as blocks the window draws one after another.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// One block: its text, and how it is drawn (0 body, 1 heading, 2 code, 3 quote).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub text: String,
    pub kind: i32,
}

const BODY: i32 = 0;
const HEADING: i32 = 1;
const CODE: i32 = 2;
const QUOTE: i32 = 3;

pub fn blocks(markdown: &str) -> Vec<Block> {
    let mut blocks = vec![];
    let mut text = String::new();
    let mut kind = BODY;
    let flush = |blocks: &mut Vec<Block>, text: &mut String, kind| {
        if !text.trim().is_empty() {
            blocks.push(Block { text: std::mem::take(text), kind })
        }
    };
    for event in Parser::new_ext(markdown, Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Start(Tag::Heading { .. }) => {
                flush(&mut blocks, &mut text, kind);
                kind = HEADING;
            }
            Event::Start(Tag::CodeBlock(_)) => {
                flush(&mut blocks, &mut text, kind);
                kind = CODE;
            }
            Event::Start(Tag::BlockQuote(_)) => {
                flush(&mut blocks, &mut text, kind);
                kind = QUOTE;
            }
            Event::Start(Tag::Item) => {
                flush(&mut blocks, &mut text, kind);
                text.push_str("• ");
            }
            Event::End(
                TagEnd::Heading(_) | TagEnd::Paragraph | TagEnd::CodeBlock | TagEnd::Item | TagEnd::BlockQuote(_),
            ) => {
                flush(&mut blocks, &mut text, kind);
                kind = BODY;
            }
            Event::Text(t) | Event::Code(t) => text.push_str(&t),
            Event::SoftBreak | Event::HardBreak => text.push('\n'),
            Event::TaskListMarker(done) => {
                if text == "• " {
                    text.clear()
                }
                text.push_str(if done { "☑ " } else { "☐ " })
            }
            Event::Rule => {
                flush(&mut blocks, &mut text, kind);
                text.push_str("────────────");
                flush(&mut blocks, &mut text, BODY);
            }
            _ => {}
        }
    }
    flush(&mut blocks, &mut text, kind);
    blocks
}
