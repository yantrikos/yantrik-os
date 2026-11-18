//! Calendar, driven headless through `yantrik-testing`: the window, the jobs and the clock run when
//! each test says, against an in-memory calendar service (`backend::fake`), a table of callers and
//! a fake Mind.
//!
//! - `parity.rs`: the published actions and `describe`, pinned to what the app published before
//!   the framework; `describe.rs`: the day, the names behind the ids, and who is asking.
//! - `window.rs`: the buttons, the form, the agenda's trash icon, the rail and the card.
//! - `reading.rs` and `writing.rs`: each intent through the real dispatch.
//! - `store.rs`: following the store and the clock, the order reads land in, and jobs that fail.
//! - `ending.rs` and `closing.rs`: a logout, the display going, and the window closed, with and
//!   without a write in flight.

use std::sync::Arc;

use serde_json::Value;
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::{App, Paths, Services};
use yantrik_testing::{Pending, TestApp};

use crate::backend::fake::{self, FakeCalendar};
use crate::today::FakeClock;
use crate::Calendar;

mod closing;
mod describe;
mod ending;
mod parity;
mod reading;
mod rest;
mod store;
mod window;
mod writing;

/// Wednesday 23 September 2026, ten in the morning.
pub(crate) const NOW: &str = "2026-09-23 10:00:00";

/// An app under test, with the fakes it talks to.
pub(crate) struct Fixture {
    pub app: TestApp<Calendar>,
    pub store: FakeCalendar,
    pub clock: Arc<FakeClock>,
    pub mind: Arc<FakeMind>,
}

/// A day with two things on it and one later in the week, as the store holds them.
pub(crate) fn a_week() -> FakeCalendar {
    FakeCalendar::default()
        .holding("ev-standup", "Standup", "2026-09-23T09:30:00", "2026-09-23T09:45:00")
        .holding("ev-review", "Design review", "2026-09-23T14:00:00", "2026-09-23T15:30:00")
        .holding("ev-dentist", "Dentist", "2026-09-25T13:00:00", "2026-09-25T14:00:00")
}

/// Started at [`NOW`] over `store`, with the first read landed.
pub(crate) fn open(store: FakeCalendar) -> Fixture {
    open_with(store, FakeMind::new())
}

pub(crate) fn open_with(store: FakeCalendar, mind: FakeMind) -> Fixture {
    start(store, mind, NOW)
}

/// Started at `now` (`YYYY-MM-DD HH:MM:SS`) over `store`, with the first read landed.
pub(crate) fn open_at(store: FakeCalendar, now: &str) -> Fixture {
    start(store, FakeMind::new(), now)
}

fn start(store: FakeCalendar, mind: FakeMind, now: &str) -> Fixture {
    let paths = Paths::temporary(Calendar::ID);
    let clock = Arc::new(FakeClock::at(now));
    fake::install(&paths, fake::machine(&store, &clock));
    let mind = Arc::new(mind);
    let app = TestApp::<Calendar>::start_at_with(paths, Vec::new(), mind.clone(), Services::fakes());
    let fixture = Fixture { app, store, clock, mind };
    fixture.quiet();
    fixture
}

impl Fixture {
    /// Runs jobs, and what they posted, until none is left.
    pub fn quiet(&self) {
        const ROUNDS: usize = 16;
        for _ in 0..ROUNDS {
            self.app.tick();
            if self.app.jobs_waiting() == 0 {
                return;
            }
            self.app.run_jobs();
        }
        panic!("jobs still waiting after {ROUNDS} rounds");
    }

    /// The person's own call of an intent, answered once its work has landed.
    pub fn act(&self, name: &str, args: Value) -> Result<Value, String> {
        let pending = self.app.act(name, args)?;
        self.settled(pending)
    }

    pub fn settled(&self, pending: Pending) -> Result<Value, String> {
        self.quiet();
        pending.settle()
    }

    /// `describe`, and the read it may have handed back to the window applied.
    pub fn describe(&self) -> Value {
        let described = self.app.describe();
        self.quiet();
        described
    }

    pub fn state(&self) -> Value {
        self.describe()["state"].clone()
    }

    pub fn summary(&self) -> String {
        self.describe()["summary"].as_str().unwrap_or_default().to_string()
    }

    pub fn notice(&self) -> String {
        self.app.ui(|ui| ui.get_notice().to_string())
    }

    /// The agenda's titles, as the window draws them.
    pub fn agenda(&self) -> Vec<String> {
        use slint::Model as _;
        self.app.ui(|ui| ui.get_events_today().iter().map(|e| e.title.to_string()).collect())
    }
}
