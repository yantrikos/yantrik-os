//! The only function that changes the model: every button, intent, launch and job result lands
//! here as a `Command`, and asks for its effects.
//!
//! Reading a picture, reading its folder and turning its pixels are work for a job; the screen
//! changes when that work lands, all at once, and only if it is still the newest asked for.

use std::path::PathBuf;

use yantrik_app::effects::Effects;
use yantrik_app::intents::Origin;
use yantrik_app::jobs::{Concurrency, JobKey, Scope};
use yantrik_app::mind::AskError;
use yantrik_app::rail;
use yantrik_app::Refused;

use crate::agent_rule;
use crate::ai;
use crate::model::{
    Ai, Command, Model, OpenAs, Shown, TurnVerb, Verdict, NO_PICTURE, PART_AI, PART_FIT,
    PART_NOTHING, PART_ORIENTATION, PART_PICTURE, PART_RAISE, PART_SHOWN, PART_VIEW,
};
use crate::open::{self, Loaded};
use crate::orient::Orientation;
use crate::picture;
use crate::ImageViewer;

/// One picture loads at a time: a newer open or step replaces the one in flight.
const PICTURE: JobKey = JobKey {
    feature: "picture",
    scope: Scope::App,
};
/// One turn at a time: a newer press replaces the one in flight.
const TURN: JobKey = JobKey {
    feature: "turn",
    scope: Scope::App,
};
/// One Describe ask at a time.
const DESCRIBE: JobKey = JobKey {
    feature: "describe",
    scope: Scope::App,
};

pub fn update(m: &mut Model, command: Command, fx: &mut Effects<ImageViewer>) -> Result<(), Refused> {
    // The rail says what it changed itself, and nothing else may: a part outside the rail would
    // refresh the rail again, for ever.
    if let Command::Rail(command) = command {
        return rail::update::<ImageViewer>(m, command, fx);
    }
    // Nothing redraws every part: see `PART_NOTHING`.
    fx.changed(PART_NOTHING);
    match command {
        Command::Next => step(m, fx, true)?,
        Command::Previous => step(m, fx, false)?,
        Command::Open { path, how } => start_open(m, fx, path, how),
        Command::Launch(files) => {
            // The first launch's, then any a second copy handed over (`Event::Launch`).
            let how = if m.launched { OpenAs::Asked } else { OpenAs::FirstLaunch };
            m.launched = true;
            // One window, one picture: the first file named is the one opened.
            if let Some(path) = files.into_iter().next() {
                start_open(m, fx, path, how);
            }
        }
        Command::Opened { load, outcome } => {
            if load != m.load {
                // A newer open or step has been asked for since; this one's result is not shown.
                return Ok(());
            }
            match outcome {
                Ok((gallery, loaded)) => {
                    m.gallery = gallery;
                    show(m, loaded, fx);
                    // The window comes forward for an open asked of it, as it always did.
                    if matches!(fx.origin(), Origin::Surface { .. }) {
                        raise(m, fx);
                    }
                }
                Err(why) => {
                    tracing::warn!(error = %why, "could not open the picture");
                    fx.settle(Err(Refused::new("not_opened", why)));
                    // The window is left as it was. A step this open replaced is taken up again,
                    // so the folder's current picture is the one on screen.
                    if m.verdict.as_ref().map(|v| &v.path) != m.gallery.current() {
                        start_load(m, fx);
                    }
                }
            }
        }
        Command::Loaded { load, loaded } => {
            if load != m.load {
                return Ok(());
            }
            show(m, loaded, fx);
            m.answer = agent_rule::answer(m, by_agent(fx), Some(m.gallery.index() + 1));
        }
        Command::Turn(turn) => reorient(m, turn.change(), fx)?,
        Command::TurnNamed { verb, direction } => {
            if !m.shown.has_picture() {
                return Err(no_picture());
            }
            let change: fn(Orientation) -> Orientation = match (verb, direction.to_lowercase().as_str()) {
                (TurnVerb::Rotate, "left" | "anticlockwise" | "counterclockwise") => Orientation::rotate_left,
                (TurnVerb::Rotate, "right" | "clockwise") => Orientation::rotate_right,
                (TurnVerb::Rotate, other) => {
                    return Err(Refused::new("bad_direction", format!("unknown direction `{other}`; use left or right")))
                }
                (TurnVerb::Flip, "horizontal" | "horizontally" | "left-right") => Orientation::flip_horizontal,
                (TurnVerb::Flip, "vertical" | "vertically" | "top-bottom") => Orientation::flip_vertical,
                (TurnVerb::Flip, other) => {
                    return Err(Refused::new(
                        "bad_direction",
                        format!("unknown direction `{other}`; use horizontal or vertical"),
                    ))
                }
            };
            reorient(m, change, fx)?;
        }
        Command::Turned { screen, orientation, pixels } => {
            // Turned pixels of another picture, or for a turn the view has moved on from.
            if screen != m.screen || orientation != m.shown.orientation {
                return Ok(());
            }
            match pixels {
                Some(pixels) => m.shown.turned = Some((orientation, pixels)),
                None => {
                    tracing::warn!("this picture's pixels cannot be read, so it cannot be turned");
                    // The view says how it is drawn: the last turn that landed, or as stored.
                    m.shown.orientation = m.shown.turned.as_ref().map_or_else(Orientation::default, |(o, _)| *o);
                    fx.changed(PART_ORIENTATION);
                }
            }
            fx.changed(PART_PICTURE);
        }
        Command::Fit => {
            if !m.shown.has_picture() {
                return Err(no_picture());
            }
            // The fit button's own step: from a zoom, back to the whole picture; otherwise
            // between the whole picture in the window and the window filled.
            if m.view.zoomed {
                m.view.zoomed = false;
                m.view.fit_contain = true;
            } else {
                m.view.fit_contain = !m.view.fit_contain;
            }
            fx.changed(PART_FIT);
        }
        Command::ToggleInfo => {
            if !m.shown.has_picture() {
                return Err(no_picture());
            }
            m.view.info_open = !m.view.info_open;
            fx.changed(PART_VIEW);
        }
        // What the window shows is what it is: nothing to draw.
        Command::ViewSeen(view) => m.view = view,
        Command::AskDescribe => ask_describe(m, fx),
        Command::Described { name, outcome } => {
            m.ai.working = false;
            fx.changed(PART_AI);
            // The person may have moved to the next picture while the ask was in flight; an
            // answer about the old file must not caption the new one.
            if name != m.shown.name {
                return Ok(());
            }
            m.ai.response = match outcome {
                Ok(text) => text,
                Err(e) => e.to_string(),
            };
        }
        Command::DismissAi => {
            m.ai.response.clear();
            fx.changed(PART_AI);
        }
        Command::Show => {
            m.answer = agent_rule::answer(m, by_agent(fx), None);
            raise(m, fx);
        }
        Command::Rail(_) => unreachable!("handled above"),
    }
    Ok(())
}

fn no_picture() -> Refused {
    Refused::new("no_picture", NO_PICTURE)
}

/// The command runs under an agent's call.
fn by_agent(fx: &Effects<ImageViewer>) -> bool {
    matches!(fx.origin(), Origin::Surface { agent: true })
}

fn raise(m: &mut Model, fx: &mut Effects<ImageViewer>) {
    m.raises += 1;
    fx.changed(PART_RAISE);
}

/// The next or previous picture in the folder, wrapping at its ends.
fn step(m: &mut Model, fx: &mut Effects<ImageViewer>, forward: bool) -> Result<(), Refused> {
    if m.gallery.is_empty() {
        return Err(Refused::new("nothing_open", "no pictures are open"));
    }
    if forward {
        m.gallery.next();
    } else {
        m.gallery.prev();
    }
    start_load(m, fx);
    Ok(())
}

/// Open `path`, off the UI thread. The window does not change until it lands.
fn start_open(m: &mut Model, fx: &mut Effects<ImageViewer>, path: PathBuf, how: OpenAs) {
    m.load += 1;
    let load = m.load;
    let gallery = m.gallery.clone();
    let agent = by_agent(fx);
    fx.spawn_keyed_or(
        PICTURE,
        Concurrency::ReplacePrevious,
        Command::Opened {
            load,
            outcome: Err("the picture could not be opened".into()),
        },
        move |_| Command::Opened {
            load,
            outcome: open::open(path, gallery, agent, how),
        },
    );
}

/// Load the folder's current picture, off the UI thread. Until it lands an agent is shown
/// nothing of it (`agent_rule::hidden`).
fn start_load(m: &mut Model, fx: &mut Effects<ImageViewer>) {
    m.load += 1;
    m.verdict = None;
    let load = m.load;
    let Some(path) = m.gallery.current().cloned() else { return };
    fx.spawn_keyed_or(
        PICTURE,
        Concurrency::ReplacePrevious,
        Command::Loaded {
            load,
            loaded: Loaded::unreadable(path.clone()),
        },
        move |_| Command::Loaded {
            load,
            loaded: open::load(&path),
        },
    );
}

/// Put a loaded picture on screen, with what its file says about it.
fn show(m: &mut Model, loaded: Loaded, fx: &mut Effects<ImageViewer>) {
    let Loaded {
        path,
        info,
        picture,
        shown_to_agents,
    } = loaded;
    // Whatever was said about the last picture is not about this one: the panel would otherwise
    // carry the old description over the new file, which reads as a caption of it.
    m.ai = Ai::default();
    m.shown = Shown {
        name: path.file_name().unwrap_or_default().to_string_lossy().to_string(),
        counter: m.gallery.counter_text(),
        // A file that is not a picture, or one that has gone: the name stays on screen and the
        // reason beside it.
        notice: match &picture {
            Some(_) => String::new(),
            None => format!("Could not open {}", path.display()),
        },
        info,
        picture,
        // A new picture is shown as it is stored, not as the last one was turned.
        orientation: Orientation::default(),
        turned: None,
        path: Some(path.clone()),
    };
    m.verdict = Some(Verdict { path, shown_to_agents });
    m.screen += 1;
    fx.changed(PART_PICTURE);
    fx.changed(PART_SHOWN);
    fx.changed(PART_ORIENTATION);
    fx.changed(PART_AI);
}

/// Turn or mirror the picture on screen. The one path for the toolbar's four buttons and the
/// `rotate` and `flip` actions. The file is never touched: the view is redrawn, and the next
/// picture is shown as it is stored. The turned pixels are made in a job and drawn when they
/// land; the toolbar and `describe` say the new orientation at once.
fn reorient(m: &mut Model, change: fn(Orientation) -> Orientation, fx: &mut Effects<ImageViewer>) -> Result<(), Refused> {
    let Some(picture) = m.shown.picture.clone() else {
        return Err(no_picture());
    };
    let next = change(m.shown.orientation);
    m.shown.orientation = next;
    fx.changed(PART_ORIENTATION);
    if next.is_identity() {
        m.shown.turned = None;
        fx.changed(PART_PICTURE);
        return Ok(());
    }
    let screen = m.screen;
    fx.spawn_keyed(TURN, Concurrency::ReplacePrevious, move |_| Command::Turned {
        screen,
        orientation: next,
        pixels: picture::turn(&picture, next),
    });
    Ok(())
}

/// The Describe button: what the file records about the picture, asked of the companion from a
/// job. With no picture, or no companion to ask, the panel says why instead.
fn ask_describe(m: &mut Model, fx: &mut Effects<ImageViewer>) {
    fx.changed(PART_AI);
    if !m.shown.has_picture() {
        m.ai.response = NO_PICTURE.into();
        return;
    }
    if let Some(hint) = m.rail.reach().hint() {
        m.ai.response = hint.into();
        return;
    }
    let name = m.shown.name.clone();
    let prompt = ai::describe_prompt(&name, &ai::facts(&m.shown.info));
    m.ai = Ai {
        working: true,
        response: String::new(),
    };
    let mind = m.mind.clone();
    fx.spawn_keyed_or(
        DESCRIBE,
        Concurrency::ReplacePrevious,
        Command::Described {
            name: name.clone(),
            outcome: Err(AskError::Failed("the ask did not finish".into())),
        },
        move |_| Command::Described {
            name,
            outcome: mind.ask(&prompt),
        },
    );
}
