//! The only code that changes the model: every button, intent, timer and job result lands here
//! as a `Command`, and asks for its effects.
//!
//! The person's buttons and the Mind's intents are the same commands where they do the same
//! thing: a day picked, a month stepped, an event stored, taken off or moved go through one path
//! each, so neither can report an outcome the other would not. Every read of the store and every
//! write runs in a job (`work.rs`); what it found comes back here as a command.

use yantrik_app::effects::Effects;
use yantrik_app::Refused;

use crate::command::{Command, Work};
use crate::events;
use crate::model::{Model, NOTHING, PART_CALENDAR, PART_CLOCK};
use crate::work::{self, Listing, Read, Start};
use crate::Calendar;

mod ai;
mod change;
mod form;
mod navigate;
mod remove;

/// The rail's suggestion and the header's AI button ask the same question.
pub use ai::EXPLAIN;
/// The rail's other suggestion: back to today.
pub use ai::TODAY;
/// The card an answer about the day is drawn on, by the rail or by the header's own ask.
pub use ai::answer_card;
/// The first wake at midnight, armed at start.
pub use navigate::arm_midnight;

type Fx = Effects<Calendar>;

/// How long a close waits for the writes in flight before the framework ends the app anyway
/// (`fx.linger`'s backstop, which logs that it did). Each call is bounded by the client's own
/// timeout, so this is reached only by a service that stopped answering.
pub const CLOSE_BACKSTOP: std::time::Duration = std::time::Duration::from_secs(10);

pub fn update(m: &mut Model, command: Command, fx: &mut Fx) -> Result<(), Refused> {
    if (m.ending || m.closing) && starts_a_write(&command) {
        return Err(Refused::new("closing", "Calendar is closing, so nothing more is written."));
    }
    if write_landed(&command) {
        m.writes = m.writes.saturating_sub(1);
    }
    let result = dispatch(m, command, fx);
    // The end waits for the writes already sent, so none is lost to a logout: it is the store's
    // answer to each that says whether it happened, and the window is gone after it.
    if (m.ending || m.closing) && m.writes == 0 {
        fx.exit();
    }
    result
}

/// A command that sends something to the store to be written.
fn starts_a_write(command: &Command) -> bool {
    matches!(
        command,
        Command::SaveForm { .. } | Command::Add(_) | Command::DeleteRow(_) | Command::Delete { .. } | Command::Update { .. }
    )
}

/// A command that carries a write's outcome back: the store's answer, a refusal from its job, or
/// the job ending without a result.
fn write_landed(command: &Command) -> bool {
    matches!(
        command,
        Command::Saved(_)
            | Command::Removed(_)
            | Command::Moved(_)
            | Command::Kept(_)
            | Command::Declined(_)
            | Command::WorkFailed(Work::Save(_) | Work::Remove(_) | Work::Change)
    )
}

fn dispatch(m: &mut Model, command: Command, fx: &mut Fx) -> Result<(), Refused> {
    match command {
        Command::PrevMonth
        | Command::NextMonth
        | Command::ShowMonth(_)
        | Command::DayClicked(_)
        | Command::SelectDay(_)
        | Command::SwitchView(_)
        | Command::SetView(_)
        | Command::Today
        | Command::ShowDate(_)
        | Command::Refresh
        | Command::Minute
        | Command::Midnight
        | Command::Watch
        | Command::Activated
        | Command::Listed(_)
        | Command::Followed(_)
        | Command::ReadFailed
        | Command::OpenNoticeEvent
        | Command::NoticeEvent(_)
        | Command::Between(_)
        | Command::BetweenRead(_) => navigate::update(m, command, fx),
        Command::OpenForm
        | Command::UseTemplate { .. }
        | Command::SaveForm { .. }
        | Command::CancelForm
        | Command::Add(_)
        | Command::Saved(_) => form::update(m, command, fx),
        Command::DeleteRow(_) | Command::Delete { .. } | Command::Removed(_) => remove::update(m, command, fx),
        Command::Update { .. } | Command::Moved(_) => change::update(m, command, fx),
        Command::Kept(kept) => {
            // The caller gets the rule, with the ids and the other door; the person gets which
            // event, that it is untouched, and who asked.
            m.notice.kept(kept.refused, &kept.title, kept.who.as_deref(), &kept.id);
            fx.changed(crate::model::PART_NOTICE);
            fx.settle(Err(Refused::new("not_yours", kept.why)));
            Ok(())
        }
        Command::Declined(why) => {
            fx.changed(NOTHING);
            fx.settle(Err(Refused::new("not_found", why)));
            Ok(())
        }
        Command::WorkFailed(work) => {
            remove::work_failed(m, work, fx);
            Ok(())
        }
        Command::DismissNotice => {
            m.notice.say("");
            fx.changed(crate::model::PART_NOTICE);
            Ok(())
        }
        Command::Explain | Command::DismissAi | Command::Explained(_) | Command::Rail(_) => {
            ai::update(m, command, fx)
        }
        Command::Attach(inbox) => {
            m.inbox = Some(inbox);
            fx.changed(NOTHING);
            Ok(())
        }
        Command::Ending => {
            // Nothing asks first: a logout has nobody there to answer, and a draft the person
            // has typed but not saved is not kept, as it never was. A write already sent is
            // waited for; `update` ends the app once none is left.
            m.ending = true;
            fx.changed(NOTHING);
            Ok(())
        }
        Command::Closing(window) => {
            // The window closes at once, as the person asked; the process runs on without it
            // (`fx.linger`) until the writes already sent have landed, as it did when they ran on
            // the UI thread and the close could not be handled before them. Then `update` ends
            // the app. A window that is really closed cannot be brought back by a stale
            // activation; only a second launch opens it again (`Event::Presented`).
            m.closing = true;
            fx.changed(NOTHING);
            fx.close(window);
            fx.linger(CLOSE_BACKSTOP);
            Ok(())
        }
        Command::Reopened => {
            reopen(m, fx);
            Ok(())
        }
    }
}

/// A second launch opened the window again while a close waited on writes (`Event::Presented`):
/// the linger is over, and so is the close. The writes in flight land as any other; new writes are
/// taken again. A logout's ending is not a close and is not cancelled. After `fx.exit()` the
/// framework opens no window, so nothing here can bring an ending app back.
fn reopen(m: &mut Model, fx: &mut Fx) {
    fx.changed(NOTHING);
    if m.closing {
        m.closing = false;
    }
}

/// Read the store for the range on screen, in a job. Without `force` the job asks the store's
/// revision first and lists only when the range or the store moved (`work::reread`).
pub(crate) fn read(m: &mut Model, fx: &mut Fx, force: bool) {
    spawn_read(m, fx, force, Start::MayStart);
}

/// The store read again after a write, for wherever the window is when the write lands: in a job
/// of its own, after the write's own result, so a read that fails cannot unsay the write.
pub(crate) fn read_back(m: &mut Model, fx: &mut Fx) {
    read(m, fx, true);
    redraw(m, fx);
}

/// The watch's read: from a service already listening, never one it starts.
pub(crate) fn watch_read(m: &mut Model, fx: &mut Fx) {
    spawn_read(m, fx, false, Start::Never);
}

fn spawn_read(m: &mut Model, fx: &mut Fx, force: bool, start: Start) {
    let wanted = m.visible_range();
    let held = m.held();
    let api = m.machine.api.clone();
    let sequence = m.reads.sequence.clone();
    m.reads.in_flight += 1;
    fx.spawn_or(Command::ReadFailed, move |_| {
        Command::Listed(Box::new(work::reread(api.as_ref(), &sequence, wanted, &held, force, start)))
    });
}

/// A read, applied if it is still the window's: for the range on screen, and taken after the read
/// whose events are in hand. Says whether it was.
///
/// A read for a range the window has since left is dropped: the read for where the window is now
/// was asked for when it moved. Of two reads for the same range, the one taken later wins,
/// whatever order they land in.
pub(crate) fn apply(m: &mut Model, listing: Listing) -> bool {
    if listing.wanted != m.visible_range() || listing.seq < m.reads.shown {
        return false;
    }
    m.reads.shown = listing.seq;
    match listing.read {
        Read::Unmoved => {}
        Read::Listed { events, revision } => {
            m.events = events;
            m.range = Some(listing.wanted);
            m.revision = revision;
        }
        Read::Failed(e) => {
            tracing::warn!(error = %e, "could not read the calendar");
            m.events.clear();
            m.range = None;
            m.revision = None;
        }
    }
    true
}

/// Everything on screen is drawn again from the model, and the clock read with it: anything that
/// changes the month, the selected day, the view, or what is stored ends here.
pub(crate) fn redraw(m: &mut Model, fx: &mut Fx) {
    m.now = m.machine.clock.now();
    m.utc_offset = m.machine.clock.utc_offset_seconds();
    fx.changed(PART_CALENDAR);
    fx.changed(PART_CLOCK);
}

/// Put the window on `date`: its month, that day picked. What an action just changed is then what
/// the window shows, and what the answer's first line says. `add_event` used to leave the window
/// on whatever day was picked, so a mind adding to the 30th read "3 things on day 25", saw no sign
/// of its event, and added it again (yantrik-mind R2, T3 rep 3), and the duplicate became a false
/// claim in the next task. A string that does not start with a date leaves the window where it is.
pub(crate) fn show_date(m: &mut Model, date: &str) {
    if let Some((year, month, day)) = events::ymd(date) {
        m.year = year;
        m.month = month;
        m.selected_day = day;
    }
}
