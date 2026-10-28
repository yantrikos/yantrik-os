//! The spec editor's unsaved text, in the framework's draft lane.
//!
//! The editor is a workbench: a spec is typed (or pasted, or loaded from a character) and then
//! saved as a game or a character. Text the library does not hold yet is drafted as it is typed,
//! debounced, and at once when the app ends. A draft found at the next start is put back in the
//! editor with a line that says so. It goes when the text it holds is kept by a save that has
//! landed, or when the editor is empty.

use serde::{Deserialize, Serialize};
use yantrik_app::effects::Effects;

use crate::model::{Model, DRAFT};
use crate::Arcade;

type Fx = Effects<Arcade>;

/// What the draft holds. Kept as a struct so the file can grow.
#[derive(Serialize, Deserialize)]
pub struct Draft {
    pub text: String,
}

/// The editor's text as a draft's bytes.
fn bytes(text: &str) -> Vec<u8> {
    serde_json::to_vec(&Draft { text: text.to_string() }).unwrap_or_default()
}

/// What a draft found at start holds, if it holds any text.
pub fn recovered(drafts: &yantrik_app::Drafts) -> Option<String> {
    drafts.json::<Draft>(DRAFT).map(|d| d.text).filter(|t| !t.trim().is_empty())
}

/// The editor changed: its unsaved text is drafted, and a draft that nothing needs any more goes.
pub fn typed(m: &mut Model, fx: &mut Fx) {
    if m.unsaved() {
        fx.save_draft(DRAFT, bytes(&m.spec_text));
        m.drafted = true;
    } else if m.drafted {
        fx.discard_draft(DRAFT);
        m.drafted = false;
    }
}

/// The app is ending: unsaved text is written now, without asking.
pub fn at_the_end(m: &mut Model, fx: &mut Fx) {
    if m.unsaved() {
        fx.checkpoint_draft(DRAFT, bytes(&m.spec_text));
        m.drafted = true;
    }
}
