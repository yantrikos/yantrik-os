//! What a mind may ask yPresent by name: the actions worded, argued and graded as they always
//! were, each the command its window action runs, in the order yPresent has always published them
//! (tests/parity.rs pins it).
//!
//! One name changes: `present` is the framework's own action on every surface (the desktop's
//! hand-over, which shows the window), so an app may not declare it. The full-screen presenter
//! view is `presenter_view` now, with its description and its `mode` word as they were.
//!
//! Every answer is built from that call's own work (update/surface.rs) and settled when it has
//! landed (`Settle::Explicit`), so nothing the person does meanwhile, and no other call, becomes
//! this call's answer.

use std::time::Duration;

use serde_json::Value;
use yantrik_app::control::{Action, Param};
use yantrik_app::intents::{Args, Intent, Intents, Settle};
use yantrik_app::Refused;

use crate::caller::Caller;
use crate::command::{Act, Command, Index};
use crate::model::Model;
use crate::YPresent;

/// How long a call may wait for its file work. The old surface answered inside a 3 s round trip,
/// and a deck is at most 4 MiB.
const ANSWER_WITHIN: Duration = Duration::from_secs(3);

/// A text argument as given. The dispatch has checked it is there; whitespace in it is the
/// caller's, since a slide's text is theirs to space.
fn text(a: &Args<'_>, name: &str) -> Option<String> {
    a.raw().get(name).and_then(Value::as_str).map(str::to_string)
}

/// A slide index out of the arguments.
///
/// A model writing JSON by hand sends `2` and `2.0` about equally often, so both are read rather
/// than one of them being refused as "not a whole number" when it plainly is one.
fn index_arg(a: &Args<'_>, name: &str) -> Index {
    let value = a.raw().get(name);
    let raw = value
        .and_then(Value::as_i64)
        .or_else(|| value.and_then(Value::as_f64).filter(|f| f.fract() == 0.0).map(|f| f as i64))
        .ok_or_else(|| format!("`{name}` must be a whole number"))?;
    if raw < 0 {
        return Err(format!("`{name}` is {raw}; slides are numbered from 0"));
    }
    Ok(raw as usize)
}

/// Declares an intent: its arguments read into an [`Act`], raised with who is calling, answered
/// once its work has landed.
fn add(table: &mut Intents<YPresent>, action: Action, parse: impl Fn(&Args<'_>) -> Act + 'static) {
    table.add(
        Intent::new(action, move |a| Ok::<Command, Refused>(Command::Surface { act: parse(a), caller: Caller::now() }))
            .settles(Settle::Explicit { within: ANSWER_WITHIN })
            .reply(|m: &Model| m.answer.clone()),
    );
}

pub fn table(table: &mut Intents<YPresent>) {
    add(
        table,
        Action::new("open", "Open a deck file in this window").arg(Param::text("path").describe("Path to a .ydeck file")),
        |a| Act::Open { path: text(a, "path").unwrap_or_default() },
    );

    add(table, Action::new("show", "Bring the window forward"), |_| Act::Show);

    add(table, Action::new("save", "Write the deck to its file"), |_| Act::Save);

    add(
        table,
        Action::new("save_as", "Write the deck to a path of your choosing")
            .arg(Param::text("path").describe("Where to write the .ydeck file"))
            .arg(Param::flag("overwrite").optional().describe("Replace a file that is already at that path")),
        |a| Act::SaveAs {
            path: text(a, "path").unwrap_or_default(),
            overwrite: a.raw().get("overwrite").and_then(Value::as_bool).unwrap_or(false),
        },
    );

    add(
        table,
        Action::new("new_deck", "Start an empty deck").arg(Param::text("title").describe("What to call it").optional()),
        |a| Act::NewDeck { title: text(a, "title") },
    );

    add(
        table,
        Action::new("add_slide", "Add a slide and select it")
            .arg(Param::text("title").optional())
            .arg(Param::text("body").optional())
            .arg(Param::text("notes").optional())
            .arg(Param::number("after").describe("Insert after this slide index").optional()),
        |a| Act::AddSlide {
            title: text(a, "title"),
            body: text(a, "body"),
            notes: text(a, "notes"),
            after: a.raw().get("after").map(|_| index_arg(a, "after")),
        },
    );

    add(
        table,
        Action::new("set_slide", "Change the text of one slide")
            .arg(Param::number("index"))
            .arg(Param::text("title").optional())
            .arg(Param::text("body").optional())
            .arg(Param::text("notes").optional()),
        |a| Act::SetSlide {
            index: index_arg(a, "index"),
            title: text(a, "title"),
            body: text(a, "body"),
            notes: text(a, "notes"),
        },
    );

    // Graded sensitive because it destroys writing. Nothing else on this surface does: `new_deck`
    // and `open` save the deck they replace before replacing it, `move_slide` reorders, and
    // `set_slide` is a change whose text the caller supplied. This is the one action whose whole
    // effect is that a slide someone wrote is gone, out of a deck that may never have been saved.
    add(
        table,
        Action::new("delete_slide", "Remove a slide from the deck").risk("sensitive").arg(Param::number("index")),
        |a| Act::DeleteSlide { index: index_arg(a, "index") },
    );

    add(
        table,
        Action::new("move_slide", "Move a slide to another position")
            .arg(Param::number("from"))
            .arg(Param::number("to")),
        |a| Act::MoveSlide { from: index_arg(a, "from"), to: index_arg(a, "to") },
    );

    add(
        table,
        Action::new("go_to", "Select a slide by index").arg(Param::number("index")),
        |a| Act::GoTo { index: index_arg(a, "index") },
    );

    add(table, Action::new("next", "The next slide"), |_| Act::Next);

    add(table, Action::new("previous", "The previous slide"), |_| Act::Previous);

    add(
        table,
        Action::new("presenter_view", "Enter or leave the full-screen presenter view").arg(
            Param::text("mode").describe("on | fullscreen | true | start | off | windowed | exit | false | stop | toggle"),
        ),
        |a| Act::PresenterView { mode: text(a, "mode").unwrap_or_default() },
    );

    add(
        table,
        Action::new("export_markdown", "Write the deck out as Markdown")
            .arg(Param::text("path").describe("Where to write it").optional()),
        |a| Act::ExportMarkdown { path: text(a, "path") },
    );
}
