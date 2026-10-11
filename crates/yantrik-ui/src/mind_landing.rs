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

/// How long a launch has to put up a window before the answer is "none listed yet".
pub const BUDGET: Duration = Duration::from_secs(8);

/// How often the probe is asked while waiting.
pub const STEP: Duration = Duration::from_millis(250);

/// What `running::mark_launch_failed`'s status starts with when the launch was refused before any
/// process started (Mind View was down): nothing ran anywhere, so nothing is "exited with".
pub const REFUSED: &str = "refused: ";

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
    /// The launch was refused before anything started, with the reason in words.
    Refused(String),
    /// The program started and exited, with what is known of how.
    Failed(String),
    /// Nothing yet.
    Nothing,
}

/// Why the wait ended without a window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Miss {
    Refused(String),
    Failed(String),
    /// Nothing was listed in the time given. Says nothing about where the window went, or whether
    /// it will still appear.
    Timeout(u64),
}

/// Ask `probe` until a window shows, the launch is known to have failed, or `budget` runs out.
/// `step` is the pause between looks (zero in tests).
pub fn wait_for_window(
    probe: &mut dyn FnMut() -> Seen,
    budget: Duration,
    step: Duration,
) -> Result<Place, Miss> {
    let deadline = Instant::now() + budget;
    loop {
        match probe() {
            Seen::Window(place) => return Ok(place),
            Seen::Refused(why) => return Err(Miss::Refused(why)),
            Seen::Failed(why) => return Err(Miss::Failed(why)),
            Seen::Nothing => {}
        }
        if Instant::now() >= deadline {
            return Err(Miss::Timeout(budget.as_secs().max(1)));
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
            return match failure.status.strip_prefix(REFUSED) {
                Some(why) => Seen::Refused(why.to_string()),
                None => Seen::Failed(format!(
                    "`{}` exited with {} after {} ms, before it showed a window",
                    failure.binary, failure.status, failure.lived_ms
                )),
            };
        }
        if crate::mind_view::is_mind_view_app(&id) {
            let binary = crate::running::running()
                .into_iter()
                .find(|a| a.app_id == id)
                .map(|a| a.binary)
                .unwrap_or_default();
            return match crate::mind_view::nested_window_lines() {
                Some(lines) if crate::mind_view::lists_app(&lines, &id, &[&binary]) => {
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
/// `describe_as`); the window is added to it, or the failure is the answer. It says only what was
/// observed: "nothing was opened on the person's desktop" is added for a refusal, where the
/// shell knows it, and never for a timeout, where a slow start may still land anywhere.
pub fn answer(
    mut base: serde_json::Value,
    name: &str,
    seen: Result<Place, Miss>,
) -> Result<serde_json::Value, String> {
    match seen {
        Ok(place) => {
            base["window"] = "appeared".into();
            base["where"] = place.words().into();
            if place == Place::MindView {
                if let Some(display) = crate::mind_view::display_now() {
                    base["display"] = display.into();
                }
            }
            Ok(base)
        }
        Err(Miss::Refused(why)) => Err(format!(
            "`{name}` was not started: {why}. It was not opened on the person's desktop. \
             `describe shell` lists `failed_launches` and `mind_view`."
        )),
        Err(Miss::Failed(why)) => Err(format!(
            "`{name}` was started but there is no window to show for it: {why}. \
             `describe shell` lists `failed_launches` and `mind_view`."
        )),
        Err(Miss::Timeout(secs)) => Err(format!(
            "`{name}` was started but no window of it was listed within {secs} s. It may still be \
             starting; `describe shell` lists `failed_launches` and `mind_view`, and its window \
             will show in `list_windows` if it comes up."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(mut looks: Vec<Seen>) -> (Result<Place, Miss>, usize) {
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

    /// The bug itself: the launch was accepted, nothing ever drew, and the answer said done. And
    /// the repaired answer claims nothing it did not check: a timeout says nothing about the
    /// person's desktop, because a cold browser can still land there at second nine.
    #[test]
    fn open_app_answers_the_truth_when_no_window_appears() {
        let (out, _) = run(vec![]);
        let miss = out.expect_err("no window must not be an answer of success");
        assert!(matches!(miss, Miss::Timeout(_)), "{miss:?}");
        let text = answer(serde_json::json!({ "launching": "terminal" }), "terminal", Err(miss))
            .expect_err("the answer to a launch with no window is an error");
        assert!(text.contains("no window of it was listed"), "{text}");
        assert!(text.contains("may still be starting"), "{text}");
        assert!(!text.contains("desktop"), "a timeout must not assert where nothing went: {text}");
    }

    /// Only a refusal, where the shell knows nothing was started, says the desktop was spared; and
    /// it is not dressed up as "exited with ..." (it did not run).
    #[test]
    fn a_refusal_is_told_as_one_and_only_it_vouches_for_the_desktop() {
        let text = answer(
            serde_json::json!({}),
            "notes",
            Err(Miss::Refused("Mind View is not available (no labwc)".into())),
        )
        .unwrap_err();
        assert!(text.contains("was not started: Mind View is not available (no labwc)"), "{text}");
        assert!(text.contains("not opened on the person's desktop"), "{text}");
        assert!(!text.contains("exited with"), "{text}");
        let died = answer(serde_json::json!({}), "notes", Err(Miss::Failed("`notes` exited with 1 after 40 ms".into())))
            .unwrap_err();
        assert!(!died.contains("desktop"), "{died}");
    }

    #[test]
    fn a_launch_known_to_have_died_does_not_wait_out_the_budget() {
        let (out, asked) = run(vec![Seen::Failed("it exited".into()), Seen::Window(Place::Desktop)]);
        assert_eq!(out, Err(Miss::Failed("it exited".to_string())));
        assert_eq!(asked, 1);
    }

    #[test]
    fn the_answer_names_where_the_window_is() {
        let ok = answer(serde_json::json!({ "launching": "notes" }), "notes", Ok(Place::MindView)).unwrap();
        assert_eq!(ok["window"], "appeared");
        assert_eq!(ok["where"], "Mind View");
        let desk = answer(serde_json::json!({}), "notes", Ok(Place::Desktop)).unwrap();
        assert_eq!(desk["where"], "the person's desktop");
    }
}
