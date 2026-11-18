//! One rule for every prompt, of either kind: the close prompt (a window's close, or a quit) and the
//! leaving prompt (leaving a note whose file is gone).
//!
//! - **One prompt at a time.** While one window's prompt asks, or a close waits on its saves, any
//!   other window's new prompt is refused, said in that window only: "Answer the other window's
//!   question first." (or "Please wait…"). A close with nothing to ask still closes.
//! - **Only where it asked.** A prompt is drawn and answered only in the window it asked in.
//! - **No dialog without its prompt.** A window whose prompt dialog is up with no live prompt
//!   behind it (a state no path should leave) has the dialog put away after the command, and is
//!   never drawn with it: there is never a modal that nothing answers.

use yantrik_app::app::WindowId;
use yantrik_app::documents::close::Stage;
use yantrik_app::effects::Effects;

use super::closing::{self, CLOSE_DIALOG};
use super::{leaving, said};
use crate::model::Model;
use crate::view::render::{CHROME, STATUS};
use crate::Notes;

/// Whether `window` owns a live prompt, of either kind.
pub fn owns(m: &Model, window: WindowId) -> bool {
    leaving::asks_in(m, window) || closing::asks_in(m, window)
}

/// Why a new prompt raised from `window` must wait: another window's prompt asks, or a close
/// waits on its saves. `quitting`: the new prompt is a quit's, which is no window's own, so any
/// close flow is another. `None` when nothing stands in its way.
pub fn waits_on_another(m: &Model, window: WindowId, quitting: bool) -> Option<&'static str> {
    if let Some(flow) = &m.closing {
        let other = quitting || flow.window != Some(window);
        if other {
            return Some(match flow.stage {
                Stage::Asking => said::ANSWER_THE_OTHER_FIRST,
                _ => said::WAIT_BEFORE_CLOSING,
            });
        }
    }
    m.leaving.as_ref().filter(|l| l.window != window).map(|_| said::ANSWER_THE_OTHER_FIRST)
}

/// The refusal of a new prompt raised from `window`, said there and nowhere else.
pub fn refuse(m: &mut Model, fx: &mut Effects<Notes>, window: WindowId, sentence: &str) {
    m.say_in(vec![window], sentence);
    fx.changed(STATUS);
}

/// After every command: a prompt dialog with no live prompt behind it is put away.
pub fn settle(m: &mut Model, fx: &mut Effects<Notes>) {
    let orphaned: Vec<WindowId> =
        m.panes().filter(|p| p.dialog == CLOSE_DIALOG && !owns(m, p.window)).map(|p| p.window).collect();
    for window in &orphaned {
        if let Some(pane) = m.pane_of_mut(*window) {
            pane.dialog = 0;
        }
    }
    if !orphaned.is_empty() {
        fx.changed(CHROME);
    }
}
