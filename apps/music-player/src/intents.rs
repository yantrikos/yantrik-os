//! What the Mind may ask Music by name: the same eight actions, with the same names,
//! descriptions, parameters and grades as before, in the same order, each the command a button
//! runs where there is a button for it.
//!
//! # Settling
//!
//! Every command to the player and every scan runs in a job now, so each intent is answered when
//! its own job lands (`Settle::Explicit`), in the command that carries it (`answers.rs`): with
//! what mpv reports, asked by that job, or with what that scan found. A check that needs only
//! the model (an `index` below 1, a `seek` past the end) answers at once, and is said on the
//! notice line too, as every refusal always was.

use std::time::Duration;

use serde_json::Value;
use yantrik_app::control::{Action, Param};
use yantrik_app::intents::{Intent, Intents, Settle, MAX_SETTLE_WAIT};

use crate::command::Command;
use crate::model::Model;
use crate::Music;

/// How long a call to the player may take to answer: 14 s.
///
/// The ceiling is the Mind's: its `app_action` client gives up after 15 s, so settling within 14
/// means the app answers first, with mpv's report or its own refusal. The floor is the work: a
/// player started for the call (up to 5 s), the commands ahead of it, and the 4 s wait for mpv
/// to say the file is playing.
pub const WITHIN: Duration = Duration::from_secs(14);

pub fn table(table: &mut Intents<Music>) {
    table.add(answered(
        Intent::new(
            act(
                "play",
                "Start or resume playback. With `index`, play that track of the library list (1 is \
                 the first) and make the list the queue; without it, resume what is paused, or play \
                 the queue's current track, or the library from the top. Answers with what mpv \
                 reports once it is playing.",
            )
            .arg(
                Param::integer("index")
                    .describe("A track number in the library list, from 1, as `describe.library` numbers them.")
                    .optional(),
            ),
            |a| Ok(Command::Play(a.raw().get("index").and_then(Value::as_i64))),
        ),
        WITHIN,
    ));
    table.add(answered(
        Intent::new(
            act("pause", "Pause what is playing. Answers with mpv's own report of pause and position."),
            |_| Ok(Command::Pause),
        ),
        WITHIN,
    ));
    table.add(answered(
        Intent::new(act("next", "Play the next track in the queue. Refused at the end of the queue."), |_| {
            Ok(Command::Next)
        }),
        WITHIN,
    ));
    table.add(answered(
        Intent::new(
            act(
                "previous",
                "Go back: to the start of the track if it has played more than three seconds, \
                 otherwise to the track before it in the queue.",
            ),
            |_| Ok(Command::Previous),
        ),
        WITHIN,
    ));
    table.add(answered(
        Intent::new(
            act("seek", "Move to a point in the playing track. Answers with the position mpv reports after the move.")
                .arg(Param::number("seconds").describe("Seconds from the start of the track.")),
            |a| Ok(Command::Seek(a.number("seconds")?)),
        ),
        WITHIN,
    ));
    table.add(answered(
        Intent::new(
            act("set_volume", "Set the volume, 0 to 100. Answers with the volume mpv reports.")
                .arg(Param::number("percent").describe("0 is silent, 100 is full volume.")),
            |a| Ok(Command::SetVolume(a.number("percent")?)),
        ),
        WITHIN,
    ));
    table.add(answered(
        Intent::new(
            act(
                "play_file",
                "Play one sound file now, by its path, as the whole queue. An agent may name only \
                 files in the person's home outside its protected places.",
            )
            .arg(Param::text("path").describe("The file's path: absolute, or starting with ~/. mp3, flac, ogg, opus, wav or m4a.")),
            // Checked in the update and its job, so a refusal is said on the notice line too.
            |a| Ok(Command::PlayFile(a.raw().get("path").and_then(Value::as_str).unwrap_or_default().to_string())),
        ),
        WITHIN,
    ));
    table.add(answered(
        Intent::new(
            act(
                "rescan",
                "Scan the library folders again and answer with what was found: how many tracks in \
                 each folder, and any folder that could not be read.",
            )
            .expected_seconds(30),
            |_| Ok(Command::Rescan),
        ),
        // A scan of a large library takes a while; the framework's ceiling is the deadline.
        MAX_SETTLE_WAIT,
    ));
}

fn act(name: &str, sentence: &str) -> Action {
    Action::new(name, sentence).risk("standard")
}

/// An intent answered when its own work has landed, within `within`, with what that work found.
fn answered(intent: Intent<Music>, within: Duration) -> Intent<Music> {
    intent.settles(Settle::Explicit { within }).reply(answer)
}

/// What an intent answers: what `answers.rs` wrote in the command that settled it.
fn answer(m: &Model) -> Value {
    m.answer.clone()
}
