//! The one read `describe` makes before it answers: whether the store has moved under the window.
//!
//! A mind reads `describe` immediately after acting — on this calendar, or just as often through
//! its own tools, which write to the same store — and that is exactly the moment a month read some
//! navigations ago lies. `one-calendar.py` had to step a month forward and back before every read
//! to get past this, which is a probe measuring the app's cache and knowing it. So the question is
//! put before answering, not after: two numbers over a local socket, only to a service that is
//! already listening (`calls::revision`), and the range is listed again only when the answer says
//! it moved or the window holds no read of it.
//!
//! This is the one read the app makes on the UI thread, as it always made it, and `describe` is
//! the one place that cannot wait for a job: its answer is owed now. What it read is handed back to
//! the model through the core's endpoint, so the window shows what the Mind was just told, and the
//! next `describe` finds nothing to read.

use crate::command::Command;
use crate::events::CalEvent;
use crate::model::Model;
use crate::work::{self, Listing, Read};

/// The events `describe` should report: the model's, or a fresh read when the store moved.
pub fn before_describe(m: &Model) -> Option<Vec<CalEvent>> {
    let listing = work::reread(
        m.machine.api.as_ref(),
        &m.reads.sequence,
        m.visible_range(),
        &m.held(),
        false,
        work::Start::MayStart,
    );
    let events = match &listing.read {
        Read::Unmoved => return None,
        Read::Listed { events, .. } => events.clone(),
        Read::Failed(_) => Vec::new(),
    };
    hand_back(m, listing);
    Some(events)
}

/// The read, to the model, as any other read lands.
fn hand_back(m: &Model, listing: Listing) {
    let Some(inbox) = &m.inbox else { return };
    let command = Command::Followed(Box::new(listing));
    if inbox.0.post(yantrik_app::cx::Inbound::Command(command)).is_err() {
        tracing::debug!("the app is closing; the read describe made is not kept");
    }
}
