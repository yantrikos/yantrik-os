//! The handles a caller names windows and elements by.
//!
//! A caller never sees a bus name or an object path: it gets `w17`, and the only handles that
//! resolve are ones this service handed out. Two things changed when the screen model arrived
//! (#1256), and both live here:
//!
//! - **One object, one handle.** An event names a window by its handle and a reader passes that
//!   handle to `a11y.describe`, so a window must keep its handle for as long as it is open. The
//!   table used to mint a new one on every listing.
//! - **Shared, and bounded.** The watcher and the request thread hand out from the same table,
//!   and the table forgets its oldest handles past [`CAPACITY`] instead of growing for as long as
//!   the session lasts. A window the model still holds is pinned and never forgotten.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use zbus::zvariant::OwnedObjectPath;

/// An AT-SPI object reference: which connection, and where on it.
pub type Reference = (String, OwnedObjectPath);

/// Handles kept at most. A described window is up to 120 elements, so this is some hundreds of
/// windows' worth before the oldest element handles are forgotten.
const CAPACITY: usize = 20_000;

#[derive(Default)]
pub struct Handles {
    by_id: HashMap<String, Reference>,
    by_reference: HashMap<Reference, String>,
    /// Oldest first, for forgetting.
    order: VecDeque<String>,
    pinned: HashSet<String>,
    next: u64,
}

/// The table as the two threads share it.
pub type Shared = Arc<Mutex<Handles>>;

impl Handles {
    /// The handle for an object: the one it already has, or a new one.
    pub fn hand_out(&mut self, reference: Reference) -> String {
        if let Some(id) = self.by_reference.get(&reference) {
            return id.clone();
        }
        self.next += 1;
        let id = format!("w{}", self.next);
        self.by_id.insert(id.clone(), reference.clone());
        self.by_reference.insert(reference, id.clone());
        self.order.push_back(id.clone());
        self.forget_past(CAPACITY);
        id
    }

    pub fn resolve(&self, id: &str) -> Option<Reference> {
        self.by_id.get(id).cloned()
    }

    /// Keep this handle for as long as its window is open, whatever its age.
    pub fn pin(&mut self, id: &str) {
        if self.by_id.contains_key(id) {
            self.pinned.insert(id.to_string());
        }
    }

    /// The window closed: its handle resolves to nothing from now on.
    pub fn forget(&mut self, id: &str) {
        self.pinned.remove(id);
        if let Some(reference) = self.by_id.remove(id) {
            self.by_reference.remove(&reference);
        }
        // `order` keeps the id until it reaches the front; `forget_past` skips ids that are gone.
    }

    /// Forget the oldest handles until at most `capacity` are left, passing over pinned ones.
    fn forget_past(&mut self, capacity: usize) {
        let mut passes = self.order.len();
        while self.by_id.len() > capacity && passes > 0 {
            passes -= 1;
            let Some(id) = self.order.pop_front() else { break };
            if self.pinned.contains(&id) {
                self.order.push_back(id);
            } else if let Some(reference) = self.by_id.remove(&id) {
                self.by_reference.remove(&reference);
            }
        }
    }
}

/// Hand out under the shared lock. A poisoned lock means a thread panicked while holding the
/// table; the table itself is still sound, so it is used as it stands.
pub fn hand_out(shared: &Shared, reference: Reference) -> String {
    shared.lock().unwrap_or_else(|e| e.into_inner()).hand_out(reference)
}

pub fn resolve(shared: &Shared, id: &str) -> Option<Reference> {
    shared.lock().unwrap_or_else(|e| e.into_inner()).resolve(id)
}

pub fn pin(shared: &Shared, id: &str) {
    shared.lock().unwrap_or_else(|e| e.into_inner()).pin(id);
}

pub fn forget(shared: &Shared, id: &str) {
    shared.lock().unwrap_or_else(|e| e.into_inner()).forget(id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::ObjectPath;

    fn reference(n: usize) -> Reference {
        let path = ObjectPath::try_from(format!("/org/a11y/atspi/accessible/{n}")).expect("a valid path");
        (":1.9".to_string(), path.into())
    }

    #[test]
    fn the_same_object_keeps_its_handle() {
        let mut handles = Handles::default();
        let first = handles.hand_out(reference(1));
        assert_eq!(handles.hand_out(reference(1)), first, "listed twice, one handle");
        assert_ne!(handles.hand_out(reference(2)), first);
        assert_eq!(handles.resolve(&first), Some(reference(1)));
    }

    #[test]
    fn a_handle_nobody_handed_out_resolves_to_nothing() {
        let handles = Handles::default();
        assert_eq!(handles.resolve("w1"), None);
        assert_eq!(handles.resolve("/org/a11y/atspi/accessible/1"), None, "a path is not a handle");
    }

    #[test]
    fn a_forgotten_window_resolves_to_nothing_and_gets_a_new_handle_if_it_returns() {
        let mut handles = Handles::default();
        let id = handles.hand_out(reference(1));
        handles.pin(&id);
        handles.forget(&id);
        assert_eq!(handles.resolve(&id), None);
        assert_ne!(handles.hand_out(reference(1)), id, "a handle is never reused for another life");
    }

    #[test]
    fn the_oldest_handles_are_forgotten_past_the_capacity_but_never_a_pinned_one() {
        let mut handles = Handles::default();
        let window = handles.hand_out(reference(0));
        handles.pin(&window);
        let elements: Vec<String> = (1..=10).map(|n| handles.hand_out(reference(n))).collect();
        handles.forget_past(4);
        assert_eq!(handles.by_id.len(), 4);
        assert_eq!(handles.resolve(&window), Some(reference(0)), "the open window keeps its handle");
        assert_eq!(handles.resolve(&elements[0]), None, "the oldest element is gone");
        assert_eq!(handles.resolve(&elements[9]), Some(reference(10)), "the newest stays");
        assert_eq!(handles.by_reference.len(), 4, "both sides of the table agree");
    }

    #[test]
    fn forgetting_stops_when_everything_left_is_pinned() {
        let mut handles = Handles::default();
        for n in 0..5 {
            let id = handles.hand_out(reference(n));
            handles.pin(&id);
        }
        handles.forget_past(2);
        assert_eq!(handles.by_id.len(), 5, "pinned handles are kept, and the loop ends");
    }
}
