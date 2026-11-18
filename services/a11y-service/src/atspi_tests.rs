//! What a window is summarised as, and how small the answer stays.

use super::*;

fn el(role: &str, name: &str) -> Element {
    Element {
        role: role.into(),
        name: name.into(),
        text: String::new(),
        actions: Vec::new(),
        id: "x".into(),
    }
}

#[test]
fn a_window_is_summarised_the_way_someone_would_say_it() {
    let elements = vec![
        el("dialog", "Expenses"),
        el("label", "Merchant"),
        el("text", ""),
        el("text", ""),
        el("push button", "Cancel"),
        el("push button", "OK"),
    ];
    let s = summarise(&elements);
    assert!(s.starts_with("Expenses"), "the lead should be what the window calls itself: {s}");
    assert!(s.contains("2 push buttons"), "{s}");
    assert!(s.contains("2 texts"), "{s}");
}

#[test]
fn scaffolding_is_not_counted() {
    // Six nested panels is what a real zenity dialog looks like, and none of them is a thing
    // a person would mention.
    let mut elements = vec![el("dialog", "Expenses")];
    elements.extend((0..6).map(|_| el("panel", "")));
    let s = summarise(&elements);
    assert!(!s.contains("panel"), "layout must not appear in the summary: {s}");
}

#[test]
fn an_empty_window_says_so_rather_than_inventing_content() {
    assert_eq!(summarise(&[]), "an empty window");
}

#[test]
fn the_caps_are_small_enough_to_stay_a_glance() {
    // The point of this over a screenshot is that it is cheap to read. A cap large enough to
    // dump a document would give that away.
    assert!(MAX_ELEMENTS <= 200);
    assert!(MAX_TEXT <= 1000);
    assert!(MAX_DEPTH <= 20);
}
