//! The deck, tested without a desktop.
//!
//! The fault these exist for is the one a screenshot could never show. `on_next_slide` and
//! `on_prev_slide` moved `current-slide-index` and did nothing else: the canvas went on holding
//! the slide you had just been typing into, and the next thing that committed it (a thumbnail
//! click, Add, Duplicate) wrote that text into the slide you had moved to. Both of the two
//! most-used buttons in the app destroyed a slide, silently, and the window looked right the
//! whole time. `next_then_select_does_not_write_one_slide_over_the_next` below is that sequence
//! written out. It fails on the old code and passes on the new one.
//!
//! These were `tests/presentation-core`, which pulled this module in with `#[path]`. The ones
//! that were about the file and the draft moved to `tests/files.rs` and `tests/recovery.rs`,
//! where they run through the framework's document layer on real files.

use super::content::FORMAT_VERSION;
use super::{encode, parse, theme_spec, Content, Deck, Edits, LoadError, Slide, LAYOUT_COUNT, MAX_SLIDES, THEMES};

/// What the canvas would be showing for the slide that is selected.
///
/// This is exactly what the window hands to every command: the render writes the selected slide
/// onto the four `current-*` properties, and the window reads them back.
fn on_screen(deck: &Deck) -> Edits {
    Edits::from(deck.current_slide())
}

fn slide(title: &str, body: &str, notes: &str, layout: i32) -> Slide {
    let mut s = Slide::new(title, body, layout);
    s.notes = notes.to_string();
    s
}

/// The deck as if its file held what it shows now.
fn saved(deck: &mut Deck) {
    deck.saved_as(deck.content().clone());
}

/// A deck of three slides, selected on the first.
fn three() -> Deck {
    let mut deck = Deck::blank("Talk");
    deck.set_slide(0, Some("A"), Some("body a"), Some("note a")).unwrap();
    let canvas = on_screen(&deck);
    deck.add_slide(&canvas, None, slide("B", "body b", "note b", 1)).unwrap();
    let canvas = on_screen(&deck);
    deck.add_slide(&canvas, None, slide("C", "body c", "note c", 1)).unwrap();
    let canvas = on_screen(&deck);
    deck.go_to(&canvas, 0);
    deck
}

// ── The bug ──────────────────────────────────────────────────────────

#[test]
fn next_then_select_does_not_write_one_slide_over_the_next() {
    let mut deck = Deck::blank("Talk");
    let canvas = on_screen(&deck);
    deck.add_slide(&canvas, None, slide("Slide 2", "second body", "", 1)).unwrap();
    let canvas = on_screen(&deck);
    deck.go_to(&canvas, 0);

    // The person types into slide 1. The two-way binding means the canvas holds this and the
    // model does not, yet.
    let typed = Edits { title: "Opening".into(), body: "What this talk is about".into(), notes: "smile".into(), layout: 0 };

    // Next. This is the exact moment the old code got wrong.
    assert_eq!(deck.go_to(&typed, 1), 1);

    // The window now draws slide 2 onto the canvas, the half the old Next skipped.
    let canvas = on_screen(&deck);
    assert_eq!(canvas.title, "Slide 2", "the canvas must follow the selection");

    // The thumbnail click. Under the old code this committed slide 1's text into slide 2.
    deck.go_to(&canvas, 0);

    assert_eq!(deck.slides()[0].title, "Opening");
    assert_eq!(deck.slides()[0].body, "What this talk is about");
    assert_eq!(deck.slides()[0].notes, "smile");
    assert_eq!(deck.slides()[1].title, "Slide 2", "slide 2 must be untouched");
    assert_eq!(deck.slides()[1].body, "second body");
}

#[test]
fn go_to_commits_before_it_moves() {
    // The other half of the same rule: the edit is in the model the instant the selection
    // leaves, not once something else happens to commit.
    let mut deck = three();
    let typed = Edits { title: "Edited".into(), body: "typed".into(), notes: String::new(), layout: 0 };
    deck.go_to(&typed, 2);
    assert_eq!(deck.slides()[0].title, "Edited");
    assert_eq!(deck.current(), 2);
}

#[test]
fn a_commit_only_touches_the_slide_that_is_selected() {
    let mut deck = three();
    let canvas = on_screen(&deck);
    deck.go_to(&canvas, 1);
    deck.commit(&Edits { title: "B edited".into(), body: "body b".into(), notes: "note b".into(), layout: 1 });
    assert_eq!(deck.outline(), vec!["A", "B edited", "C"]);
}

#[test]
fn stepping_past_either_end_stops_there_rather_than_wrapping() {
    // Where a keystroke at the end of a deck arrives. Wrapping would hand a presenter the title
    // slide again in front of an audience.
    let mut deck = three();
    let canvas = on_screen(&deck);
    assert_eq!(deck.step(&canvas, -1), 0);
    let canvas = on_screen(&deck);
    deck.go_to(&canvas, 2);
    let canvas = on_screen(&deck);
    assert_eq!(deck.step(&canvas, 1), 2);
}

// ── Structure ────────────────────────────────────────────────────────

#[test]
fn deleting_the_only_slide_is_refused_and_changes_nothing() {
    let mut deck = Deck::blank("One");
    let err = deck.delete_slide(0).unwrap_err();
    assert!(err.contains("at least one"), "{err}");
    assert_eq!(deck.len(), 1);
    assert!(!deck.dirty(), "a refusal must not mark the deck modified");
}

#[test]
fn deleting_a_slide_that_is_not_there_names_the_deck_it_looked_in() {
    let mut deck = three();
    let err = deck.delete_slide(9).unwrap_err();
    assert!(err.contains("slide 10"), "{err}");
    assert!(err.contains("deck of 3"), "{err}");
    assert_eq!(deck.len(), 3);
}

#[test]
fn deleting_keeps_the_rest_in_order_and_selects_what_took_its_place() {
    let mut deck = three();
    let canvas = on_screen(&deck);
    deck.go_to(&canvas, 1);
    let gone = deck.delete_slide(1).unwrap();
    assert_eq!(gone.title, "B");
    assert_eq!(deck.outline(), vec!["A", "C"]);
    assert_eq!(deck.current(), 1, "the slide that moved up is the one selected");
}

#[test]
fn deleting_the_last_slide_pulls_the_selection_back() {
    let mut deck = three();
    let canvas = on_screen(&deck);
    deck.go_to(&canvas, 2);
    deck.delete_slide(2).unwrap();
    assert_eq!(deck.current(), 1);
    assert_eq!(deck.current_slide().title, "B");
}

#[test]
fn moving_a_slide_reorders_the_deck_and_the_selection_follows_it() {
    let mut deck = three();
    deck.move_slide(0, 2).unwrap();
    assert_eq!(deck.outline(), vec!["B", "C", "A"]);
    assert_eq!(deck.current(), 2);
    assert_eq!(deck.current_slide().title, "A");

    deck.move_slide(2, 0).unwrap();
    assert_eq!(deck.outline(), vec!["A", "B", "C"]);
    assert_eq!(deck.current(), 0);
}

#[test]
fn moving_past_the_end_lands_on_the_end() {
    let mut deck = three();
    assert_eq!(deck.move_slide(0, 99).unwrap(), 2);
    assert_eq!(deck.outline(), vec!["B", "C", "A"]);
}

#[test]
fn duplicating_puts_the_copy_next_to_the_original_and_selects_it() {
    let mut deck = three();
    let canvas = on_screen(&deck);
    deck.go_to(&canvas, 1);
    let canvas = on_screen(&deck);
    let at = deck.duplicate(&canvas).unwrap();
    assert_eq!(at, 2);
    assert_eq!(deck.outline(), vec!["A", "B", "B", "C"]);
    assert_eq!(deck.slides()[2], deck.slides()[1]);
    assert_eq!(deck.current(), 2);
}

#[test]
fn duplicating_copies_the_text_that_is_on_the_canvas_not_the_stale_row() {
    // The same class of fault as the navigation bug, one button along: Duplicate has to copy
    // what the person can see, not what the model last heard about.
    let mut deck = three();
    let typed = Edits { title: "A, as edited".into(), body: "body a".into(), notes: "note a".into(), layout: 0 };
    deck.duplicate(&typed).unwrap();
    assert_eq!(deck.outline(), vec!["A, as edited", "A, as edited", "B", "C"]);
}

#[test]
fn adding_after_a_named_slide_puts_it_there() {
    let mut deck = three();
    let canvas = on_screen(&deck);
    let at = deck.add_slide(&canvas, Some(2), slide("D", "", "", 1)).unwrap();
    assert_eq!(at, 3);
    assert_eq!(deck.outline(), vec!["A", "B", "C", "D"]);
}

#[test]
fn a_deck_will_not_grow_past_its_bound() {
    let mut deck = Deck::blank("Big");
    while deck.len() < MAX_SLIDES {
        let canvas = on_screen(&deck);
        deck.add_slide(&canvas, None, slide("x", "", "", 1)).unwrap();
    }
    let canvas = on_screen(&deck);
    let err = deck.add_slide(&canvas, None, slide("one too many", "", "", 1)).unwrap_err();
    assert!(err.contains(&MAX_SLIDES.to_string()), "{err}");
    assert_eq!(deck.len(), MAX_SLIDES);
}

// ── The format ───────────────────────────────────────────────────────

#[test]
fn a_deck_comes_back_from_its_text_with_every_field() {
    let mut deck = Deck::blank("Quarterly");
    deck.set_theme(3);
    deck.set_slide(0, Some("First"), Some("body one"), Some("note one")).unwrap();
    deck.set_layout(0, 3);
    let canvas = on_screen(&deck);
    deck.add_slide(&canvas, None, slide("Second", "body two", "note two", 2)).unwrap();

    let back = parse(&encode(deck.content())).unwrap();
    assert_eq!(back.title, "Quarterly");
    assert_eq!(back.theme, 3);
    assert_eq!(back.slides, deck.slides());
    assert_eq!(back.slides[0].layout, 3);
    assert_eq!(back.slides[1].notes, "note two");
}

#[test]
fn the_file_says_which_version_it_is_and_says_it_first() {
    let deck = Deck::blank("Named");
    let text = encode(deck.content());
    assert!(
        text.trim_start().starts_with("{\n  \"version\": 1"),
        "a reader that understands nothing else must still find the version: {text}"
    );
    assert_eq!(parse(&text).unwrap(), *deck.content());
}

#[test]
fn a_file_from_a_newer_build_is_refused_by_name_and_nothing_is_half_loaded() {
    let text = encode(three().content()).replace("\"version\": 1", "\"version\": 99");
    let err = parse(&text).unwrap_err();
    assert_eq!(err, LoadError::VersionAhead(99));
    let said = err.to_string();
    assert!(said.contains("99"), "{said}");
    assert!(said.contains(&FORMAT_VERSION.to_string()), "{said}");
}

#[test]
fn the_malformed_and_the_missing_are_told_apart() {
    assert!(matches!(parse("this is not json"), Err(LoadError::NotJson(_))));
    assert_eq!(
        parse("{\"title\":\"x\",\"slides\":[]}"),
        Err(LoadError::VersionMissing),
        "no version means there is no way to know what the file is"
    );
    assert!(matches!(parse("{\"version\":1,\"title\":\"x\",\"slides\":\"nope\"}"), Err(LoadError::Malformed(_))));
    assert_eq!(parse("{\"version\":1,\"title\":\"x\",\"slides\":[]}"), Err(LoadError::NoSlides));
}

#[test]
fn a_layout_this_build_cannot_draw_is_clamped_rather_than_drawn_blank() {
    let text = "{\"version\":1,\"title\":\"t\",\"theme\":0,\
                \"slides\":[{\"title\":\"a\",\"body\":\"\",\"notes\":\"\",\"layout\":99}]}";
    assert_eq!(parse(text).unwrap().slides[0].layout, LAYOUT_COUNT - 1);
}

#[test]
fn a_theme_number_from_a_build_with_more_themes_still_draws_something() {
    let text = "{\"version\":1,\"title\":\"t\",\"theme\":41,\
                \"slides\":[{\"title\":\"a\",\"body\":\"\",\"notes\":\"\",\"layout\":0}]}";
    assert!(parse(text).unwrap().theme < THEMES.len());
    assert!(!theme_spec(41).name.is_empty());
}

#[test]
fn an_empty_content_is_not_a_deck() {
    let text = encode(&Content::default());
    assert_eq!(parse(&text), Err(LoadError::NoSlides));
}

#[test]
fn a_deck_with_more_slides_than_the_bound_is_refused_by_name() {
    let one = Slide::new("s", "", 0);
    let content = Content { title: "big".into(), theme: 0, slides: vec![one; MAX_SLIDES + 1] };
    assert_eq!(parse(&encode(&content)), Err(LoadError::TooManySlides(MAX_SLIDES + 1)));
}

// ── Dirty ────────────────────────────────────────────────────────────

#[test]
fn a_new_deck_is_not_modified_until_something_is_typed_into_it() {
    let mut deck = Deck::blank("Untitled");
    assert!(!deck.dirty());
    deck.set_slide(0, Some("Typed"), None, None).unwrap();
    assert!(deck.dirty());
}

#[test]
fn moving_the_selection_is_not_an_edit() {
    let mut deck = three();
    saved(&mut deck);
    assert!(!deck.dirty());
    let canvas = on_screen(&deck);
    deck.go_to(&canvas, 2);
    assert!(!deck.dirty(), "looking at slide three is not a change to the deck");
}

#[test]
fn a_change_that_is_typed_and_then_undone_stops_counting_as_one() {
    // `dirty()` is a comparison against what is on disk, not a flag. A flag would keep saying
    // "unsaved" for a deck that is byte-for-byte the file.
    let mut deck = three();
    saved(&mut deck);
    deck.set_slide(0, Some("changed"), None, None).unwrap();
    assert!(deck.dirty());
    deck.set_slide(0, Some("A"), None, None).unwrap();
    assert!(!deck.dirty());
}

#[test]
fn edits_made_while_a_save_was_in_flight_stay_unsaved() {
    let mut deck = three();
    let written = deck.content().clone();
    deck.set_slide(0, Some("typed meanwhile"), None, None).unwrap();
    deck.saved_as(written);
    assert!(deck.dirty(), "the file holds what was written, not what was typed since");
}

// ── Undo ─────────────────────────────────────────────────────────────

#[test]
fn undo_steps_back_over_a_change_and_redo_puts_it_back() {
    let mut deck = three();
    deck.set_slide(1, Some("B edited"), None, None).unwrap();
    assert!(deck.can_undo());
    assert!(deck.undo(false));
    assert_eq!(deck.outline(), vec!["A", "B", "C"]);
    assert!(deck.can_redo());
    assert!(deck.undo(true));
    assert_eq!(deck.outline(), vec!["A", "B edited", "C"]);
}

#[test]
fn a_commit_that_changed_nothing_is_not_a_step_to_undo() {
    // Commit runs on every keystroke and again before every navigation. Without the guard in
    // `mutate`, half the undo stack would be steps that restore what they came from, and
    // pressing Undo three times would look like it had done nothing.
    let mut deck = Deck::opened(three().content().clone());

    let canvas = on_screen(&deck);
    deck.commit(&canvas);
    deck.go_to(&canvas, 1);
    let canvas = on_screen(&deck);
    deck.go_to(&canvas, 2);

    assert!(!deck.can_undo());
    assert!(!deck.dirty(), "and the deck is still what is on disk");
    assert_eq!(deck.revision(), 0, "and nothing changed for the document to hear about");
}

#[test]
fn undo_on_an_untouched_deck_says_no_rather_than_doing_something() {
    let mut deck = Deck::blank("Fresh");
    assert!(!deck.undo(false));
    assert!(!deck.undo(true));
    assert_eq!(deck.outline(), vec!["Title Slide"]);
}

#[test]
fn undo_back_past_a_delete_brings_the_slide_back() {
    let mut deck = three();
    deck.delete_slide(1).unwrap();
    assert_eq!(deck.outline(), vec!["A", "C"]);
    assert!(deck.undo(false));
    assert_eq!(deck.outline(), vec!["A", "B", "C"]);
}

#[test]
fn every_change_of_content_is_a_new_revision_and_looking_is_not() {
    let mut deck = three();
    let start = deck.revision();
    let canvas = on_screen(&deck);
    deck.go_to(&canvas, 2);
    assert_eq!(deck.revision(), start);
    deck.set_slide(0, Some("x"), None, None).unwrap();
    let after_edit = deck.revision();
    assert!(after_edit > start);
    assert!(deck.undo(false));
    assert!(deck.revision() > after_edit, "an undo is a change the document must hear about");
}

// ── Exports ──────────────────────────────────────────────────────────

#[test]
fn markdown_is_one_heading_per_slide_with_the_notes_quoted() {
    let mut deck = Deck::blank("Quarterly");
    deck.set_slide(0, Some("First"), Some("body one"), Some("note one\nsecond line")).unwrap();
    let canvas = on_screen(&deck);
    deck.add_slide(&canvas, None, slide("Second", "body two", "", 1)).unwrap();

    let md = deck.to_markdown();
    assert!(md.starts_with("<!-- yPresent deck: Quarterly -->"), "{md}");
    assert_eq!(md.matches("\n---\n").count(), 1, "one separator between two slides: {md}");
    assert!(md.contains("# First\n"), "{md}");
    assert!(md.contains("# Second\n"), "{md}");
    assert!(md.contains("body one"), "{md}");
    assert!(md.contains("> note one\n> second line\n"), "{md}");
    assert_eq!(md.matches("\n# ").count(), 2, "a heading per slide: {md}");
}

#[test]
fn a_slide_with_no_body_and_no_notes_is_a_heading_and_nothing_else() {
    let mut deck = Deck::blank("Bare");
    deck.set_slide(0, Some("Only a title"), Some(""), Some("")).unwrap();
    let md = deck.to_markdown();
    assert!(md.trim_end().ends_with("# Only a title"), "{md}");
    assert!(!md.contains("\n> "), "no empty blockquote: {md}");
}

#[test]
fn the_outline_is_the_titles_numbered() {
    assert_eq!(three().to_outline(), "# Talk\n\n1. A\n2. B\n3. C\n");
}

// ── Names ────────────────────────────────────────────────────────────

#[test]
fn a_title_becomes_a_file_name_a_shell_does_not_need_quoting_for() {
    assert_eq!(super::slug("Q3 Review — Sales!"), "q3-review-sales");
    assert_eq!(super::slug("  "), "untitled");
    assert_eq!(super::slug("already-fine"), "already-fine");
}

// ── Search ───────────────────────────────────────────────────────────

#[test]
fn search_looks_at_the_title_the_body_and_the_notes() {
    let deck = three();
    assert_eq!(deck.search("body b"), vec![1]);
    assert_eq!(deck.search("note c"), vec![2]);
    assert_eq!(deck.search("BODY"), vec![0, 1, 2], "case does not matter");
    assert_eq!(deck.search("nothing here"), Vec::<usize>::new());
    assert_eq!(deck.search("   "), Vec::<usize>::new(), "an empty query matches nothing");
}

// ── Themes ───────────────────────────────────────────────────────────

#[test]
fn choosing_a_theme_wraps_so_the_dropdown_can_simply_count_up() {
    let mut deck = Deck::blank("t");
    let last = THEMES.len() - 1;
    assert_eq!(deck.set_theme(last), last);
    assert_eq!(deck.set_theme(last + 1), 0);
    assert_eq!(deck.theme(), 0);
}

#[test]
fn the_theme_is_part_of_the_deck_and_is_saved_with_it() {
    let mut deck = three();
    deck.set_theme(2);
    assert_eq!(parse(&encode(deck.content())).unwrap().theme, 2);
}

// ── The shape of Content ─────────────────────────────────────────────

#[test]
fn the_selection_is_not_part_of_what_is_saved() {
    // Two decks looked at from different slides are the same deck, or every glance would report
    // unsaved changes.
    let mut a = three();
    let mut b = three();
    let canvas = on_screen(&b);
    b.go_to(&canvas, 2);
    assert_eq!(a.content(), b.content());
    a.set_slide(0, Some("different"), None, None).unwrap();
    assert_ne!(a.content(), b.content());
}

#[test]
fn a_deck_that_follows_its_file_keeps_the_selection_and_drops_the_history() {
    let mut deck = three();
    let canvas = on_screen(&deck);
    deck.go_to(&canvas, 2);
    deck.set_slide(0, Some("mine"), None, None).unwrap();
    let revision = deck.revision();
    let theirs = Content { title: "Theirs".into(), theme: 1, slides: vec![Slide::new("only", "", 0)] };
    deck.followed(theirs.clone());
    assert_eq!(deck.content(), &theirs);
    assert!(!deck.dirty() && !deck.can_undo());
    assert_eq!(deck.current(), 0, "the selection is kept as far as the new deck reaches");
    assert!(deck.revision() > revision);
}
