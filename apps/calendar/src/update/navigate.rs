//! Where the window is: the month, the day, the view, today, and keeping what is in hand true to
//! the store and to the clock.

use std::time::Duration;

use serde_json::json;
use yantrik_app::timers::TimerId;
use yantrik_app::Refused;

use super::{apply, read, redraw, show_date, watch_read, Fx};
use crate::answers::{self, Kind};
use crate::backend::calls;
use crate::command::{Command, Step, Work};
use crate::model::{Model, NOTHING, PART_CLOCK, PART_NOTICE};
use crate::today::{self, Shown};
use crate::views::{self, ViewMode};

/// The chain of single-shot timers that wakes the window in each new day.
pub const MIDNIGHT: TimerId = TimerId("calendar.midnight");

pub fn update(m: &mut Model, command: Command, fx: &mut Fx) -> Result<(), Refused> {
    // A reading intent is answered when its own read lands, about where it put the window
    // (`answers.rs`).
    match &command {
        Command::SelectDay(_) => answers::ask(m, fx, Kind::SelectDay),
        Command::ShowDate(date) => answers::ask(m, fx, Kind::ShowDate(date.clone())),
        Command::ShowMonth(_) => answers::ask(m, fx, Kind::ShowMonth),
        Command::Today => answers::ask(m, fx, Kind::Today),
        Command::SetView(_) => answers::ask(m, fx, Kind::SetView),
        _ => {}
    }
    match command {
        Command::PrevMonth | Command::ShowMonth(Step::Previous) => step_month(m, fx, Step::Previous),
        Command::NextMonth | Command::ShowMonth(Step::Next) => step_month(m, fx, Step::Next),
        Command::DayClicked(day) | Command::SelectDay(day) => {
            // Not just the agenda: picking a day moves the day view onto it and the week view
            // onto the week around it, and a day in the first or last week of the month needs
            // days the month fetch did not ask for.
            m.selected_day = day;
            moved(m, fx);
        }
        Command::SwitchView(mode) => {
            m.view = ViewMode::from_index(mode);
            moved(m, fx);
        }
        Command::SetView(view) => {
            m.view = view;
            moved(m, fx);
        }
        Command::Today => {
            // The clock is read here, at the press, so the button goes to the real date even if
            // midnight passed since the window last looked.
            let clock = m.machine.clock.clone();
            m.today.check(clock.as_ref());
            let target = Shown::on(m.today.date());
            m.year = target.year;
            m.month = target.month;
            m.selected_day = target.day as i32;
            moved(m, fx);
        }
        Command::ShowDate(date) => {
            show_date(m, &date);
            moved(m, fx);
        }
        Command::Refresh => {
            read(m, fx, false);
            fx.changed(NOTHING);
        }
        Command::Minute => {
            // The week and day grids are one pixel to the minute, so the line has to be told the
            // minute as well as the hour or it stands up to an hour away from where the person
            // is. A minute is as often as a line that moves a pixel a minute can usefully be
            // redrawn.
            m.now = m.machine.clock.now();
            m.utc_offset = m.machine.clock.utc_offset_seconds();
            fx.changed(PART_CLOCK);
        }
        Command::Midnight => {
            follow_today(m, fx);
            arm_midnight(m, fx);
        }
        Command::Activated => {
            follow_today(m, fx);
        }
        Command::Watch => {
            // The date first: a window that was put away over midnight should come back on the
            // right day. This also catches a midnight the timer slept through on a suspended
            // machine. Then the store, unless a read is already out.
            if !follow_today(m, fx) && m.reads.in_flight == 0 {
                watch_read(m, fx);
            }
        }
        Command::Listed(listing) => {
            m.reads.in_flight = m.reads.in_flight.saturating_sub(1);
            answers::read_landed(m, fx, &listing);
            landed(m, fx, *listing);
        }
        Command::Followed(listing) => landed(m, fx, *listing),
        Command::ReadFailed => {
            // The next redraw or tick reads again; what is on screen stays the last read taken.
            m.reads.in_flight = m.reads.in_flight.saturating_sub(1);
            answers::read_failed(m, fx);
            fx.changed(NOTHING);
        }
        Command::OpenNoticeEvent => {
            // Read from the store by id at the press, not remembered from the refusal: the event
            // may have moved since, or gone. The window goes to its day, where it is in the day's
            // list with its own delete. The notice stays up — the person opened it to act on what
            // it says.
            let id = m.notice.event.clone();
            fx.changed(NOTHING);
            if id.is_empty() {
                return Ok(());
            }
            let api = m.machine.api.clone();
            fx.spawn_or(Command::WorkFailed(Work::Open), move |_| {
                Command::NoticeEvent(calls::get(api.as_ref(), &id))
            });
        }
        Command::NoticeEvent(found) => match found {
            Ok(event) => {
                show_date(m, &event.start);
                moved(m, fx);
            }
            Err(e) => {
                m.notice.say(format!("Could not open that event: {e}"));
                fx.changed(PART_NOTICE);
            }
        },
        Command::Between((from, to)) => {
            // A read: it asks the store, not the grid, so a mind never has to steer the view
            // (`set_view`, `show_date`) to see a range. On 28 Sep - 4 Oct a mind could not get
            // the Monday week out of a Sunday-first view and gave up without writing its plan.
            fx.changed(NOTHING);
            let api = m.machine.api.clone();
            fx.spawn_or(Command::WorkFailed(Work::Between), move |_| {
                let answer = calls::service_events(api.as_ref(), from, to).map(|events| {
                    let listed = views::events_between(&events, from, to);
                    json!({
                        "from": from.to_string(),
                        "to": to.to_string(),
                        "count": listed.len(),
                        "events": listed,
                    })
                });
                Command::BetweenRead(answer)
            });
        }
        Command::BetweenRead(answer) => {
            fx.changed(NOTHING);
            match answer {
                Ok(answer) => answers::settle(m, fx, answer),
                Err(e) => fx.settle(Err(Refused::new("store", e))),
            }
        }
        other => unreachable!("not a navigation command: {other:?}"),
    }
    Ok(())
}

/// A read landed: drawn when it is the window's and replaced what was in hand.
fn landed(m: &mut Model, fx: &mut Fx, listing: crate::work::Listing) {
    let replaces = !matches!(listing.read, crate::work::Read::Unmoved);
    if apply(m, listing) && replaces {
        redraw(m, fx);
    } else {
        fx.changed(NOTHING);
    }
}

/// The window moved, or what it shows did: read the store if anything says to, and draw. A
/// reading intent's read lists whatever the revision says, because its answer is built from what
/// that read found; when the revision cannot be asked it keeps the range in hand (`work::reread`).
fn moved(m: &mut Model, fx: &mut Fx) {
    let owed = answers::target_now(m, fx);
    read(m, fx, owed);
    redraw(m, fx);
}

/// One month back or forward. Nothing is picked in the month just arrived at; the week and day
/// views read that as the first of it (`views::selected_date`).
fn step_month(m: &mut Model, fx: &mut Fx, step: Step) {
    match step {
        Step::Previous if m.month == 1 => {
            m.month = 12;
            m.year -= 1;
        }
        Step::Previous => m.month -= 1,
        Step::Next if m.month == 12 => {
            m.month = 1;
            m.year += 1;
        }
        Step::Next => m.month += 1,
    }
    m.selected_day = 0;
    moved(m, fx);
}

/// Notice that the date has turned, and follow it. Says whether it had.
///
/// Called at midnight, when the window is brought back to the front, and on the store watch's
/// tick. Nothing happens while it is still the day last seen. When it is not, the marker moves to
/// the real date, and a window that was showing the old today moves with it; one the person had
/// moved elsewhere stays where it is (`today::follow`).
fn follow_today(m: &mut Model, fx: &mut Fx) -> bool {
    let clock = m.machine.clock.clone();
    let Some(was) = m.today.check(clock.as_ref()) else {
        fx.changed(NOTHING);
        return false;
    };
    let shown = Shown { year: m.year, month: m.month, day: m.selected_day.max(0) as u32 };
    let next = today::follow(shown, was, m.today.date());
    m.year = next.year;
    m.month = next.month;
    m.selected_day = next.day as i32;
    moved(m, fx);
    true
}

/// Wake at the next midnight, and a second past it. Midnight is a chain of single-shot timers,
/// each set for the next midnight by the one before; the runtime owns them, so they end with the
/// app.
pub fn arm_midnight(m: &crate::model::Model, fx: &mut Fx) {
    let wait: Duration = today::until_midnight(m.machine.clock.now());
    fx.after(MIDNIGHT, wait, Command::Midnight);
}
