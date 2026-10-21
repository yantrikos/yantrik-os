//! The notice line, as each window draws it. `Model::notice` is the one line `describe` reads.
//! Some sentences are about some windows only: a close or a leave refused while another window's
//! prompt asks, said in the window that was refused, or a note let go, said in the windows that
//! showed it. Those windows draw the sentence; every other window goes on drawing what was said
//! before it, so a save failure the other window was showing stays in view there.

use yantrik_app::app::WindowId;

use crate::model::Model;

/// A notice drawn only in `windows`. It holds while the notice is still `sentence`: whatever is
/// said next is every window's again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedNotice {
    pub windows: Vec<WindowId>,
    pub sentence: String,
    /// What the other windows draw meanwhile: the notice as it was before.
    pub before: String,
}

impl Model {
    /// `sentence` in the notice, drawn only in `windows` (`notice_for`).
    pub fn say_in(&mut self, windows: Vec<WindowId>, sentence: &str) {
        let before = match self.scoped() {
            Some(scoped) => scoped.before.clone(),
            None => self.notice.clone(),
        };
        self.notice = sentence.into();
        self.lasting_notice = false;
        self.notice_in = Some(ScopedNotice { windows, sentence: sentence.into(), before });
    }

    /// The notice as `window` draws it.
    pub fn notice_for(&self, window: WindowId) -> &str {
        match self.scoped() {
            Some(scoped) if !scoped.windows.contains(&window) => &scoped.before,
            _ => &self.notice,
        }
    }

    /// The scoped notice, while it is still what the notice says.
    fn scoped(&self) -> Option<&ScopedNotice> {
        self.notice_in.as_ref().filter(|s| s.sentence == self.notice)
    }
}
