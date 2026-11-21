//! Each element the Lens draws as markdown: what the parser makes of it, and what the bubble is
//! handed. The list numbering tests are the ones for #1346's "10, 20, 30, 40".

use super::*;
use crate::markdown_blocks::{content_blocks, number_lists};
use slint::Model;

fn kinds(blocks: &[ParsedBlock]) -> Vec<&'static str> {
    blocks.iter().map(|b| b.block_type).collect()
}

/// The list entries of a reply as the bubble draws them: `depth marker text`.
fn entries(blocks: &[crate::ContentBlock]) -> Vec<String> {
    blocks
        .iter()
        .filter(|b| b.block_type == "item")
        .map(|b| format!("{} {} {}", b.depth, b.marker, b.text))
        .collect()
}

#[test]
fn a_heading_keeps_its_level_up_to_six() {
    let blocks = parse_blocks(
        "# One\n## Two\n### Three\n#### Four\n###### Six\n####### seven hashes is text",
    );
    let seen: Vec<(u8, &str)> = blocks.iter().map(|b| (b.level, b.text.as_str())).collect();
    assert_eq!(
        seen[..5],
        [
            (1, "One"),
            (2, "Two"),
            (3, "Three"),
            (4, "Four"),
            (6, "Six")
        ]
    );
    assert_eq!(blocks[5].block_type, "text");
}

#[test]
fn a_code_block_has_its_language_and_its_exact_lines() {
    let blocks = parse_blocks(
        "Run:\n\n```rust\nfn main() {\n    let x = **y**;\n\n}\n```\n\n```\nno language\n```",
    );
    assert_eq!(kinds(&blocks), vec!["text", "code", "code"]);
    assert_eq!(blocks[1].lang, "rust");
    assert_eq!(
        blocks[1].text, "fn main() {\n    let x = **y**;\n\n}",
        "indent, blank line and markers are kept"
    );
    assert_eq!(blocks[2].lang, "");
    let drawn = content_blocks(blocks);
    assert_eq!(
        (drawn[1].lang.as_str(), drawn[1].text.as_str()),
        ("rust", "fn main() {\n    let x = **y**;\n\n}")
    );
}

#[test]
fn a_code_block_mid_stream_is_code_until_its_fence_closes() {
    let blocks = parse_blocks("```py\nprint(1)");
    assert_eq!(
        (
            blocks[0].block_type,
            blocks[0].lang.as_str(),
            blocks[0].text.as_str()
        ),
        ("code", "py", "print(1)")
    );
}

#[test]
fn a_table_is_a_header_row_and_body_rows() {
    let blocks = parse_blocks(
        "Before\n\n| Name | Count |\n|:-----|------:|\n| `a` | **2** |\n| b | 3 |\n\nAfter",
    );
    assert_eq!(kinds(&blocks), vec!["text", "table", "text"]);
    assert_eq!(
        blocks[1].rows,
        vec![vec!["Name", "Count"], vec!["`a`", "**2**"], vec!["b", "3"]]
    );
    assert_eq!(
        blocks[1].text, "Name | Count\na | 2\nb | 3",
        "the plain text drops the inline markers"
    );
    let drawn = content_blocks(blocks);
    let cells = drawn[1].rows.row_data(1).unwrap().cells;
    assert!(
        format!("{:?}", cells.row_data(0).unwrap()).contains("Code"),
        "cell markdown is styled"
    );
    assert!(
        format!(
            "{:?}",
            drawn[1]
                .rows
                .row_data(0)
                .unwrap()
                .cells
                .row_data(0)
                .unwrap()
        )
        .contains("Strong"),
        "header is bold"
    );
}

#[test]
fn a_ragged_table_is_padded_to_its_widest_row_and_an_escaped_pipe_stays_in_its_cell() {
    let blocks = parse_blocks("a | b | c\n--|--|--\n1 | 2\nx \\| y | `p|q` | z | extra");
    assert_eq!(
        blocks[0].rows,
        vec![
            vec!["a", "b", "c", ""],
            vec!["1", "2", "", ""],
            vec!["x | y", "`p|q`", "z", "extra"]
        ]
    );
}

#[test]
fn pipes_without_a_separator_row_are_a_paragraph() {
    assert_eq!(kinds(&parse_blocks("a | b\nc | d")), vec!["text"]);
    assert_eq!(
        kinds(&parse_blocks("| a | b |\n|---|")),
        vec!["text"],
        "a separator of another width is not one"
    );
}

#[test]
fn a_table_mid_stream_is_a_paragraph_until_its_separator_arrives_then_a_table() {
    assert_eq!(kinds(&parse_blocks("| a | b |\n|--")), vec!["text"]);
    let done = parse_blocks("| a | b |\n|---|---|\n| 1 |");
    assert_eq!(
        (kinds(&done), done[0].rows[1].clone()),
        (vec!["table"], vec!["1".to_string(), String::new()])
    );
}

#[test]
fn a_block_quote_runs_its_lines_together_and_a_rule_is_its_own_block() {
    let blocks = parse_blocks("> one **two**\n> three\n\n---\n\n***\ntext");
    assert_eq!(kinds(&blocks), vec!["quote", "rule", "rule", "text"]);
    assert_eq!(
        (blocks[0].text.as_str(), blocks[0].markdown.as_str()),
        ("one two three", "one **two** three")
    );
    assert!(
        format!("{:?}", styled(&blocks[0])).contains("Strong"),
        "a quote's inline markdown is drawn"
    );
}

#[test]
fn a_nested_list_keeps_its_depth_and_a_wrapped_line_joins_its_item() {
    let blocks =
        parse_blocks("1. first\n   wrapped on\n   - inner **bold**\n     more inner\n2. second");
    assert_eq!(kinds(&blocks), vec!["bullet"]);
    let drawn = content_blocks(blocks);
    assert_eq!(
        entries(&drawn),
        vec![
            "0 1. first wrapped on",
            "1 \u{25E6} inner bold more inner",
            "0 2. second"
        ]
    );
}

/// A numbered list the reply spaced out with blank lines is one list, counted in order, whatever
/// number each line was written with.
#[test]
fn numbers_run_in_order_across_blank_lines() {
    let drawn = content_blocks(parse_blocks("1. one\n\n2. two\n\n3. three"));
    assert_eq!(entries(&drawn), vec!["0 1. one", "0 2. two", "0 3. three"]);
    let lazy = content_blocks(parse_blocks("1. one\n\n1. two\n\n1. three"));
    assert_eq!(
        entries(&lazy),
        vec!["0 1. one", "0 2. two", "0 3. three"],
        "CommonMark counts a list on from its first number"
    );
}

/// An item followed by an indented paragraph, then the next number: the paragraph is a block of its
/// own, the list begins again from the number it was written with.
#[test]
fn an_item_then_a_paragraph_then_the_next_number_keeps_the_written_number() {
    let drawn = content_blocks(parse_blocks("1. one\n\nA paragraph.\n\n2. two\n3. three"));
    assert_eq!(entries(&drawn), vec!["0 1. one", "0 2. two", "0 3. three"]);
}

/// Hermes puts a turn in many text items, and the Lens parses each on its own: a list split across
/// them is still counted on, and a list that starts from `1.` in each piece reads 1, 2, 3.
#[test]
fn a_list_split_across_text_items_counts_on() {
    let mut blocks = content_blocks(parse_blocks("Steps:\n\n1. open it"));
    blocks.extend(content_blocks(parse_blocks("1. then close it")));
    blocks.extend(content_blocks(parse_blocks("1. then leave")));
    number_lists(&mut blocks);
    assert_eq!(
        entries(&blocks),
        vec!["0 1. open it", "0 2. then close it", "0 3. then leave"]
    );
    let mut split = content_blocks(parse_blocks("3. third"));
    split.extend(content_blocks(parse_blocks("4. fourth")));
    number_lists(&mut split);
    assert_eq!(entries(&split), vec!["0 3. third", "0 4. fourth"]);
}

/// Streaming: the reply is parsed again, whole, on every chunk. At every cut the numbers so far are
/// the numbers of the finished reply, in order, and an unclosed `**` is literal until it closes.
#[test]
fn every_prefix_of_a_streaming_list_is_numbered_in_order() {
    let full = "Steps:\n\n1. **Open** it\n\n2. Press `go`\n   and wait\n\n3. Done";
    for cut in (0..=full.len()).filter(|c| full.is_char_boundary(*c)) {
        let drawn = content_blocks(parse_blocks(&full[..cut]));
        let markers: Vec<String> = drawn
            .iter()
            .filter(|b| b.block_type == "item")
            .map(|b| b.marker.to_string())
            .collect();
        let want: Vec<String> = (1..=markers.len()).map(|n| format!("{n}.")).collect();
        assert_eq!(markers, want, "cut at {cut}: {:?}", &full[..cut]);
    }
    let mid = content_blocks(parse_blocks("1. it is **very"));
    assert_eq!(
        mid[0].styled,
        slint::StyledText::from_plain_text("it is **very")
    );
}

/// #1346: a step list drawn "10, 20, 30, 40". Stored text on the test VM reads `1.`, `2.`. These
/// are the readings the code gives to the inputs that could make it: none puts a `0` after a digit.
#[test]
fn no_reading_of_a_step_list_makes_tens() {
    let inputs = [
        "1. a\n2. b\n3. c\n4. d",
        "1. a\n\n2. b\n\n3. c\n\n4. d",
        "1. a\n   para under a\n2. b\n   para under b",
        "1. a\n\n   indented paragraph under a\n\n2. b\n\n   indented under b",
        "10. a\n20. b",
        "1.  a\n2.  b",
        "1. a\n2. b\n\n```\ncode\n```\n\n3. c\n4. d",
    ];
    for input in inputs {
        let drawn = content_blocks(parse_blocks(input));
        for line in entries(&drawn) {
            let marker = line.split(' ').nth(1).unwrap();
            assert!(
                !matches!(marker, "10." | "20." | "30." | "40.") || input.contains(marker),
                "{input:?} -> {line}"
            );
        }
    }
}

#[test]
fn a_link_whose_text_is_not_its_address_shows_where_it_goes() {
    use crate::markdown::show_link_targets as shown;
    assert_eq!(shown("[Guide](https://docs.example.com/x?a=1)"), "[Guide](https://docs.example.com/x?a=1) *(docs.example.com)*");
    // The deceptive link: the text says one place, the target is another.
    assert_eq!(
        shown("see [https://bank.example](http://other.example) now"),
        "see [https://bank.example](http://other.example) *(other.example)* now"
    );
    assert_eq!(shown("[x](https://u:p@h.example:8080/a)"), "[x](https://u:p@h.example:8080/a) *(h.example)*");
    // Its own address, a bare URL, an autolink, an image and a code span stay as they are.
    for same in [
        "[https://a.example/](https://a.example)",
        "https://a.example/x",
        "<https://a.example>",
        "![pic](https://a.example/p.png)",
        "`[x](https://a.example)`",
        "no link [here] (at all)",
    ] {
        assert_eq!(shown(same), same);
    }
    // Two links, and a scheme the gate refuses is shown as written.
    assert_eq!(shown("[a](https://one.example) [b](javascript:x)"), "[a](https://one.example) *(one.example)* [b](javascript:x) *(javascript:x)*");
    // It reaches a list entry and a table cell through the one builder.
    let drawn = content_blocks(parse_blocks("- [Guide](https://docs.example.com/x)"));
    assert_eq!(drawn[0].text, "Guide", "the copyable text is untouched");
}
