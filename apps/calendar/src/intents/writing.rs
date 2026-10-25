//! The intents that change what is stored: `add_event`, the two delete doors and the two update
//! doors, with the grades their authors chose and the reasons they wrote beside them.
//!
//! Who is calling is read here, inside the dispatch, where the kernel's account of the call and
//! its agent token are in scope (`who::Caller::now`), and travels with the command; nothing the
//! request itself says is consulted.

use yantrik_app::control::{Action, Param};
use yantrik_app::intents::{Intent, Intents};

use super::{explicit, refused};
use crate::asks::{AddAsk, NameAsk, UpdateAsk};
use crate::command::Command;
use crate::model::Model;
use crate::who::Caller;
use crate::Calendar;

pub fn table(table: &mut Intents<Calendar>) {
    table.add(explicit(
        Intent::new(
            Action::new("add_event", "Put something on the calendar")
                .arg(Param::text("title"))
                .arg(Param::text("date").describe("YYYY-MM-DD"))
                .arg(Param::text("time")
                    .describe("HH:MM, 24-hour; needed unless `all_day` is set")
                    .optional())
                .arg(Param::text("notes").optional())
                // The form has no duration field and the template path already carries minutes,
                // so the one caller that could say how long a thing runs was the one that could
                // not: a mind asking for a fifteen-minute call got an hour and was not told.
                .arg(Param::integer("duration_min")
                    .describe("How long it runs, in minutes; an hour when not given")
                    .optional())
                // The contract carries `is_all_day` and nothing on this surface could set it, so
                // a whole-day event asked for here was stored as a one-hour appointment at
                // whatever time happened to be passed.
                .arg(Param::flag("all_day")
                    .describe("A whole day rather than a time; `time` and `duration_min` are \
                               not used with it")
                    .optional())
                // The reminder is a fact about the event, stored with it and announced by the
                // notifications service whether or not this window is ever opened again (#78).
                .arg(Param::integer("reminder_minutes")
                    .describe("How many minutes before it starts to announce it; ten when not \
                               given. All-day events are not announced")
                    .optional()),
            |a| Ok(Command::Add(Box::new(AddAsk::parse(a.raw(), Caller::now()).map_err(refused)?))),
        )
        .reply(answer),
    ));
    table.add(explicit(
        Intent::new(
            // Graded `sensitive`, and the reason is that there is no trash. A delete here removes
            // the file the event lives in; nothing on this machine keeps a copy, so an appointment
            // a person or a mind put on the calendar is gone and its time with it. That is a
            // different thing from `add_event`, which is `standard` because it is undone by this
            // action, and from `update_event`, which moves something that still exists. It sits
            // below `dangerous` because it destroys one named thing the caller asked for by name,
            // not a range and not a directory — the range delete Google sync used to do, which
            // took every local event in the window with it, would have been the other grade.
            Action::new("delete_event", "Take an event off the calendar. It is not recoverable")
                .risk("sensitive")
                .arg(Param::text("id")
                    .describe("The id the store gave the event — `add_event` answers with it and \
                               `describe` lists it for every event it shows")
                    .optional())
                .arg(Param::text("title")
                    .describe("The event's exact title, given with `date`, when the id is not known")
                    .optional())
                .arg(Param::text("date")
                    .describe("YYYY-MM-DD, given with `title`")
                    .optional()),
            |a| delete(a.raw(), false),
        )
        .reply(answer),
    ));
    table.add(explicit(
        Intent::new(
            // Graded `standard`, and the only events it can reach are the ones the store records
            // as created by this very caller: `who` reads the kernel's account of the call — the
            // peer walk of #221, or the agent an agent token belongs to — at creation, the service
            // keeps that in the event's own file, and the rule in `ownership` compares the two
            // when a delete arrives. Nothing the request itself says is consulted on either side.
            // For the caller that made it, this is the inverse of its own `add_event`, which is
            // `standard` too: an unattended harness can put an event on and take it off again
            // without a person being asked, which is the whole of issue #201. Everything else — a
            // person's event, another caller's, an event stored before the record existed — is
            // refused here and stays with `delete_event` above: same grade, same card, exactly as
            // it was.
            Action::new(
                "delete_own_event",
                "Take an event this caller created itself off the calendar. Anything else is \
                 `delete_event`, which asks a person first",
            )
            .arg(Param::text("id")
                .describe("The id the store gave the event — `add_event` answers with it and \
                           `describe` lists it, marked with whether this caller may delete it \
                           unasked")
                .optional())
            .arg(Param::text("title")
                .describe("The event's exact title, given with `date`, when the id is not known")
                .optional())
            .arg(Param::text("date")
                .describe("YYYY-MM-DD, given with `title`")
                .optional()),
            |a| delete(a.raw(), true),
        )
        .reply(answer),
    ));
    table.add(explicit(
        Intent::new(
            // Graded `sensitive` (#332), by `docs/sdk/grades.md`'s rule to grade an action by the
            // worst its arguments allow: the `id` names any event in the store — the person's
            // own, a Google-synced one, another caller's — so this door can rewrite an
            // appointment that is not the caller's, and the person sees a card. Nothing is
            // destroyed by a move — the event stays under the same id and the previous time is
            // the only thing lost — so it sits below the delete's grade in kind, and editing an
            // event the caller created itself stays `standard`, at `update_own_event` below: the
            // #201 split the two delete doors already have.
            update_door_args(
                Action::new("update_event", "Move or rename any event that is already on the calendar")
                    .risk("sensitive"),
            ),
            |a| update(a.raw(), false),
        )
        .reply(answer),
    ));
    table.add(explicit(
        Intent::new(
            // Graded `standard`, and the only events it can reach are the ones the store records
            // as created by this very caller — the rule in `ownership`, the same comparison
            // `delete_own_event` keys off, with both sides established by the machine and nothing
            // the request says consulted (#332, on #201's door). An unattended harness can move
            // the events it made without a person being asked; everything else is refused here
            // in a sentence that points at `update_event` above, where the person sees the card.
            update_door_args(Action::new(
                "update_own_event",
                "Move or rename an event this caller created itself. Anything else is \
                 `update_event`, which asks a person first",
            )),
            |a| update(a.raw(), true),
        )
        .reply(answer),
    ));
}

/// The arguments both update doors declare, written down once so the two cannot drift.
fn update_door_args(action: Action) -> Action {
    action
        .arg(Param::text("id").describe("The id the store gave the event"))
        .arg(Param::text("title").describe("A new title").optional())
        .arg(Param::text("date").describe("Move it to this day, YYYY-MM-DD").optional())
        .arg(Param::text("time").describe("Move it to this time, HH:MM").optional())
        .arg(Param::integer("duration_min")
            .describe("How long it runs, in minutes; unchanged when not given")
            .optional())
        .arg(Param::text("notes").optional())
        .arg(Param::integer("reminder_minutes")
            .describe("How many minutes before it starts to announce it; unchanged \
                       when not given")
            .optional())
}

fn delete(args: &serde_json::Value, own: bool) -> Result<Command, yantrik_app::Refused> {
    let ask = NameAsk::parse(args).map_err(refused)?;
    Ok(Command::Delete { ask, own, caller: Caller::now() })
}

fn update(args: &serde_json::Value, own: bool) -> Result<Command, yantrik_app::Refused> {
    let ask = UpdateAsk::parse(args).map_err(refused)?;
    Ok(Command::Update { ask: Box::new(ask), own, caller: Caller::now() })
}

/// What a write answers: the store read back, as the command that landed set it.
fn answer(m: &Model) -> serde_json::Value {
    m.answer.clone()
}
