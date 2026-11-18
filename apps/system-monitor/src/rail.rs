//! What the agent rail knows and offers here.
//!
//! This app's whole content is measurements, so its context is the readings themselves. None of
//! it is asked of a model: the machine knows what it is doing. The one thing a model adds is
//! what the numbers MEAN, and that is the only suggestion.

use yantrik_app::mind::Reach;
use yantrik_app::rail::{ContextItem, HasRail, Prompt, Proposal, Rail, Source, Suggestion};

use crate::model::Model;

const TITLE: &str = "The machine right now";
const FROM: &str = "from the current readings";

/// The current readings, phrased as a question for the companion. Shared by the rail's
/// suggestion and the header's AI Explain, so the two cannot come to differ.
///
/// Handed over as readings. Describing them in prose first and asking the model to re-derive
/// them is how a monitor starts reporting numbers nobody measured.
pub fn machine_question(m: &Model) -> String {
    // The status row as the person reads it, and the limits it was checked against.
    let row = [m.status.lead.as_str(), m.status.rest.as_str()]
        .iter()
        .filter(|part| !part.is_empty())
        .map(|part| part.to_string())
        .collect::<Vec<_>>()
        .join(" · ");
    let facts = format!(
        "{row}; memory {} of {}; {}",
        m.readout.memory_used, m.readout.memory_total, m.status.thresholds
    );
    format!(
        "Here are my machine's readings: {facts}. In at most three short lines say whether \
         anything needs attention and why. Use only these numbers; do not guess at causes \
         you cannot see."
    )
}

impl Source for Model {
    fn context(&self) -> Vec<ContextItem> {
        let r = &self.readout;
        vec![
            ContextItem {
                id: "cpu".into(),
                // No figure before the first interval: one read is the average since boot.
                label: if r.cpu_measured {
                    format!("CPU {:.0}%", r.cpu_usage)
                } else {
                    "CPU —".into()
                },
                detail: "processor".into(),
                source: "file".into(),
            },
            ContextItem {
                id: "mem".into(),
                label: format!("Memory {} of {}", r.memory_used, r.memory_total),
                detail: format!("{:.0}% in use", r.memory_usage),
                source: "file".into(),
            },
        ]
    }

    fn suggestions(&self, reach: Reach) -> Vec<Suggestion> {
        if reach != Reach::Ready {
            return Vec::new();
        }
        vec![Suggestion {
            id: "explain".into(),
            label: "Is anything wrong?".into(),
            detail: "reads the current numbers".into(),
            icon: "spark".into(),
            proposes: false,
        }]
    }

    fn prompt(&self, suggestion: &str) -> Option<Prompt> {
        if suggestion != "explain" {
            return None;
        }
        // The same question the header's AI Explain asks, from the same readings.
        Some(Prompt {
            text: machine_question(self),
            title: TITLE.into(),
            source: FROM.into(),
        })
    }

    fn proposal(&self, _suggestion: &str, answer: &str) -> Proposal {
        // An answer to read: no impact, so the card draws no Apply, and Close goes to dismiss.
        Proposal {
            title: TITLE.into(),
            body: answer.into(),
            source: FROM.into(),
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
