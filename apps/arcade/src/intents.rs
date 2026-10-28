//! What the Mind may ask Arcade by name: the same nine actions as before, with the same names,
//! descriptions, parameters and grades, in the same order, each the request a button makes where
//! there is a button for it.
//!
//! `new_character`, `new_game`, `update_character`, `update_game` and `build` are plain file writes
//! (standard; an update keeps the spec it replaces one step back, and a build is milliseconds to
//! redo). `play`, `verify` and `screenshot` reach a browser and defer to workers. `delete` goes to
//! the Trash and is recoverable from Files, which keeps it standard rather than dangerous.
//!
//! # Settling
//!
//! Every action is answered when its own command has landed on the library's lane
//! (`Settle::Explicit`), in the command that carries it (`answers.rs`). The three browser actions
//! answer that the job has started, as they always did, and the verdict lands in `describe`.

use std::time::Duration;

use serde_json::Value;
use yantrik_app::control::{Action, Param};
use yantrik_app::intents::{Args, Intent, Intents, Settle};
use yantrik_app::Refused;

use crate::command::{Command, Request, Verb};
use crate::model::Model;
use crate::spec;
use crate::Arcade;

/// How long a command may take to land before the Mind is told it did not.
pub const WITHIN: Duration = Duration::from_secs(14);

/// The request an intent's arguments make, or the sentence the surface has always refused with.
fn request(verb: Verb, a: &Args<'_>) -> Result<Command, Refused> {
    Request::from_args(verb, a.raw())
        .map(Command::Run)
        .map_err(|sentence| Refused::new("missing_argument", sentence))
}

/// The declarations only, kept apart from the commands so a test can read exactly what a caller is
/// shown.
pub fn table(table: &mut Intents<Arcade>) {
    for (verb, action) in actions() {
        table.add(
            Intent::new(action, move |a| request(verb, a))
                .settles(Settle::Explicit { within: WITHIN })
                .reply(answer),
        );
    }
}

/// What an intent answers: what `update` wrote in the command that settled it.
fn answer(m: &Model) -> Value {
    m.answer.clone()
}

fn actions() -> Vec<(Verb, Action)> {
    vec![
        (
            Verb::NewCharacter,
            Action::new("new_character", "Save a character spec into the library").arg(Param::text("spec").describe(&format!(
                "The character JSON. Required: name, archetype, proportions (head_body, limb_length, width). Optional: ears, tail, palette (base/belly/accent/nose/eye as #rrggbb), expression, stance. The enumerated fields take {}. Ranges and the rest are in `describe`'s spec_grammar. A value outside any of them is refused with a sentence naming the field.",
                spec::vocabulary_line("character")
            ))),
        ),
        (
            Verb::NewGame,
            Action::new("new_game", "Save a game spec into the library").arg(Param::text("spec").describe(&format!(
                "The game JSON. Required: title, arena (size, theme), player (character, speed), collectible (kind, count), hazards[] (kind, speed, count), lives, music. `player.character` may name a saved character or inline a whole one, in which case the character vocabulary applies to it too. The enumerated fields take {}. Ranges are in `describe`'s spec_grammar.",
                spec::vocabulary_line("game")
            ))),
        ),
        // Standard, not sensitive like `delete`: iterating on a design is the
        // ordinary use of a kit, the spec it replaces is kept one step back, and
        // the build it drops is milliseconds to redo (#112).
        (
            Verb::UpdateGame,
            Action::new("update_game", "Replace a saved game's spec; the old build, verdict and screenshot go with the old spec, and `build` runs again")
                .arg(Param::text("spec").describe(
                    "The whole game JSON, same grammar as new_game; its title says which saved game it replaces. The spec it replaces is kept beside it as spec.previous.json.",
                ))
                // Optional as the handler has always read it: the spec's own title says
                // which saved game to replace, and `game` is only for when the new spec
                // changes the title.
                .arg(Param::text("game").optional().describe(
                    "Optional: title or slug of the saved game to replace, for when the new spec changes the title",
                )),
        ),
        (
            Verb::UpdateCharacter,
            Action::new("update_character", "Replace a saved character's spec; games that cast it by name lose their builds and need `build` again")
                .arg(Param::text("spec").describe(
                    "The whole character JSON, same grammar as new_character; its name says which saved character it replaces. Games that inline a copy are untouched.",
                )),
        ),
        (
            Verb::Build,
            Action::new("build", "Compile a saved game into its one HTML file")
                .arg(Param::text("game").describe("Title or slug of a saved game")),
        ),
        (
            Verb::Play,
            Action::new("play", "Open a built game in the desktop Browser and confirm it draws; if the Browser has no WebGL, say so and take a headless screenshot instead")
                .defers()
                .arg(Param::text("game").describe("Title or slug of a built game")),
        ),
        (
            Verb::Verify,
            Action::new("verify", "Run the headless gates: boots, clean console, frame renders, input moves, bot wins, bot loses, frame budget. Each gate ends passed, failed or inconclusive; inconclusive means this machine could not settle it (too slow to run a bot to the end of its simulated budget, or too slow for the frame budget to judge the game at all) and is not a verdict on the game, though it is not a pass either")
                .defers()
                .arg(Param::text("game").describe("Title or slug of a built game")),
        ),
        (
            Verb::Screenshot,
            Action::new("screenshot", "Take a headless PNG of a built game")
                .defers()
                .arg(Param::text("game").describe("Title or slug of a built game")),
        ),
        // Sensitive, like Notes' own `trash` and unlike everything else here: a built game
        // is work somebody asked for, and the Trash is a recovery a person has to know
        // about. Not `dangerous` — that grade is for deleting a path the caller names,
        // which is what Files does; this one can only reach Arcade's own library.
        (
            Verb::Delete,
            Action::new("delete", "Move a game to the Trash, where Files can bring it back")
                .risk("sensitive")
                .arg(Param::text("game").describe("Title or slug of a saved game")),
        ),
    ]
}
