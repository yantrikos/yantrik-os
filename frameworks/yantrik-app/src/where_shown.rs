//! Where an app's window is shown: the person's screen, Mind View, or nowhere (#1129).
//!
//! A Mind that opens an app is answered "Done", and "Done" says the command ran, not that anyone
//! can see anything. Two Minds told the person "it's on your screen" when the window had opened in
//! Mind View, and one when the instance had been started on demand with no window at all (#1112,
//! F19 of #1126). So the framework, not each app, says where the window is, from two facts it
//! already holds: how many windows the process has open, and whether it is drawn for a Mind
//! (`for_mind`: the launch marker, or a display the shell records as a Mind's).
//!
//! The fact is added to every `app.describe`, and to the reply of every action that opens,
//! shows or launches ([`stamps`]). The shell's `open_app` answers with the same word, from its own
//! look at the compositors (`mind_landing::Place::shown`).

use serde_json::Value;

/// The key every reply carries it under.
pub const KEY: &str = "shown";

/// What a windowless instance's open says, so a Mind does not report a picture nobody can see.
pub const NOT_SHOWN: &str = "loaded, not shown: no window of this app is open. Call present_to_person to show it on the person's screen";

/// Where the window is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Where {
    /// A window on the person's display.
    Person,
    /// A window on a Mind's display (Mind View).
    MindView,
    /// No window at all: an instance started to answer a Mind.
    Nowhere,
}

impl Where {
    /// The word on the wire.
    pub fn key(self) -> &'static str {
        match self {
            Where::Person => "person",
            Where::MindView => "mind-view",
            Where::Nowhere => "nowhere",
        }
    }
}

/// The one rule. `windows` is how many windows are open, `None` when the runtime has not said
/// (an app on the older runtime always has its one window); `for_mind` is [`crate::for_mind`].
pub fn derive(windows: Option<u32>, for_mind: bool) -> Where {
    match (windows, for_mind) {
        (Some(0), _) => Where::Nowhere,
        (_, true) => Where::MindView,
        _ => Where::Person,
    }
}

/// The count of open windows, as the core last reported it; `u32::MAX` until it has.
static WINDOWS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(u32::MAX);

/// The core's report of how many windows are open. Called from the UI thread, read from the RPC one.
pub(crate) fn note_windows(count: u32) {
    WINDOWS.store(count, std::sync::atomic::Ordering::Relaxed);
}

/// [`derive`] for this process, now.
pub fn now() -> Where {
    let count = WINDOWS.load(std::sync::atomic::Ordering::Relaxed);
    derive((count != u32::MAX).then_some(count), crate::for_mind::from_env())
}

/// Whether an action's reply is one that says where something was shown: it opens, shows,
/// presents or launches.
pub fn stamps(action: &str) -> bool {
    action == "launch"
        || action == "show"
        || action.starts_with("open")
        || action.starts_with("present")
        || action.starts_with("show_app")
}

/// Add `shown` to a describe answer or an action's reply (and, for no window, what to do next).
pub fn stamp(reply: &mut Value) {
    stamp_as(reply, now());
}

fn stamp_as(reply: &mut Value, at: Where) {
    let Some(map) = reply.as_object_mut() else { return };
    map.insert(KEY.into(), at.key().into());
    if at == Where::Nowhere {
        map.insert("shown_note".into(), NOT_SHOWN.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shown_is_derived_for_each_case() {
        assert_eq!(derive(Some(1), false), Where::Person);
        assert_eq!(derive(Some(2), false), Where::Person);
        assert_eq!(derive(Some(1), true), Where::MindView);
        assert_eq!(derive(Some(0), false), Where::Nowhere);
        assert_eq!(derive(Some(0), true), Where::Nowhere, "no window is nowhere, whoever it was for");
        assert_eq!(derive(None, false), Where::Person, "the older runtime always has its window");
        assert_eq!(derive(None, true), Where::MindView);
    }

    #[test]
    fn a_windowless_open_reports_nowhere_and_how_to_show_it() {
        let mut reply = json!({ "accepted": true });
        stamp_as(&mut reply, derive(Some(0), false));
        assert_eq!(reply["shown"], "nowhere");
        assert!(reply["shown_note"].as_str().unwrap().starts_with("loaded, not shown"));
        let mut seen = json!({ "accepted": true });
        stamp_as(&mut seen, Where::MindView);
        assert_eq!(seen["shown"], "mind-view");
        assert!(seen.get("shown_note").is_none());
    }

    #[test]
    fn launch_like_actions_are_stamped_and_others_are_not() {
        for a in ["open", "open_file", "present", "present_to_person", "launch", "show", "open_message"] {
            assert!(stamps(a), "{a}");
        }
        for a in ["rename", "delete", "set_zoom", "show_date"] {
            assert!(!stamps(a), "{a}");
        }
    }
}
