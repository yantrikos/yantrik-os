//! The Markdown work, tested as the pure functions it is: the outline, find and replace, the
//! format buttons, the counts and the HTML export. These are the assertions yDoc's old
//! `tests/document-core` held for `document.rs`, moved with the code.

use super::*;

// ── The outline ─────────────────────────────────────────────────────────────

#[test]
fn headings_come_out_in_order_with_their_level_and_their_offset() {
    let text = "intro\n\n# One\ntext\n## Two\n### Three\n";
    let found = outline(text);
    assert_eq!(
        found.iter().map(|h| (h.title.as_str(), h.level)).collect::<Vec<_>>(),
        vec![("One", 1), ("Two", 2), ("Three", 3)]
    );
    // The offset is a byte offset into the document, which is what moves the caret.
    for h in &found {
        assert!(text[h.offset..].starts_with('#'), "offset {} of {text:?}", h.offset);
    }
    assert_eq!(found[0].line, 3);
}

#[test]
fn a_hash_inside_a_fenced_code_block_is_not_a_heading() {
    let text = "# Real\n\n```sh\n# not a heading, a shell comment\n```\n\n## Also real\n";
    assert_eq!(
        outline(text).iter().map(|h| h.title.clone()).collect::<Vec<_>>(),
        vec!["Real".to_string(), "Also real".to_string()]
    );
}

#[test]
fn a_hash_with_no_space_after_it_is_not_a_heading() {
    assert!(outline("#hashtag\n").is_empty());
    assert!(outline("####### seven hashes\n").is_empty());
    assert_eq!(outline("###### six\n").len(), 1);
}

#[test]
fn closing_hashes_are_not_part_of_the_title() {
    assert_eq!(outline("## Middle ##\n")[0].title, "Middle");
}

// ── Find and replace ────────────────────────────────────────────────────────

#[test]
fn find_is_case_insensitive_and_counts_every_match() {
    let text = "Cat cat CAT concatenate";
    assert_eq!(matches(text, "cat").len(), 4);
    assert_eq!(matches(text, "").len(), 0);
    assert_eq!(matches(text, "dog").len(), 0);
}

#[test]
fn find_treats_its_query_as_text_and_not_as_a_pattern() {
    let text = "a.b and axb";
    assert_eq!(matches(text, "a.b"), vec![(0, 3)]);
}

#[test]
fn replace_all_reports_how_many_it_replaced() {
    let (out, count) = replace_all("one two one", "one", "1").expect("replace");
    assert_eq!(out, "1 two 1");
    assert_eq!(count, 2);

    let (unchanged, none) = replace_all("nothing here", "zebra", "x").unwrap();
    assert_eq!(unchanged, "nothing here");
    assert_eq!(none, 0);
}

#[test]
fn replace_keeps_offsets_true_across_multi_byte_characters() {
    let text = "café CAFÉ café";
    let (out, count) = replace_all(text, "café", "tea").expect("replace");
    assert_eq!(count, 3);
    assert_eq!(out, "tea tea tea");
}

#[test]
fn a_replacement_that_would_burst_the_limit_is_refused_before_it_is_built() {
    let text = "x".repeat(1000);
    let error = replace_all(&text, "x", &"y".repeat(2000)).expect_err("refused");
    assert!(error.contains("1 MiB"), "{error}");
}

// ── The format buttons ──────────────────────────────────────────────────────

fn formatted(text: &str, start: usize, end: usize, what: Format) -> (String, usize, usize) {
    let edit = format(text, start, end, what).expect("a format");
    (edit.text, edit.start, edit.end)
}

#[test]
fn bold_wraps_the_selection_and_keeps_the_selection_on_the_same_words() {
    let (text, start, end) = formatted("make this bold", 5, 9, Format::Bold);
    assert_eq!(text, "make **this** bold");
    assert_eq!(&text[start..end], "this");
}

#[test]
fn bold_on_something_already_bold_takes_the_markers_off_again() {
    let (text, start, end) = formatted("make **this** bold", 7, 11, Format::Bold);
    assert_eq!(text, "make this bold");
    assert_eq!(&text[start..end], "this");
}

#[test]
fn bold_over_a_selection_that_includes_the_markers_also_unwraps() {
    let (text, start, end) = formatted("make **this** bold", 5, 13, Format::Bold);
    assert_eq!(text, "make this bold");
    assert_eq!(&text[start..end], "this");
}

#[test]
fn a_wrap_with_nothing_selected_leaves_the_caret_between_the_markers() {
    let (text, start, end) = formatted("ab", 1, 1, Format::Italic);
    assert_eq!(text, "a**b");
    assert_eq!(start, 2);
    assert_eq!(end, 2);
}

#[test]
fn the_other_wraps_write_the_marks_the_format_actually_uses() {
    assert_eq!(formatted("gone", 0, 4, Format::Strikethrough).0, "~~gone~~");
    assert_eq!(formatted("code", 0, 4, Format::InlineCode).0, "`code`");
}

#[test]
fn a_heading_replaces_whatever_heading_the_line_already_had() {
    assert_eq!(formatted("plain\n", 0, 0, Format::Heading(2)).0, "## plain\n");
    assert_eq!(formatted("## two\n", 4, 4, Format::Heading(3)).0, "### two\n");
    // The same level again is the way back to a paragraph.
    assert_eq!(formatted("## two\n", 4, 4, Format::Heading(2)).0, "two\n");
}

#[test]
fn a_line_format_applies_to_every_line_the_selection_touches() {
    let text = "one\ntwo\nthree\n";
    let (out, _, _) = formatted(text, 1, 9, Format::Bullet);
    assert_eq!(out, "- one\n- two\n- three\n");
}

#[test]
fn a_line_format_over_lines_that_all_have_it_takes_it_off_all_of_them() {
    let (out, _, _) = formatted("> a\n> b\n", 0, 7, Format::Quote);
    assert_eq!(out, "a\nb\n");
}

#[test]
fn a_mixed_selection_gets_the_mark_rather_than_losing_it() {
    let (out, _, _) = formatted("- one\ntwo\n", 0, 9, Format::Bullet);
    assert_eq!(out, "- one\n- two\n");
}

#[test]
fn a_bullet_pressed_on_a_checklist_item_converts_it_instead_of_clearing_it() {
    assert_eq!(formatted("- [ ] task\n", 8, 8, Format::Bullet).0, "- task\n");
    assert_eq!(formatted("- [x] done\n", 8, 8, Format::Checklist).0, "done\n");
    assert_eq!(formatted("- item\n", 4, 4, Format::Checklist).0, "- [ ] item\n");
}

#[test]
fn a_code_block_fences_the_lines_and_leaves_the_caret_inside_them() {
    let (text, start, end) = formatted("let x = 1;\n", 0, 3, Format::CodeBlock);
    assert_eq!(text, "```\nlet x = 1;\n```\n");
    assert_eq!(&text[start..end], "let x = 1;");
}

#[test]
fn a_divider_goes_in_on_a_line_of_its_own() {
    let (text, start, end) = formatted("above\nbelow\n", 6, 6, Format::Divider);
    assert_eq!(text, "above\n---\nbelow\n");
    assert_eq!(start, 10);
    assert_eq!(end, 10);
}

#[test]
fn a_link_with_no_address_selects_the_placeholder_so_it_can_be_typed_over() {
    let edit = format_link("see here", 4, 8, "").expect("a link");
    assert_eq!(edit.text, "see [here](url)");
    assert_eq!(&edit.text[edit.start..edit.end], "url");
}

#[test]
fn a_link_with_an_address_is_finished_and_the_caret_is_after_it() {
    let edit = format_link("see here", 4, 8, "https://example.invalid").expect("a link");
    assert_eq!(edit.text, "see [here](https://example.invalid)");
    assert_eq!(edit.start, edit.text.len());
}

#[test]
fn a_caret_offset_inside_a_multi_byte_character_does_not_split_it() {
    // 'é' is two bytes; offset 2 is the middle of it. Snapping back is the only safe answer.
    let edit = format("café", 2, 2, Format::Bold).expect("no panic");
    assert!(edit.text.contains("**"));
    assert!(edit.text.chars().any(|c| c == 'é'), "{}", edit.text);
}

#[test]
fn an_offset_past_the_end_of_the_document_is_clamped_to_it() {
    let edit = format("ab", 9999, 9999, Format::Bold).expect("no panic");
    assert_eq!(edit.text, "ab****");
}

// ── Counts and export ───────────────────────────────────────────────────────

#[test]
fn characters_are_counted_as_characters_and_not_as_bytes() {
    let text = "café";
    assert_eq!(char_count(text), 4);
    assert_eq!(text.len(), 5, "the byte length the old status bar reported");
    assert_eq!(word_count("one two  three\nfour"), 4);
}

#[test]
fn the_unsaved_count_is_the_edit_and_not_the_document() {
    assert_eq!(unsaved_chars("same", "same"), 0);
    assert_eq!(unsaved_chars("a long document, edited", "a long document"), 8);
    assert_eq!(unsaved_chars("", "everything was deleted"), 0);
    // Characters, not bytes: an accented word replaced by another is counted as characters.
    assert_eq!(unsaved_chars("café", ""), 4);
}

#[test]
fn the_html_export_renders_the_markdown_rather_than_quoting_it() {
    let html = to_html("# Title\n\n- a\n- b\n\n~~gone~~\n", "Title");
    assert!(html.starts_with("<!doctype html>"), "{html}");
    assert!(html.contains("<title>Title</title>"), "{html}");
    assert!(html.contains("<h1>Title</h1>"), "{html}");
    assert!(html.contains("<li>a</li>"), "{html}");
    assert!(html.contains("<del>gone</del>"), "{html}");
}

#[test]
fn the_html_export_escapes_a_title_that_looks_like_markup() {
    let html = to_html("body\n", "a <script> & co");
    assert!(html.contains("<title>a &lt;script&gt; &amp; co</title>"), "{html}");
}
