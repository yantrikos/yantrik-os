//! What the agent rail knows and offers here: the list of downloads, and the one question worth
//! asking about it. The header's AI button asks the same question through the same path.

use yantrik_app::mind::Reach;
use yantrik_app::rail::{ContextItem, HasRail, Prompt, Proposal, Rail, Source, Suggestion};

use crate::describe::LISTING_CAP;
use crate::engine::Download;
use crate::model::Model;

/// The rail's one suggestion, and what the header's AI button asks.
pub const EXPLAIN: &str = "explain";

const TITLE: &str = "Your downloads";
const SOURCE: &str = "from the download list";

impl Source for Model {
    /// Nothing to point at: the question carries the list.
    fn context(&self) -> Vec<ContextItem> {
        Vec::new()
    }

    fn suggestions(&self, reach: Reach) -> Vec<Suggestion> {
        if reach != Reach::Ready || self.items().is_empty() {
            return Vec::new();
        }
        vec![Suggestion {
            id: EXPLAIN.into(),
            label: "What needs my attention?".into(),
            detail: "reads the download list".into(),
            icon: "spark".into(),
            proposes: false,
        }]
    }

    /// The list, handed over as the list. Describing it in prose first and asking the model to
    /// re-derive the numbers is how an app starts reporting sizes nobody measured.
    fn prompt(&self, suggestion: &str) -> Option<Prompt> {
        if suggestion != EXPLAIN {
            return None;
        }
        Some(Prompt {
            text: format!(
                "These are the downloads on my machine right now:\n{}\nIn at most three short \
                 lines say what is worth my attention and what to do about it. Use only these \
                 rows; do not guess at causes you cannot see.",
                facts(&self.items())
            ),
            title: TITLE.into(),
            source: SOURCE.into(),
        })
    }

    /// The card's verb is "Close": the companion's answer is an answer, not an edit.
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

/// The list as lines a model can read, capped the same way `describe` is.
///
/// One line a download, with the words the surface already publishes. A row that is restored or
/// interrupted says so, because "this has been sitting paused since your last session" is exactly
/// the kind of thing worth being told and nothing else in the prompt would reveal it.
pub fn facts(items: &[Download]) -> String {
    if items.is_empty() {
        return "(nothing queued)".to_string();
    }
    let mut lines: Vec<String> = items
        .iter()
        .rev()
        .take(LISTING_CAP)
        .map(|d| {
            let mut line = format!("- {} \u{2014} {}, {}", d.filename, d.status.as_str(), d.size_text());
            if d.interrupted {
                line.push_str(", interrupted by the last shutdown");
            } else if d.restored {
                line.push_str(", from a previous session");
            }
            if d.checksum_status == "fail" {
                line.push_str(", checksum does not match");
            }
            if !d.error.is_empty() {
                line.push_str(&format!(", error: {}", d.error));
            }
            line
        })
        .collect();
    if items.len() > LISTING_CAP {
        lines.push(format!("- (and {} more not listed)", items.len() - LISTING_CAP));
    }
    lines.join("\n")
}
