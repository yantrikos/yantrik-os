//! How much text the open tabs may hold between them.
//!
//! There used to be a fixed eight tabs, with no reason written down for eight ("But why
//! limit?", Pranab, 4 October 2026). What a tab costs is the text it holds, at most
//! [`MAX_BYTES`] each, so the rule is about bytes: a tab is opened, created or edited only while
//! the open tabs together hold no more than [`MAX_OPEN_BYTES`]. A tab is counted at the larger of
//! its text and the saved copy it keeps beside it ([`weight`]).

use super::limits::MAX_BYTES;

/// How much text all open tabs may hold between them.
pub const MAX_OPEN_BYTES: usize = 64 * MAX_BYTES;

/// The most tabs open at once, however small, so the tab strip and the drafts stay bounded. Above
/// the 64 a full budget of 1 MiB files fills, so the budget is what bites first.
pub const MAX_TABS: usize = 128;

/// One open tab as the budget sees it: the length of its text and of its saved copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Weighed {
    pub text: usize,
    pub saved: usize,
}

/// What one tab counts against the budget: the larger of its text and its saved copy.
///
/// Counting the text alone let a tab cost nothing while holding a 1 MiB saved copy: open a 1 MiB
/// file, empty it, and repeat, and 128 tabs held 128 MiB outside a 64 MiB budget (confirm pass on
/// #620). The larger of the two, not their sum, because in the ordinary case they are the same
/// text: a tab just opened or saved costs its file once.
pub fn weight(tab: Weighed) -> usize {
    tab.text.max(tab.saved)
}

/// What all of `tabs` count against [`MAX_OPEN_BYTES`] between them.
pub fn open_bytes(tabs: &[Weighed]) -> usize {
    tabs.iter().copied().map(weight).sum()
}

/// Whether one more tab weighing `adding` bytes fits beside `tabs`. A tab opened from a file has
/// the same text and saved copy, so its weight is the file's length.
pub fn room_for(tabs: &[Weighed], adding: usize) -> bool {
    tabs.len() < MAX_TABS && open_bytes(tabs).saturating_add(adding) <= MAX_OPEN_BYTES
}

/// Whether tab `at` may come to hold `len` bytes: within the budget beside the other tabs, or no
/// bigger than it is now. Opening was not enough to hold to (security review of #620): an empty
/// tab always fits, so 128 of them, each then filled with 1 MiB by `append`, held twice the
/// budget.
pub fn fits(tabs: &[Weighed], at: usize, len: usize) -> bool {
    let Some(tab) = tabs.get(at).copied() else { return false };
    let now = weight(tab);
    let then = len.max(tab.saved);
    then <= now || open_bytes(tabs) - now + then <= MAX_OPEN_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(text: usize, saved: usize) -> Weighed {
        Weighed { text, saved }
    }

    #[test]
    fn the_open_budget_is_bytes_not_eight_tabs() {
        let small: Vec<Weighed> = (0..60).map(|_| tab(10, 10)).collect();
        assert!(room_for(&small, 10), "sixty small tabs are nowhere near the budget");
        let full: Vec<Weighed> = (0..64).map(|_| tab(MAX_BYTES, MAX_BYTES)).collect();
        assert!(!room_for(&full, 1), "64 full tabs are the whole budget");
        assert!(room_for(&full[..63], MAX_BYTES), "63 leave room for one more");
        let many: Vec<Weighed> = (0..MAX_TABS).map(|_| tab(0, 0)).collect();
        assert!(!room_for(&many, 0), "the strip is bounded however small the tabs");
    }

    #[test]
    fn a_tab_is_counted_at_its_saved_copy_too() {
        // A 1 MiB file opened and emptied still costs its saved copy.
        assert_eq!(weight(tab(0, MAX_BYTES)), MAX_BYTES);
        let tabs: Vec<Weighed> = (0..64).map(|_| tab(0, MAX_BYTES)).collect();
        assert!(!room_for(&tabs, 1));
        // Shrinking always fits; growing past the budget does not.
        assert!(fits(&tabs, 0, 10));
        let mut nearly: Vec<Weighed> = (0..63).map(|_| tab(MAX_BYTES, MAX_BYTES)).collect();
        nearly.push(tab(0, 0));
        assert!(fits(&nearly, 63, MAX_BYTES), "the 64th MiB fits");
        nearly[63] = tab(MAX_BYTES, 0);
        nearly.push(tab(0, 0));
        assert!(!fits(&nearly, 64, 1), "past the 64th it does not");
        assert!(!fits(&nearly, 99, 1), "no such tab");
    }
}
