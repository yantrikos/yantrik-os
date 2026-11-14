//! The person's own opening of what a prepared card found (#1184).
//!
//! The Mind opens nothing for a prepared item: the shell refuses a Mind a window of any framework
//! app (#978 step 1). So the card gives the person the buttons instead, and a press opens the file
//! or the draft **as the person**, through the path a click on a file uses (`open_with::launch`,
//! after `mime_dispatch::classify`), never through the Mind's `open_app` / `app_action`.
//!
//! The card in Slint carries only labels. The paths and the commitment ids stay here, in the cards
//! the companion last sent, and a press names a card (its item) and a file's place, so a press can
//! only open what the companion located for that card.

use std::path::{Path, PathBuf};

use yantrik_companion::commitment_cards::{Card, MOST_FILES_SHOWN};

/// The longest file name a button shows whole; a longer one is cut in the middle, keeping its end
/// (the extension), so "Open <name>" fits the 300px card at one line.
const LONGEST_NAME: usize = 24;

/// What the card shows for its openings: the labels of the first two files, "and N more", and the
/// draft's label. Empty is "not shown".
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Openings {
    pub file1: String,
    pub file2: String,
    pub more: String,
    pub draft: String,
}

pub fn openings(card: &Card) -> Openings {
    let label = |i: usize| card.files.get(i).map(|f| format!("Open {}", elide(&file_name(&f.path, &f.label)))).unwrap_or_default();
    let more = card.files.len().saturating_sub(MOST_FILES_SHOWN);
    Openings {
        file1: label(0),
        file2: label(1),
        more: if more > 0 { format!("and {more} more") } else { String::new() },
        draft: if card.draft_for.is_some() { "Open the draft".into() } else { String::new() },
    }
}

fn file_name(path: &str, fallback: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| fallback.to_string())
}

/// `name`, or its head and tail around an ellipsis when it is long.
pub fn elide(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= LONGEST_NAME {
        return name.to_string();
    }
    let (head, tail) = (LONGEST_NAME / 2 - 1, LONGEST_NAME / 2 - 2);
    format!("{}\u{2026}{}", chars[..head].iter().collect::<String>(), chars[chars.len() - tail..].iter().collect::<String>())
}

/// The file the card `item` offers at place `index` (0 or 1): only what the companion located for
/// that card, and only the places the card shows.
pub fn file_for(cards: &[Card], item: &str, index: usize) -> Option<PathBuf> {
    if index >= MOST_FILES_SHOWN {
        return None;
    }
    cards.iter().find(|c| c.item == item)?.files.get(index).map(|f| PathBuf::from(&f.path))
}

/// The commitment whose draft the card `item` offers, if it offers one.
pub fn draft_for(cards: &[Card], item: &str) -> Option<String> {
    cards.iter().find(|c| c.item == item)?.draft_for.clone()
}

/// The person's click on a file: the app the one MIME rule picks, launched as Files launches it.
/// `open` is how the file is launched; the shell passes [`person_open`].
pub fn open_file(cards: &[Card], item: &str, index: usize, open: impl FnOnce(&Path, &str)) -> bool {
    let Some(path) = file_for(cards, item, index) else { return false };
    let name = file_name(&path.to_string_lossy(), "");
    open(&path, &name);
    true
}

/// A file opened the way a person's click on it opens it. A Mind never reaches this: it is called
/// only from the card's pointer-only buttons.
pub fn person_open(path: &Path, name: &str, ui: &crate::App) {
    let action = crate::mime_dispatch::classify(name);
    super::open_with::launch(&action, name, path, Some(ui));
}

/// What a launch of Email names to have it open the draft prepared for a commitment: Email maps a
/// file `<dir>/<commitment id>` (no such file exists; the name is the promise) to its person-only
/// `OpenPreparedFor`. The same directory is `PREPARED_LAUNCH_DIR` in apps/email/src/lib.rs. A file
/// launch, not an intent: the person's own buttons are no action a Mind is shown.
const PREPARED_LAUNCH_DIR: &str = "/yantrik-prepared-draft";

/// The launch argument for the draft of `commitment`. A commitment id is digits only.
pub fn draft_launch_arg(commitment: &str) -> Option<String> {
    (!commitment.is_empty() && commitment.chars().all(|c| c.is_ascii_digit())).then(|| format!("{PREPARED_LAUNCH_DIR}/{commitment}"))
}

/// Open the draft in Email, as the person.
pub fn open_draft(commitment: &str) -> bool {
    let Some(arg) = draft_launch_arg(commitment) else { return false };
    super::dock::spawn_app_with_args("email", "yantrik-email", &[&arg]);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_companion::commitment_cards::CardFile;

    fn card(files: &[&str], draft: Option<&str>) -> Card {
        Card {
            item: "commitment-prep-7".into(),
            title: "Got something ready".into(),
            text: String::new(),
            yes_id: "y".into(),
            yes_label: "Got it",
            no_id: "n".into(),
            no_label: "Stop getting things ready",
            files: files.iter().map(|p| CardFile { label: "x".into(), path: (*p).into() }).collect(),
            draft_for: draft.map(String::from),
        }
    }

    #[test]
    fn two_files_and_a_draft_are_three_buttons_and_no_more_text() {
        let o = openings(&card(&["/h/Documents/Q4-deck.ydeck", "/h/Documents/budget.csv"], Some("7")));
        assert_eq!(o, Openings { file1: "Open Q4-deck.ydeck".into(), file2: "Open budget.csv".into(), more: String::new(), draft: "Open the draft".into() });
    }

    #[test]
    fn more_than_two_files_show_two_and_say_how_many_more() {
        let o = openings(&card(&["/a/1.csv", "/a/2.csv", "/a/3.csv", "/a/4.csv"], None));
        assert_eq!((o.file1.as_str(), o.file2.as_str(), o.more.as_str(), o.draft.as_str()), ("Open 1.csv", "Open 2.csv", "and 2 more", ""));
    }

    #[test]
    fn a_long_name_is_elided_in_the_middle_and_keeps_its_end() {
        let long = "quarterly-budget-review-final-v3.xlsx";
        let shown = elide(long);
        assert!(shown.chars().count() <= LONGEST_NAME && shown.contains('\u{2026}') && shown.ends_with(".xlsx"), "{shown}");
        assert_eq!(elide("short.csv"), "short.csv");
    }

    #[test]
    fn a_press_opens_only_what_the_card_located_and_only_the_two_it_shows() {
        let cards = [card(&["/a/1.csv", "/a/2.csv", "/a/3.csv"], None)];
        assert_eq!(file_for(&cards, "commitment-prep-7", 1), Some(PathBuf::from("/a/2.csv")));
        assert_eq!(file_for(&cards, "commitment-prep-7", 2), None, "a third file has no button");
        assert_eq!(file_for(&cards, "commitment-prep-8", 0), None, "an unknown card opens nothing");
    }

    #[test]
    fn a_press_hands_the_located_path_and_its_name_to_the_opener_and_a_third_file_to_none() {
        let cards = [card(&["/h/Documents/budget.csv"], Some("7"))];
        let mut seen = None;
        assert!(open_file(&cards, "commitment-prep-7", 0, |p, name| seen = Some((p.to_path_buf(), name.to_string()))));
        assert_eq!(seen, Some((PathBuf::from("/h/Documents/budget.csv"), "budget.csv".to_string())));
        // Only the hand-over is proved here; that `person_open` runs `classify` then `launch` is VM 520's.
        assert!(!open_file(&cards, "commitment-prep-7", 5, |_, _| panic!("opened a file the card does not show")));
    }

    #[test]
    fn the_draft_is_opened_by_a_launch_for_its_own_commitment_and_nothing_else() {
        let cards = [card(&[], Some("7")), Card { item: "commitment-prep-8".into(), ..card(&["/a/1.csv"], None) }];
        assert_eq!(draft_for(&cards, "commitment-prep-7").as_deref(), Some("7"));
        assert_eq!(draft_for(&cards, "commitment-prep-8"), None);
        assert_eq!(draft_launch_arg("7").as_deref(), Some("/yantrik-prepared-draft/7"));
        assert_eq!(draft_launch_arg("7/../../etc"), None);
        assert_eq!(draft_launch_arg(""), None);
    }

    #[test]
    fn a_press_on_a_card_no_longer_held_opens_nothing() {
        // Rust holds no card, or a different one: neither the file nor the draft of the card the
        // press names is opened. The check stays; its sentence to the person is gone (a log line).
        for held in [Vec::new(), vec![Card { item: "commitment-prep-8".into(), ..card(&["/a/other.csv"], Some("8")) }]] {
            assert!(!open_file(&held, "commitment-prep-7", 0, |_, _| panic!("opened a file of a card that is not held")));
            assert_eq!(draft_for(&held, "commitment-prep-7"), None);
        }
    }
}
