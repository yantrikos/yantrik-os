//! The Lens's blocks: [`crate::markdown::ParsedBlock`]s turned into the `ContentBlock`s its bubble
//! draws. The one place that does it, so a reply read live, read at the end of the stream and put
//! back from the store is drawn the same way.

use crate::markdown::{styled, ParsedBlock};
use crate::{ContentBlock, TableRow};
use slint::{ModelRc, SharedString, StyledText, VecModel};

/// A piece of inline markdown as `StyledText`; text `StyledText` refuses is drawn as it reads.
fn inline(markdown: &str) -> StyledText {
    StyledText::from_markdown(markdown)
        .unwrap_or_else(|_| StyledText::from_plain_text(&crate::markdown::strip_inline(markdown)))
}

/// A table cell; the header row's are bold. A cell that is only a dash, or opens like a list item
/// or a heading, is the characters it is: read as markdown a lone `-` is an empty bullet.
fn table_cell(markdown: &str, header: bool) -> StyledText {
    let looks_like_block = markdown.starts_with(['#', '>'])
        || ["-", "+", "*"]
            .iter()
            .any(|m| markdown == *m || markdown.starts_with(&format!("{m} ")));
    if looks_like_block {
        StyledText::from_plain_text(markdown)
    } else if header && !markdown.is_empty() {
        inline(&format!("**{markdown}**"))
    } else {
        inline(markdown)
    }
}

/// The blocks the bubble draws for parsed markdown: a list is one `item` block per entry (marker,
/// depth, its own text), a table one block holding its rows, a code block its language.
pub fn content_blocks(parsed: Vec<ParsedBlock>) -> Vec<ContentBlock> {
    let mut out = Vec::with_capacity(parsed.len());
    for block in parsed {
        if block.block_type == "bullet" {
            out.extend(block.items.iter().map(|item| {
                ContentBlock {
                    block_type: "item".into(),
                    text: crate::markdown::strip_inline(&item.markdown).into(),
                    styled: inline(&item.markdown),
                    marker: item
                        .number
                        .map_or_else(String::new, |n| format!("{n}."))
                        .into(),
                    depth: item.depth.min(4) as i32,
                    ..Default::default()
                }
            }));
            continue;
        }
        out.push(ContentBlock {
            block_type: SharedString::from(block.block_type),
            text: block.text.as_str().into(),
            styled: styled(&block),
            lang: block.lang.as_str().into(),
            level: block.level as i32,
            rows: ModelRc::new(VecModel::from(
                block
                    .rows
                    .iter()
                    .enumerate()
                    .map(|(n, row)| TableRow {
                        cells: ModelRc::new(VecModel::from(
                            row.iter()
                                .map(|c| table_cell(c, n == 0))
                                .collect::<Vec<_>>(),
                        )),
                    })
                    .collect::<Vec<_>>(),
            )),
            // The card the bubble draws for a tool call; empty for every other block.
            call: block
                .call
                .as_ref()
                .map(crate::trail::ToolCall::to_card)
                .unwrap_or_default(),
            ..Default::default()
        });
    }
    number_lists(&mut out);
    out
}

/// Numbers the numbered items in order. A list the reply broke with a blank line, with an indented
/// paragraph, or across two of the turn's text items is still one list while only items lie
/// between its entries, so it counts on (`1.` `2.` `3.`) as CommonMark counts it, not from `1.`
/// again at each piece. Anything else between two items starts a list afresh from the number it
/// was written with. Bullets get their glyph by depth.
pub fn number_lists(blocks: &mut [ContentBlock]) {
    let mut counts: Vec<Option<u64>> = Vec::new();
    for block in blocks.iter_mut() {
        if block.block_type != "item" {
            counts.clear();
            continue;
        }
        let depth = block.depth as usize;
        counts.truncate(depth + 1);
        counts.resize(depth + 1, None);
        // An empty marker is a bullet; a marker written as `N.` is a number to carry on from.
        let written = block.marker.trim_end_matches('.').parse::<u64>().ok();
        counts[depth] = match (written, counts[depth]) {
            (Some(_), Some(prev)) => Some(prev + 1),
            (written, _) => written,
        };
        block.marker = match counts[depth] {
            Some(n) => format!("{n}."),
            None => ["\u{2022}", "\u{25E6}", "\u{25AA}"][depth % 3].to_string(),
        }
        .into();
    }
}
