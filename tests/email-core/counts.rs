//! The folder header when no folder list came back at all (#1089).
//!
//! Its own test target so `lib.rs`, which is far over the size limit, does not grow. It reaches
//! the app's state module through the library the other tests here are built from.

use yantrik_email_core_tests::state::{self, Counted};

#[test]
fn no_folder_list_at_all_has_not_been_counted() {
    // The look died before LIST — the session timed out while the window sat closed — so
    // there are no folders and no rows. An empty list is not a list that reports the folder
    // empty, and it has no row the header could be missing, so counting the page in hand
    // here reports a failed look as an empty inbox.
    let counts = Counted::of(&[], "INBOX", 0, 0);
    // The reason says only what is known: no list arrived. It does not say why (a timeout, or a
    // first list still on its way) and it does not blame the server for listing nothing.
    assert_eq!(counts, Counted::Unavailable { reason: "no folder list came back".into() });
    assert_eq!(counts.known(), None);
    let line = state::folder_summary("INBOX", &counts, None);
    assert!(!line.contains("0 unread of 0"), "{line} reports a failed look as an empty inbox");
    assert!(line.contains("counts unavailable"), "{line} does not say the counts are missing");
    // Rows in hand change nothing: no list means no count, whatever the list would have been.
    assert_eq!(Counted::of(&[], "INBOX", 9, 21), counts);
}
