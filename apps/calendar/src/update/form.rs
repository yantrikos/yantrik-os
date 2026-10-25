//! Putting an event on the calendar: the form, its templates, and `add_event`.
//!
//! The form's Save and the `add_event` action share one path — the checks in [`plan`], the
//! create in a job, the store read back after it — so neither can report an outcome it did not
//! get. The action used to answer `{"added": ...}` the moment it had handed the title to the
//! window, and the window's own save dropped the service's error on the floor — which is how a
//! calendar that was storing nothing told every caller it had.

use serde_json::json;
use yantrik_app::Refused;
use yantrik_ipc_contracts::calendar::MAX_REMINDER_MINUTES;

use super::{read_back, show_date, Fx};
use crate::answers;
use crate::command::{Command, Door, Saved, Work};
use crate::model::{Model, DEFAULT_EVENT_MINUTES, NOTHING, PART_FORM, PART_FORM_FIELDS, PART_NOTICE};
use crate::views;
use crate::work::{self, NewEvent};

pub fn update(m: &mut Model, command: Command, fx: &mut Fx) -> Result<(), Refused> {
    match command {
        Command::OpenForm => {
            // A blank form is an hour long; a template says otherwise and is honoured once.
            open(m, fx, String::new(), DEFAULT_EVENT_MINUTES);
        }
        Command::UseTemplate { title, minutes } => {
            // The four templates belong to the screen; pressing one hands over the two things the
            // app needs, the title to start from and how long the thing runs. That is the whole
            // of what "use a template" can honestly mean while the form has a title, a date, a
            // time and notes and nothing else: the title is filled in, and the minutes wait in
            // the model until Save.
            open(m, fx, title, if minutes > 0 { minutes } else { DEFAULT_EVENT_MINUTES });
            m.notice.say("");
            fx.changed(PART_NOTICE);
        }
        Command::CancelForm => {
            m.form.open = false;
            fx.changed(PART_FORM);
        }
        Command::SaveForm { title, date, time, notes } => save_form(m, fx, title, date, time, notes)?,
        Command::Add(ask) => {
            // Who is asking, verified: the record a later `delete_own_event` is checked against.
            let creator = m.machine.who.identify(&ask.caller);
            let draft = Draft {
                title: &ask.title,
                date: &ask.date,
                time: &ask.time,
                notes: &ask.notes,
                duration_min: ask.duration_min,
                all_day: ask.all_day,
                reminder_minutes: ask.reminder_minutes,
            };
            match plan(m, &draft, creator) {
                // A failure is put on screen as well as returned: when a mind tries to put
                // something on the calendar and cannot, the person watching the window is owed
                // the reason too.
                Err(e) => {
                    m.notice.say(format!("Could not save “{}”: {e}", ask.title));
                    fx.changed(PART_NOTICE);
                    fx.settle(Err(Refused::new("not_saved", e)));
                }
                Ok(new) => {
                    create(m, fx, Door::Surface, new, &ask.date, &ask.time);
                    fx.changed(NOTHING);
                }
            }
        }
        Command::Saved(saved) => landed(m, fx, *saved),
        other => unreachable!("not a form command: {other:?}"),
    }
    Ok(())
}

/// The form, opened on the selected day at nine, with `title` and the length a Save will give it.
fn open(m: &mut Model, fx: &mut Fx, title: String, minutes: i32) {
    m.form.generation += 1;
    m.form.duration_min = minutes;
    m.form.date = m.selected_date().format("%Y-%m-%d").to_string();
    m.form.time = "09:00".into();
    m.form.title = title;
    m.form.notes = String::new();
    m.form.open = true;
    fx.changed(PART_FORM);
    fx.changed(PART_FORM_FIELDS);
}

/// The form's Save. The form has no duration field, no all-day switch and no reminder field, so
/// it says nothing about any of them: the duration comes from the model, where a template left
/// it; an event typed into this form is a timed one; and its reminder is the default the store
/// applies. No creator is recorded: there is no caller on the socket, only the person.
fn save_form(m: &mut Model, fx: &mut Fx, title: String, date: String, time: String, notes: String) -> Result<(), Refused> {
    if m.form.saving {
        return Err(Refused::new("busy", "The event is still being saved."));
    }
    let draft = Draft {
        title: &title,
        date: &date,
        time: &time,
        notes: &notes,
        duration_min: None,
        all_day: false,
        reminder_minutes: None,
    };
    match plan(m, &draft, None) {
        // The form stays open holding what was typed. Closing it on a failed save threw the
        // event away twice: once from the store, once from the screen.
        Err(e) => {
            m.notice.say(format!("Could not save “{}”: {e}", title.trim()));
            fx.changed(PART_NOTICE);
        }
        Ok(new) => {
            m.form.saving = true;
            create(m, fx, Door::Form, new, &date, &time);
            fx.changed(NOTHING);
        }
    }
    Ok(())
}

/// An event as it was asked for, before it is checked: from the form, or from `add_event`.
struct Draft<'a> {
    title: &'a str,
    date: &'a str,
    time: &'a str,
    notes: &'a str,
    duration_min: Option<i32>,
    all_day: bool,
    reminder_minutes: Option<u32>,
}

/// The checks before anything is stored, and the two timestamps an event gets.
///
/// How long it runs is the caller's if it said, and the form's otherwise: the form has no
/// duration field, and a template pressed a moment ago has already said 15 or 30 or 120. The
/// arithmetic that turns a date, a clock time and a length into two ISO timestamps is in `views`
/// so it can be tested -- it is where 23:30 plus an hour used to become "24:30", which is not a
/// time.
fn plan(m: &Model, draft: &Draft<'_>, creator: Option<String>) -> Result<NewEvent, String> {
    let Draft { title, date, time, notes, duration_min, all_day, reminder_minutes } = *draft;
    let title = title.trim();
    if title.is_empty() {
        return Err("an event needs a title".into());
    }
    if let Some(minutes) = reminder_minutes {
        if minutes > MAX_REMINDER_MINUTES {
            return Err(format!(
                "`reminder_minutes` cannot be longer than {MAX_REMINDER_MINUTES}, and was {minutes}"
            ));
        }
    }
    let (start, end) = if all_day {
        // A whole day has no clock, so the time is not consulted and not silently applied.
        views::all_day_bounds(date).ok_or_else(|| format!("`{date}` is not a date"))?
    } else {
        let duration = duration_min.unwrap_or(m.form.duration_min);
        if duration < 0 {
            return Err(format!("`duration_min` cannot be negative, and was {duration}"));
        }
        views::start_and_end(date, time, duration)
            .ok_or_else(|| format!("`{date} {time}` is not a date and a time"))?
    };
    Ok(NewEvent {
        title: title.to_string(),
        start,
        end,
        notes: notes.to_string(),
        all_day,
        reminder_minutes,
        creator,
    })
}

/// Store the event in a job. Its result is the job's whole answer: the store is read back in a job
/// of its own once the create has landed (`read_back`), so a read that fails cannot make a stored
/// event look unsaved and invite saving it twice.
fn create(m: &mut Model, fx: &mut Fx, door: Door, new: NewEvent, date: &str, time: &str) {
    let api = m.machine.api.clone();
    let (date, time, form) = (date.to_string(), time.to_string(), m.form.generation);
    m.writes += 1;
    fx.spawn_or(Command::WorkFailed(Work::Save(door)), move |_| {
        let result = work::create(api.as_ref(), &new);
        Command::Saved(Box::new(Saved { door, title: new.title, date, time, all_day: new.all_day, result, form }))
    });
}

/// A create landed: the window shows what the store holds, and the caller is answered with the id
/// it was stored under, so "added" cannot be a guess about what the window did next.
fn landed(m: &mut Model, fx: &mut Fx, saved: Saved) {
    if saved.door == Door::Form {
        m.form.saving = false;
    }
    let (id, reminder) = match saved.result {
        Ok(stored) => stored,
        Err(e) => {
            m.notice.say(format!("Could not save “{}”: {e}", saved.title));
            fx.changed(PART_NOTICE);
            if saved.door == Door::Surface {
                fx.settle(Err(Refused::new("not_saved", e)));
            }
            return;
        }
    };
    m.notice.say("");
    fx.changed(PART_NOTICE);
    match saved.door {
        // The form that was saved closes, and its length goes back to an hour; a form the person
        // has opened since, perhaps from a template, is theirs and is left alone.
        Door::Form => {
            if m.form.generation == saved.form {
                m.form.duration_min = DEFAULT_EVENT_MINUTES;
                m.form.open = false;
                fx.changed(PART_FORM);
            }
        }
        Door::Surface => {
            m.form.duration_min = DEFAULT_EVENT_MINUTES;
            show_date(m, &saved.date);
            let on = if saved.all_day { saved.date.clone() } else { format!("{} {}", saved.date, saved.time) };
            let mut answer = json!({ "added": saved.title, "on": on, "id": id, "all_day": saved.all_day });
            if !saved.all_day {
                // The lead the store recorded — what the caller asked for, or the default when it
                // did not say. Not reported for an all-day event, which is never announced: a
                // number there would describe a reminder that will not happen.
                answer["reminder_minutes"] = json!(reminder);
            }
            answers::settle(m, fx, answer);
        }
    }
    read_back(m, fx);
}
