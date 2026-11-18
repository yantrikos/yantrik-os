//! What the Mind, the launcher and Files may ask Images by name: the same eight actions, with
//! the same names, descriptions, parameters and grades (none is graded: each is the surface's
//! `standard` floor, and none writes anything that outlives the turn, #48), each the command a
//! button runs.
//!
//! `open`, `next` and `previous` read the disk, so they are answered when the picture has
//! loaded (`Settle::Explicit`), with what is then on screen; the rest are answered at once.

use std::time::Duration;

use serde_json::json;
use yantrik_app::control::{Action, Param};
use yantrik_app::intents::{Agents, Args, Intent, Intents, Settle};
use yantrik_app::Refused;

use crate::agent_rule;
use crate::model::{Command, Model, OpenAs, TurnVerb};
use crate::ImageViewer;

/// How long a picture may take to read before the caller is told it did not finish. A large
/// photograph on a slow disk is seconds; this is a bound on one that is stuck.
const LOAD_WITHIN: Duration = Duration::from_secs(20);

/// How the picture on screen lies, as `rotate` and `flip` answer it. Turning it on screen does
/// not rewrite the file, and saying so keeps a caller from believing the picture on disk has
/// changed.
fn orientation_json(m: &Model) -> serde_json::Value {
    let o = m.shown.orientation;
    json!({
        "rotation": o.rotation,
        "flipped_horizontally": o.flip_h,
        "flipped_vertically": o.flip_v,
        "file_unchanged": true,
    })
}

/// The direction as given; `update` checks there is a picture before it reads it, as the action
/// always did.
fn direction(a: &Args<'_>) -> String {
    a.raw()["direction"].as_str().unwrap_or_default().to_string()
}

pub fn table(table: &mut Intents<ImageViewer>) {
    table.add(
        Intent::new(
            Action::new("open", "Show a picture, and the folder it is in")
                .arg(Param::text("path").describe("Path to an image file")),
            |a| {
                let raw = a.raw()["path"].as_str().unwrap_or_default().trim();
                if raw.is_empty() {
                    return Err(Refused::new("empty_path", "`path` is empty"));
                }
                Ok(Command::Open {
                    path: yantrik_foundation::home::expand(raw),
                    how: OpenAs::Asked,
                })
            },
        )
        .settles(Settle::Explicit { within: LOAD_WITHIN })
        .reply(|m: &Model| json!({ "showing": m.shown.name, "of": m.gallery.len() })),
    );
    table.add(
        Intent::new(Action::new("show", "Bring the window forward"), |_| Ok(Command::Show))
            .reply(|m: &Model| agent_rule::answer_json(&m.answer)),
    );
    // Stepping through a folder an agent may not see names its pictures one by one.
    table.add(
        Intent::new(Action::new("next", "The next picture in the folder"), |_| Ok(Command::Next))
            .agents(Agents::Rule(Box::new(|m: &Model, _: &Args<'_>| agent_rule::may_step(m))))
            .settles(Settle::Explicit { within: LOAD_WITHIN })
            .reply(|m: &Model| agent_rule::answer_json(&m.answer)),
    );
    table.add(
        Intent::new(Action::new("previous", "The previous picture in the folder"), |_| {
            Ok(Command::Previous)
        })
        .agents(Agents::Rule(Box::new(|m: &Model, _: &Args<'_>| agent_rule::may_step(m))))
        .settles(Settle::Explicit { within: LOAD_WITHIN })
        .reply(|m: &Model| agent_rule::answer_json(&m.answer)),
    );
    table.add(
        Intent::new(
            Action::new("rotate", "Turn the picture on screen")
                .arg(Param::text("direction").describe("left | right")),
            |a| Ok(Command::TurnNamed { verb: TurnVerb::Rotate, direction: direction(a) }),
        )
        .reply(orientation_json),
    );
    table.add(
        Intent::new(
            Action::new("flip", "Mirror the picture on screen")
                .arg(Param::text("direction").describe("horizontal | vertical")),
            |a| Ok(Command::TurnNamed { verb: TurnVerb::Flip, direction: direction(a) }),
        )
        .reply(orientation_json),
    );
    table.add(
        Intent::new(
            Action::new("fit", "Fit the picture to the window, or fill the window with it"),
            |_| Ok(Command::Fit),
        )
        .reply(|m: &Model| json!({ "fit_to_window": m.view.fit_contain && !m.view.zoomed })),
    );
    table.add(
        Intent::new(
            Action::new("toggle_info", "Show or hide what the file says about the picture"),
            |_| Ok(Command::ToggleInfo),
        )
        .reply(|m: &Model| json!({ "info_panel": m.view.info_open })),
    );
}

#[cfg(test)]
mod grade_tests {
    use super::*;

    /// The grades as the surface publishes them: every action at the `standard` floor the
    /// app always published them at. Nothing here writes a setting that outlives the turn, so
    /// nothing is `sensitive` (#48); none is `safe` either, since each steers the person's window.
    #[test]
    fn every_action_keeps_the_grade_it_was_published_at() {
        let mut t = Intents::<ImageViewer>::new();
        table(&mut t);
        let specs = t.specs();
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["open", "show", "next", "previous", "rotate", "flip", "fit", "toggle_info"]);
        for spec in &specs {
            assert_eq!(spec.permission, "standard", "`image-viewer.{}`", spec.name);
        }
    }
}
