//! At rest, Studio draws nothing and costs next to nothing (the kit's rule, #944 and #949),
//! measured the way the display pays for it.
//!
//! What a person sees as idle CPU in front of Studio is not Studio's: the prompt box takes the
//! keyboard when the window opens (as it always did), and a focused text field blinks its caret,
//! which asks the window for a frame every step (#975). With the window not active, as measured
//! below, the same Studio asks for none. What Studio itself arms is one timer, the folder's poll
//! every four seconds, which reads two directory timestamps and redraws only if they moved.

use std::time::Duration;

use serde_json::json;
use slint::platform::WindowEvent;
use slint::ComponentHandle as _;

use super::fake;
use crate::StudioApp;

#[test]
fn a_window_that_is_not_asking_for_a_caret_draws_nothing_and_arms_only_the_poll() {
    let f = fake("rest");
    f.act("generate", json!({ "prompt": "a", "count": 2, "width": 64, "height": 64 })).unwrap();
    f.settled();
    f.app.resize(1180, 800);
    f.app.draw_until_still();
    // The caret of the focused prompt box is the keyboard's, not the app's: move the keyboard off.
    f.app.ui(|ui: &StudioApp| ui.window().dispatch_event(WindowEvent::WindowActiveChanged(false)));
    f.app.draw_until_still();
    f.app.settle_frames();
    assert_eq!(f.app.timers_armed(), 1, "more than the folder's poll is armed");
    let rest = f.app.on_screen(Duration::from_secs(60), Duration::from_secs(1));
    assert_eq!(rest.frames, 0, "a minute at rest drew {} frames", rest.frames);
    // The poll: fifteen looks at two timestamps in a minute, and no frame from any of them.
    assert_eq!(rest.fired, 15);
    assert_eq!(rest.jobs, 15);
}

