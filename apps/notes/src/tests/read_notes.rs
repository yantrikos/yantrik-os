//! `read_notes`: many notes' text in pages a mind can read whole.

use crate::describe::{read_notes, READ_NOTE_CHARS};
use crate::note::Note;

fn note_with(title: &str, body: &str, modified: u64, trash: bool) -> Note {
    let mut n = Note::blank(title);
    n.content.text.push_str(body);
    n.modified = modified;
    n.trash = trash;
    n
}

#[test]
fn read_notes_returns_the_matching_texts_newest_first_and_never_the_trash() {
    let notes = vec![
        note_with("Standup", "Ship the demo by 9 October.", 3, false),
        note_with("Groceries", "Milk, eggs.", 2, false),
        note_with("Old plan", "Deadline 1 October.", 9, true),
        note_with("Taxes", "File by 15 October.", 5, false),
    ];
    let all = read_notes(&notes, "", 0);
    assert_eq!(all["matching"], 3, "the trashed note is left out: {all}");
    let titles: Vec<&str> = all["notes"].as_array().unwrap().iter().map(|n| n["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["Taxes", "Standup", "Groceries"], "newest first");
    assert!(all["notes"][1]["text"].as_str().unwrap().contains("9 October"), "the body itself, not only the title");
    assert_eq!(all["how_to_see_the_rest"], "", "nothing left, nothing to say");

    let october = read_notes(&notes, "OCTOBER", 0);
    assert_eq!(october["matching"], 2, "matched case-insensitively against the text: {october}");
    assert_eq!(october["left_out"], 0);
}

#[test]
fn read_notes_pages_fit_a_minds_read_and_walk_the_whole_library() {
    // Seventeen notes, some long: every page has to be read whole by a mind (whose safe reads are
    // kept to 4,000 characters), and following `skip` has to visit every note once.
    let notes: Vec<Note> = (0..17u64)
        .map(|i| {
            let body = format!("Due {i} October. {}", "y".repeat(if i % 3 == 0 { 2_500 } else { 300 }));
            note_with(&format!("Note {i}"), &body, i, false)
        })
        .collect();
    let mut seen = Vec::new();
    let mut skip = 0;
    for _ in 0..17 {
        let page = read_notes(&notes, "", skip);
        assert!(page.to_string().len() <= 4_000, "a page a mind cannot read whole: {} chars", page.to_string().len());
        let got = page["notes"].as_array().unwrap();
        assert!(!got.is_empty(), "a page with notes left must carry at least one");
        seen.extend(got.iter().map(|n| n["title"].as_str().unwrap().to_string()));
        for n in got {
            assert!(n["text"].as_str().unwrap().chars().count() <= READ_NOTE_CHARS);
        }
        skip += got.len();
        assert_eq!(page["left_out"].as_u64().unwrap() as usize, 17 - skip, "a long library is never mistaken for a short one");
        if page["left_out"] == 0 {
            break;
        }
        let hint = page["how_to_see_the_rest"].as_str().unwrap();
        assert!(hint.contains(&format!("skip {skip}")), "the next call is named exactly: {hint}");
    }
    let expected: Vec<String> = (0..17).rev().map(|i| format!("Note {i}")).collect();
    assert_eq!(seen, expected, "every note exactly once, newest first");
    let cut = read_notes(&notes, "", 0)["texts_cut_short"].as_array().unwrap().len();
    assert!(cut >= 1, "a long text says it was cut");
}

#[test]
fn read_notes_keeps_the_query_in_the_next_call_and_survives_a_skip_past_the_end() {
    let notes: Vec<Note> = (0..12u64).map(|i| note_with(&format!("Plan {i}"), &"deadline ".repeat(80), i, false)).collect();
    let first = read_notes(&notes, "deadline", 0);
    assert!(first["how_to_see_the_rest"].as_str().unwrap().contains("query \"deadline\""), "{first}");
    let past = read_notes(&notes, "deadline", 99);
    assert_eq!(past["returned"], 0);
    assert_eq!(past["left_out"], 0);
}
