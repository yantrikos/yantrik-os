//! The format buttons: read the selection the window held, do the string work, put the selection
//! back.

use yantrik_app::effects::Effects;

use super::{editing::commit, say};
use crate::markdown::{self, Format};
use crate::model::Model;
use crate::YDoc;

pub fn format(m: &mut Model, fx: &mut Effects<YDoc>, what: Format, anchor: usize, cursor: usize) {
    let outcome = markdown::format(m.text(), anchor, cursor, what)
        .and_then(|e| commit(m, fx, e.text, Some((e.start, e.end))));
    if let Err(e) = outcome {
        say(m, e);
    }
}

pub fn link(m: &mut Model, fx: &mut Effects<YDoc>, url: &str, anchor: usize, cursor: usize) {
    let outcome = markdown::format_link(m.text(), anchor, cursor, url)
        .and_then(|e| commit(m, fx, e.text, Some((e.start, e.end))));
    if let Err(e) = outcome {
        say(m, e);
    }
}
