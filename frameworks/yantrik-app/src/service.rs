//! The service start: an instance with no window, started for an agent (design #978, step 1).
//!
//! An agent's act or describe on an app that is not running used to be told to open it, and a
//! Mind that opened it got a window of its own in Mind View. Now the shell starts the app's one
//! instance on the person's display with [`FLAG`] and no window, from the person's session, and
//! the agent is answered by the instance. Such an instance:
//! - loads its store and serves intents and `describe` (`App::start_service`), and does nothing
//!   the person sees: `App::start` and the launch wait for the person's first window;
//! - never hands a launch to a running copy, and never shows a window until the person's launch
//!   asks for one: the person's launch then presents a native window of the instance, on the
//!   person's display, as for any running copy;
//! - stays windowless after the person's last window closes for as long as agents use it, and
//!   ends once none has for [`IDLE`].
//!
//! Only the shell starts one. A launch drawn for a Mind (`YANTRIK_FOR_MIND`, or a display that is
//! not the person's) never becomes the instance (`refuse_minds_launch`): the instance is always
//! on the person's display.

use std::ffi::OsString;
use std::time::Duration;

/// The command-line flag of a service start.
pub const FLAG: &str = "--service";

/// How long an instance with no window ends after the last time an agent used it (and after it
/// was started, when none has).
pub const IDLE: Duration = Duration::from_secs(300);

/// `args` without [`FLAG`], and whether it was there. The flag is the first argument or not at
/// all: one anywhere else is an argument like any other and is refused as an option.
pub fn split_flag(mut args: Vec<OsString>) -> (bool, Vec<OsString>) {
    if args.first().is_some_and(|a| a == FLAG) {
        args.remove(0);
        (true, args)
    } else {
        (false, args)
    }
}

/// What a launch drawn for a Mind is told, by an app that will not become its window or its
/// instance: the one next step, as the shell's own refusal says it, with the name the person
/// calls the app by ('Notes', 'Images'), from the one list of the framework's apps.
pub fn refuse_minds_launch(app_id: &str) -> String {
    yantrik_ipc_contracts::mind_view::refused_for_minds(
        yantrik_ipc_contracts::framework_apps::name_of(app_id),
        app_id,
    )
}

/// The refusal a copy of the app `app_id` gives when it is drawn for a Mind ('for_mind': the
/// launch marker, or a display that is a Mind's), service start or not. A copy never becomes the
/// instance on a Mind's display: the instance is always on the person's, started by the shell
/// from the person's session, which carries no marker and runs on the person's display. So a
/// `--service` copy that is drawn for a Mind is not the shell's, and is refused like any other.
pub fn start_refusal(app_id: &str, for_mind: bool) -> Option<String> {
    for_mind.then(|| refuse_minds_launch(app_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_flag_is_taken_only_as_the_first_argument() {
        assert_eq!(split_flag(os(&["--service"])), (true, vec![]));
        assert_eq!(split_flag(os(&["--service", "a.md"])), (true, os(&["a.md"])));
        assert_eq!(split_flag(os(&["a.md", "--service"])), (false, os(&["a.md", "--service"])));
        assert_eq!(split_flag(vec![]), (false, vec![]));
    }

    #[test]
    fn a_minds_launch_is_told_the_next_step_in_the_name_the_person_knows() {
        assert_eq!(
            refuse_minds_launch("notes"),
            "Notes cannot open in your view; use its actions and os_describe notes instead."
        );
        assert_eq!(
            refuse_minds_launch("image-viewer"),
            "Images cannot open in your view; use its actions and os_describe image-viewer instead."
        );
        assert_eq!(
            refuse_minds_launch("calendar"),
            "Calendar cannot open in your view; use its actions and os_describe calendar instead."
        );
        // An app that is not in the list is called by its id.
        assert_eq!(
            refuse_minds_launch("unlisted"),
            "unlisted cannot open in your view; use its actions and os_describe unlisted instead."
        );
    }

    /// H1: a copy drawn for a Mind is refused whether or not it is a service start.
    #[test]
    fn a_copy_drawn_for_a_mind_is_refused_and_one_that_is_not_is_not() {
        assert!(start_refusal("notes", true).unwrap().starts_with("Notes cannot open in your view"));
        assert_eq!(start_refusal("notes", false), None);
    }
}
