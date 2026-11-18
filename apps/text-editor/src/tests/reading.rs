//! Reading a document to a mind one page at a time (VM 520, 4 October: a mind asked to read a
//! 9,557-byte spec found no action that read a document, and `describe` showed only its first
//! 4,000 characters without saying so).

use serde_json::{json, Value};
use yantrik_testing::TestApp;

use super::{act, none, scratch, started, state, summary};
use crate::reading;
use crate::TextEditor;

/// Follows `read`'s `how_to_see_the_rest` from line 1 to the end and gives back the text it read.
fn walk(app: &TestApp<TextEditor>, args: Value) -> String {
    let mut pages = Vec::new();
    let mut from = 1;
    for _ in 0..100 {
        let mut call = args.clone();
        call["from_line"] = json!(from);
        let page = act(app, "read", call).expect("a page");
        assert!(page.to_string().len() < 4_000, "a page a mind cannot keep whole: {page}");
        pages.push(page["text"].as_str().unwrap().to_string());
        let hint = page["how_to_see_the_rest"].as_str().unwrap();
        if hint.is_empty() {
            return pages.join("\n");
        }
        from = page["to_line"].as_u64().unwrap() as usize + 1;
        assert!(hint.ends_with(&format!("from_line {from}")), "the next call is named exactly: {hint}");
    }
    panic!("100 pages and the document never ended");
}

/// A document shaped like the spec from VM 520: about 9.5 KB over `lines` lines of uneven length,
/// with the quotes, backslashes and non-ASCII that JSON makes longer.
fn spec_like(lines: usize) -> String {
    (0..lines)
        .map(|i| match i % 4 {
            0 => format!("## {i}. Section — \"quoted\" \\path\\"),
            1 => format!("- item {i}: {}", "word ".repeat(i % 13 + 6)),
            2 => String::new(),
            _ => format!("{i}\ttabbed ✓ {}", "x".repeat(i % 60)),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn read_pages_a_tab_another_tab_and_a_file() {
    let app = started();
    let spec = spec_like(240);
    act(&app, "new", json!({ "text": spec })).unwrap();
    let s = state(&app);
    assert_eq!(s["content_cut"], true, "{s}");
    assert!(s["read_with"].as_str().unwrap().starts_with("read from_line "), "{s}");
    assert!(summary(&app).contains("`read` pages"));
    assert_eq!(walk(&app, none()), spec, "the tab in front, read whole");

    // Another tab, read where it is: nothing comes forward.
    act(&app, "select_tab", json!({ "index": 0 })).unwrap();
    let page = act(&app, "read", json!({ "tab": 1 })).unwrap();
    assert_eq!(page["modified"], true, "a new tab never saved is a buffer, not a file: {page}");
    assert!(page["how_to_see_the_rest"].as_str().unwrap().contains("tab 1 and from_line "), "{page}");
    assert_eq!(walk(&app, json!({ "tab": 1 })), spec);
    assert_eq!(app.model(|m| m.active), 0, "reading a tab does not bring it forward");
    assert!(act(&app, "read", json!({ "tab": 99 })).unwrap_err().contains("no tab 99"));

    // A file is `open`ed (standard) and then read; `read` itself never takes a path.
    let file = scratch("read").join("MDG-spec.md");
    std::fs::write(&file, &spec).unwrap();
    let by_path = json!({ "path": file.display().to_string() });
    let refused = act(&app, "read", by_path.clone()).unwrap_err();
    assert!(refused.contains("has no argument `path`"), "{refused}");
    let opened = act(&app, "open", by_path).unwrap();
    assert_eq!(opened["content_cut"], true, "{opened}");
    let read_with = opened["read_with"].as_str().unwrap().to_string();
    let from: usize = read_with.strip_prefix("read from_line ").expect(&read_with).parse().unwrap();
    let rest = act(&app, "read", json!({ "from_line": from })).unwrap();
    assert_eq!(rest["modified"], false, "a page of the file as saved says so: {rest}");
    assert!(spec.lines().nth(from - 1).is_some_and(|l| rest["text"].as_str().unwrap().starts_with(l)), "{rest}");
    assert_eq!(walk(&app, none()), spec, "the opened file, read whole");
    assert_eq!(opened["matches_disk"], true, "{opened}");
}

#[test]
fn read_walks_a_long_document_exactly_once_in_pages_a_mind_keeps_whole() {
    let spec = spec_like(240);
    assert!(spec.len() > 8_000, "long enough to need pages: {}", spec.len());
    let lines_total = spec.split('\n').count();
    let mut from = 1;
    let mut pages = Vec::new();
    loop {
        let page = reading::page(&spec, json!("/home/p/mdg/MDG-spec.md"), from, "");
        let size = page.to_string().len();
        assert!(size <= reading::READ_ANSWER, "a page a mind cannot keep whole: {size}");
        assert_eq!(page["lines_total"], lines_total);
        let to = page["to_line"].as_u64().unwrap() as usize;
        assert!(to >= from, "a page always carries at least one line: {page}");
        pages.push(page["text"].as_str().unwrap().to_string());
        let hint = page["how_to_see_the_rest"].as_str().unwrap();
        if to == lines_total {
            assert_eq!(hint, "");
            break;
        }
        assert_eq!(hint, format!("lines {from}–{to} of {lines_total}: call read with from_line {}", to + 1));
        from = to + 1;
    }
    assert!(pages.len() >= 3);
    assert_eq!(pages.join("\n"), spec, "every line exactly once, in order");
}

#[test]
fn read_cuts_a_line_longer_than_a_page_and_says_so() {
    let page = reading::page("one\ntwo\n", json!(null), 1, "");
    assert_eq!((page["text"].clone(), page["lines_total"].clone()), (json!("one\ntwo\n"), json!(3)));
    assert!(reading::describe_cut("one\ntwo\n").is_none());
    let text = format!("head\n{}\ntail", "\"".repeat(9_000));
    let first = reading::page(&text, json!(null), 1, "");
    assert_eq!(first["how_to_see_the_rest"], "lines 1–1 of 3: call read with from_line 2");
    let long = reading::page(&text, json!(null), 2, "");
    assert!(long.to_string().len() < 4_000);
    assert!(long["line_cut"].as_str().unwrap().contains("9000 characters long"), "{long}");
    let past = reading::page(&text, json!(null), 9, "");
    assert!(past["how_to_see_the_rest"].as_str().unwrap().contains("past the end"), "{past}");
    let spec = spec_like(240);
    let cut = reading::describe_cut(&spec).expect("9.5 KB is cut at 4,000 characters");
    let shown = reading::describe_content(&spec);
    assert_eq!(cut, format!("read from_line {}", shown.split('\n').count()));
}
