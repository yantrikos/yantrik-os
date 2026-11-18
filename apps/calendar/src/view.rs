//! The window: its callbacks bound to commands, and the model drawn for the parts that changed.
//! The only code that touches Slint.

use slint::{ModelRc, SharedString, VecModel};
use yantrik_app::app::Changed;
use yantrik_app::cx::Cx;
use yantrik_app::rail::{self, Proposal, RailView as _};

use crate::command::Command;
use crate::events::{self, CalEvent};
use crate::model::*;
use crate::views::{self, WeekStart};
use crate::{AgentProposal, Calendar, CalendarApp, CalendarDay, CalendarEvent, CalendarTimeEvent};

/// The colours an event is drawn in, by its position in what the store returned.
const PALETTE: [(u8, u8, u8); 5] = [
    (0x4E, 0x79, 0xA7),
    (0xF2, 0x8E, 0x2C),
    (0xE1, 0x57, 0x59),
    (0x76, 0xB7, 0xB2),
    (0x59, 0xA1, 0x4F),
];

fn palette(index: usize) -> slint::Color {
    let (r, g, b) = PALETTE[index % PALETTE.len()];
    slint::Color::from_rgb_u8(r, g, b)
}

pub fn wire(cx: &Cx<Calendar>) {
    let ui = cx.ui();
    ui.on_prev_month(cx.cmd(|| Command::PrevMonth));
    ui.on_next_month(cx.cmd(|| Command::NextMonth));
    ui.on_day_clicked(cx.cmd1(Command::DayClicked));
    // The screen sets `view-mode` itself and then says so; the model follows.
    ui.on_switch_view(cx.cmd1(Command::SwitchView));
    ui.on_today_pressed(cx.cmd(|| Command::Today));
    ui.on_add_event(cx.cmd(|| Command::OpenForm));
    ui.on_cal_use_template(cx.cmd2(|title: SharedString, minutes: i32| Command::UseTemplate {
        title: title.to_string(),
        minutes,
    }));
    let save = cx.clone();
    ui.on_save_event(move |title, date, time, notes| {
        save.run(Command::SaveForm {
            title: title.to_string(),
            date: date.to_string(),
            time: time.to_string(),
            notes: notes.to_string(),
        })
    });
    ui.on_cancel_event_form(cx.cmd(|| Command::CancelForm));
    ui.on_delete_event(cx.cmd1(Command::DeleteRow));
    ui.on_dismiss_notice(cx.cmd(|| Command::DismissNotice));
    ui.on_open_notice_event(cx.cmd(|| Command::OpenNoticeEvent));
    ui.on_ai_explain_pressed(cx.cmd(|| Command::Explain));
    ui.on_ai_dismiss(cx.cmd(|| Command::DismissAi));
    drop(ui);
    // The rail's suggestions and the card's buttons. No `agent-context-activated` handler: a row in
    // the rail is an event on the day already on screen, so there is nowhere for a click to go.
    rail::wire(cx);
    // `describe` hands a read it had to make back through this (`follow.rs`).
    cx.run(Command::Attach(Inbox(cx.post())));
}

/// Whether a command named `part`. The parts that write the window's in-out properties are drawn
/// only then, never on a redraw of everything (see `model::PART_FORM_FIELDS`).
fn named(changed: &Changed, part: &str) -> bool {
    matches!(changed, Changed::Parts(parts) if parts.contains(&part))
}

pub fn render(ui: &CalendarApp, m: &Model, changed: &Changed) {
    if changed.has(PART_CALENDAR) {
        calendar(ui, m);
    }
    if changed.has(PART_CALENDAR) || changed.has(PART_CLOCK) {
        ui.set_current_hour(chrono::Timelike::hour(&m.now) as i32);
        ui.set_current_minute(chrono::Timelike::minute(&m.now) as i32);
        // The offset is all `chrono::Local` carries -- there is no zone database in this
        // dependency set -- so the strip says the offset and no more.
        ui.set_cal_timezone_display(views::timezone_label(m.utc_offset).into());
    }
    if changed.has(PART_NOTICE) {
        ui.set_notice(m.notice.text.as_str().into());
        ui.set_notice_event(m.notice.event.as_str().into());
    }
    if changed.has(PART_FORM) {
        ui.set_show_event_form(m.form.open);
    }
    if named(changed, PART_FORM_FIELDS) {
        ui.set_event_date(m.form.date.as_str().into());
        ui.set_event_time(m.form.time.as_str().into());
        ui.set_event_title(m.form.title.as_str().into());
        ui.set_event_notes(m.form.notes.as_str().into());
    }
    if changed.has(PART_AI) || changed.has(rail::PART) {
        ui.set_ai_is_working(m.ai.working);
        ui.render_rail(&m.rail);
        // The header's own card, when the rail could not ask and has none of its own up.
        if m.rail.proposal().is_none() {
            if let Some(card) = &m.ai.card {
                ui.set_proposal(proposal(card));
                ui.set_proposal_working(m.ai.asking);
            }
        }
    }
}

/// Everything drawn from where the window is and the events in hand.
fn calendar(ui: &CalendarApp, m: &Model) {
    // The day the window last saw as today, not a fresh read of the clock: the marker and the
    // window's idea of today are one value, so they cannot disagree about which day it is.
    let today_day = events::today_in(m.year, m.month, m.today.date());
    ui.set_month_title(m.month_title().into());
    ui.set_days(rows(month_grid(m.year, m.month, &m.events, today_day, m.week_start)));
    ui.set_week_starts_monday(m.week_start == WeekStart::Monday);
    ui.set_selected_day(m.selected_day);
    ui.set_view_mode(m.view.index());
    ui.set_events_today(rows(agenda(&m.events, m.year, m.month, m.selected_day)));

    // The same events, placed on the hour grids. `views` decides where each one goes; this side
    // only turns the answer into Slint rows.
    let source = events::source_events(&m.events);
    let selected = m.selected_date();
    let week = views::week_view(&source, selected, m.week_start);
    let labels: Vec<SharedString> = week.labels.iter().map(|l| SharedString::from(l.as_str())).collect();
    ui.set_week_day_labels(rows(labels));
    ui.set_week_events(rows(time_events(&week.events)));
    let day = views::day_view(&source, selected);
    ui.set_day_events(rows(time_events(&day.events)));
    ui.set_day_view_title(day.title.into());
}

fn rows<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(rows))
}

/// The month grid's cells, as the screen draws them.
pub fn month_grid(
    year: i32,
    month: u32,
    events: &[CalEvent],
    today_day: Option<u32>,
    week_start: WeekStart,
) -> Vec<CalendarDay> {
    events::month_cells(year, month, events, today_day, week_start)
        .into_iter()
        .map(|c| CalendarDay {
            day_number: c.day as i32,
            is_today: c.is_today,
            is_selected: false,
            is_current_month: c.is_current_month,
            has_events: c.event_count > 0,
            event_count: c.event_count,
        })
        .collect()
}

/// The agenda in the sidebar.
///
/// The row's `id` is its position in this list, because that is what `delete-event` hands back
/// and what `Command::DeleteRow` indexes with. It was the literal 0 on every row, so the trash
/// icon on any row of a day deleted the first one. The store's own id is not on the row because
/// the Slint struct has no field for one; the command resolves the index against
/// `events_on_day`, which is the same list these rows were built from.
fn agenda(events: &[CalEvent], year: i32, month: u32, day: i32) -> Vec<CalendarEvent> {
    let prefix = format!("{:04}-{:02}-{:02}", year, month, day);
    events::events_on_day(events, year, month, day)
        .into_iter()
        .enumerate()
        .map(|(row, e)| CalendarEvent {
            id: row as i32,
            title: e.title.as_str().into(),
            date_text: prefix.as_str().into(),
            time_text: events::time_text(e).into(),
            color: palette(e.color_index),
            is_all_day: e.is_all_day,
        })
        .collect()
}

/// Derived blocks, turned into the rows the two time grids draw.
///
/// The id is dropped on the way: `CalendarTimeEvent` has no field for one, and a grid block is
/// drawn, not named. `describe` reports the ids from the same derivation.
fn time_events(blocks: &[views::TimeEvent]) -> Vec<CalendarTimeEvent> {
    blocks
        .iter()
        .map(|b| CalendarTimeEvent {
            title: b.title.as_str().into(),
            start_hour: b.start_hour,
            start_min: b.start_min,
            duration_min: b.duration_min,
            day_index: b.day_index,
            color: palette(b.color_index),
        })
        .collect()
}

fn proposal(p: &Proposal) -> AgentProposal {
    AgentProposal {
        title: p.title.as_str().into(),
        body: p.body.as_str().into(),
        source: p.source.as_str().into(),
        impact: p.impact.as_str().into(),
        destructive: p.destructive,
        verb: p.verb.as_str().into(),
    }
}
