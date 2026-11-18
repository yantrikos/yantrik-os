//! What the agent rail knows and offers here: the counts, which are facts the app already has,
//! and one question, only when there are logs on screen to read. Offering to explain logs that
//! are not there is the kind of empty promise the rail exists not to make.

use yantrik_app::mind::Reach;
use yantrik_app::rail::{ContextItem, HasRail, Prompt, Proposal, Rail, Source, Suggestion};

use crate::model::Model;
use crate::verdict::last_lines;

const TITLE: &str = "What these logs say";
const SOURCE: &str = "from the container log";

/// Whether a log has been read, open or not: what the rail and the AI button can speak about.
pub fn has_logs(m: &Model) -> bool {
    !m.logs.text.trim().is_empty()
}

/// What to ask the companion about the log that was read, or `None` when there is none.
///
/// One place, because two controls ask it: the rail's suggestion and the header's AI button.
pub fn log_question(m: &Model) -> Option<String> {
    let tail = last_lines(&m.logs.text, 40);
    if tail.trim().is_empty() {
        return None;
    }
    let name = &m.logs.container;
    Some(format!(
        "These are the last lines of the log for container {name}. In at most four short lines \
         say what is happening and whether anything is wrong. Use only what is shown.\n\n{tail}"
    ))
}

impl Source for Model {
    fn context(&self) -> Vec<ContextItem> {
        vec![ContextItem {
            id: "counts".into(),
            label: format!("{} running, {} stopped", self.running(), self.stopped()),
            detail: self.runtime.clone(),
            source: "file".into(),
        }]
    }

    fn suggestions(&self, reach: Reach) -> Vec<Suggestion> {
        if reach != Reach::Ready || !has_logs(self) {
            return Vec::new();
        }
        vec![Suggestion {
            id: "logs".into(),
            label: "Explain these logs".into(),
            detail: "reads the last of the output".into(),
            icon: "spark".into(),
            proposes: false,
        }]
    }

    fn prompt(&self, suggestion: &str) -> Option<Prompt> {
        if suggestion != "logs" {
            return None;
        }
        Some(Prompt {
            text: log_question(self)?,
            title: TITLE.into(),
            source: SOURCE.into(),
        })
    }

    /// An answer to read: no impact, nothing to apply, so the card draws no Apply button and its
    /// Close goes to dismiss.
    fn proposal(&self, _suggestion: &str, answer: &str) -> Proposal {
        Proposal {
            title: TITLE.into(),
            body: answer.into(),
            source: SOURCE.into(),
            verb: "Close".into(),
            ..Default::default()
        }
    }
}

impl HasRail for Model {
    fn rail(&self) -> &Rail {
        &self.rail
    }

    fn rail_mut(&mut self) -> &mut Rail {
        &mut self.rail
    }
}
