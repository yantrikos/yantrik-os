//! What the watcher asks of the accessibility bus, and nothing else (#1257 review, L6).
//!
//! The watcher's wiring (what it asks, of whom, and what it does with a refusal or a silence)
//! is most of what can go wrong in it, and a bus is not something a test can have. So the
//! questions are a trait: [`Atspi`] answers them from the bus, and the watcher's tests answer
//! them from a fake, with whatever delay or silence the test needs.

use std::future::Future;

use crate::atspi::Atspi;
use crate::handles::Reference;
use crate::listing::Listing;
use crate::screen::WindowInfo;

/// The questions the watcher asks. Every one is asked under a limit by the watcher, so an
/// implementation may take as long as it likes.
pub trait Questions {
    /// An object's role and name.
    fn role_and_name(&self, r: &Reference) -> impl Future<Output = Option<(String, String)>>;
    /// A window, from its own reference. `None`: gone, or not a window.
    fn window(&self, r: &Reference) -> impl Future<Output = Option<WindowInfo>>;
    /// The object one level up.
    fn parent_of(&self, r: &Reference) -> impl Future<Output = Option<Reference>>;
    /// Whether a window says it is the active one.
    fn is_active(&self, r: &Reference) -> impl Future<Output = bool>;
    /// An object's name alone.
    fn name_of(&self, r: &Reference) -> impl Future<Output = Option<String>>;
    /// Every window of every application except those `skip` names.
    fn windows(&self, skip: &dyn Fn(&str) -> bool) -> impl Future<Output = Result<Listing, String>>;
    /// The connection that is the registry now.
    fn registry_owner(&self) -> impl Future<Output = Option<String>>;
    /// Ask the registry to have one kind of event announced.
    fn register(&self, event: &str) -> impl Future<Output = Result<(), String>>;
}

impl Questions for Atspi {
    fn role_and_name(&self, r: &Reference) -> impl Future<Output = Option<(String, String)>> {
        Atspi::role_and_name(self, r)
    }

    fn window(&self, r: &Reference) -> impl Future<Output = Option<WindowInfo>> {
        Atspi::window(self, r)
    }

    fn parent_of(&self, r: &Reference) -> impl Future<Output = Option<Reference>> {
        Atspi::parent_of(self, r)
    }

    fn is_active(&self, r: &Reference) -> impl Future<Output = bool> {
        Atspi::is_active(self, r)
    }

    fn name_of(&self, r: &Reference) -> impl Future<Output = Option<String>> {
        Atspi::name_of(self, r)
    }

    fn windows(&self, skip: &dyn Fn(&str) -> bool) -> impl Future<Output = Result<Listing, String>> {
        Atspi::windows(self, skip)
    }

    fn registry_owner(&self) -> impl Future<Output = Option<String>> {
        Atspi::registry_owner(self)
    }

    async fn register(&self, event: &str) -> Result<(), String> {
        Atspi::register(self, event).await.map_err(|e| e.to_string())
    }
}
