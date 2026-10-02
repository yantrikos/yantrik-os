//! Did the window a launch promised actually appear, and where?
//!
//! `open_app` hands the launch to the dock and used to answer `launching` at once, which a caller
//! read as "done". For a mind that was `accepted: True` with no window anywhere: Mind View empty,
//! the person's desktop untouched, `pgrep foot` finding nothing (2 Oct 2026). Acceptance is not
//! arrival, so the answer now waits for the window and says what it saw.
//!
//! The wait is a pure loop over an injected probe ([`wait_for_window`]), so the cases are tested
//! without a compositor; [`probe_for`] is the real one. It runs off the UI thread, in the worker
//! `control::answer_later` hands the answer to.

use std::time::{Duration, Instant};

/// How long a launch has to put up a window before the answer is "it did not".
pub const BUDGET: Duration = Duration::from_secs(8);

/// How often the probe is asked while waiting.
const STEP: Duration = Duration::from_millis(250);

/// Where a window appeared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// Inside Mind View, the minds' own desk.
    MindView,
    /// On the person's desktop.
    Desktop,
}

impl Place {
    pub fn words(self) -> &'static str {
        match self {
            Place::MindView => "Mind View",
            Place::Desktop => "the person's desktop",
        }
    }
}

/// What one look found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seen {
    Window(Place),
    /// The launch is known to have failed, with the reason in words.
    Failed(String),
    /// Nothing yet.
    Nothing,
}

/// Ask `probe` until a window shows, the launch is known to have failed, or `budget` runs out.
/// `step` is the pause between looks (zero in tests).
pub fn wait_for_window(
    probe: &mut dyn FnMut() -> Seen,
    budget: Duration,
    step: Duration,
) -> Result<Place, String> {
    let deadline = Instant::now() + budget;
    loop {
        match probe() {
            Seen::Window(place) => return Ok(place),
            Seen::Failed(why) => return Err(why),
            Seen::Nothing => {}
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "no window appeared within {} s of the launch",
                budget.as_secs().max(1)
            ));
        }
        std::thread::sleep(step);
    }
}

/// The real probe for the app whose window id is `id`. Cheap checks first: a recorded failure,
/// then the window itself — in Mind View's compositor when the app was launched there, otherwise
/// on the person's desktop.
pub fn probe_for(id: String) -> impl FnMut() -> Seen {
    move || {
        if let Some(failure) = crate::running::last_launch_failure(&id) {
            return Seen::Failed(format!(
                "`{}` exited with {} after {} ms, before it showed a window",
                failure.binary, failure.status, failure.lived_ms
            ));
        }
        if crate::mind_view::is_mind_view_app(&id) {
            return match crate::mind_view::nested_window_lines() {
                Some(lines) if crate::mind_view::lists_app(&lines, &id) => {
                    Seen::Window(Place::MindView)
                }
                _ => Seen::Nothing,
            };
        }
        if crate::windows::list_windows().iter().any(|w| w.app_id == id) {
            return Seen::Window(Place::Desktop);
        }
        Seen::Nothing
    }
}

/// `open_app`'s answer, from what the wait found. `base` is the answer as it stood (`launching`,
/// `describe_as`); the window is added to it, or the failure is the answer.
pub fn answer(
    mut base: serde_json::Value,
    name: &str,
    seen: Result<Place, String>,
) -> Result<serde_json::Value, String> {
    match seen {
        Ok(place) => {
            base["window"] = "appeared".into();
            base["where"] = place.words().into();
            if place == Place::MindView {
                if let Some(display) = crate::mind_view::display_now() {
                    base["display"] = display.into();
                }
            } else {
                base["note"] = "the app was already open on the person's desktop and was used where \
                                it is, not raised over their work"
                    .into();
            }
            Ok(base)
        }
        Err(why) => Err(format!(
            "`{name}` was started but there is no window to show for it: {why}. Nothing was opened \
             on the person's desktop. `describe shell` lists `failed_launches` and `mind_view`."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(mut looks: Vec<Seen>) -> (Result<Place, String>, usize) {
        let mut asked = 0;
        let mut probe = || {
            asked += 1;
            if looks.is_empty() { Seen::Nothing } else { looks.remove(0) }
        };
        let out = wait_for_window(&mut probe, Duration::from_millis(30), Duration::ZERO);
        (out, asked)
    }

    #[test]
    fn a_window_that_appears_is_reported_where_it_is() {
        let (out, asked) = run(vec![Seen::Nothing, Seen::Window(Place::MindView)]);
        assert_eq!(out, Ok(Place::MindView));
        assert_eq!(asked, 2);
    }

    /// The bug itself: the launch was accepted, nothing ever drew, and the answer said done.
    #[test]
    fn open_app_answers_the_truth_when_no_window_appears() {
        let (out, _) = run(vec![]);
        let why = out.expect_err("no window must not be an answer of success");
        assert!(why.contains("no window appeared"), "{why}");
        let answered = answer(serde_json::json!({ "launching": "terminal" }), "terminal", Err(why));
        let text = answered.expect_err("the answer to a launch with no window is an error");
        assert!(text.contains("no window to show"), "{text}");
        assert!(text.contains("Nothing was opened on the person's desktop"), "{text}");
    }

    #[test]
    fn a_launch_known_to_have_died_does_not_wait_out_the_budget() {
        let (out, asked) = run(vec![Seen::Failed("it exited".into()), Seen::Window(Place::Desktop)]);
        assert_eq!(out, Err("it exited".to_string()));
        assert_eq!(asked, 1);
    }

    #[test]
    fn the_answer_names_where_the_window_is() {
        let ok = answer(serde_json::json!({ "launching": "notes" }), "notes", Ok(Place::MindView)).unwrap();
        assert_eq!(ok["window"], "appeared");
        assert_eq!(ok["where"], "Mind View");
        let desk = answer(serde_json::json!({}), "notes", Ok(Place::Desktop)).unwrap();
        assert_eq!(desk["where"], "the person's desktop");
        assert!(desk["note"].as_str().unwrap().contains("not raised"));
    }
}
