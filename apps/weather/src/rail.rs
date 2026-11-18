//! What the agent rail knows and offers here: the conditions on screen, and the one question
//! worth asking about a forecast — not what the numbers are, they are right there, but what to
//! do about them.

use yantrik_app::mind::Reach;
use yantrik_app::rail::{ContextItem, HasRail, Prompt, Proposal, Rail, Source, Suggestion};

use crate::forecast::Current;
use crate::model::Model;

/// The one question, built from the reading on screen. Shared by the rail's suggestion and the
/// header's AI Insights button, so the two cannot come to differ.
pub fn conditions_prompt(c: &Current) -> String {
    format!(
        "It is {} at {}, feels like {}. In at most three short lines say what to plan for \
         today. Use only these conditions.",
        c.condition, c.temperature, c.feels_like
    )
}

impl Source for Model {
    fn context(&self) -> Vec<ContextItem> {
        let mut context = Vec::new();
        if let Some(c) = &self.reading.current {
            if !c.temperature.is_empty() {
                context.push(ContextItem {
                    id: "now".into(),
                    label: format!("{}, {}", c.temperature, c.condition),
                    detail: c.feels_like.clone(),
                    source: "file".into(),
                });
            }
        }
        let updated = self.last_updated_text();
        if !updated.is_empty() {
            context.push(ContextItem {
                id: "updated".into(),
                label: updated,
                detail: "last updated".into(),
                source: "file".into(),
            });
        }
        context
    }

    fn suggestions(&self, reach: Reach) -> Vec<Suggestion> {
        let has_reading = self.reading.current.as_ref().is_some_and(|c| !c.temperature.is_empty());
        if reach != Reach::Ready || !has_reading {
            return Vec::new();
        }
        vec![Suggestion {
            id: "advise".into(),
            label: "What should I plan for?".into(),
            detail: "reads today's conditions".into(),
            icon: "spark".into(),
            proposes: false,
        }]
    }

    fn prompt(&self, suggestion: &str) -> Option<Prompt> {
        if suggestion != "advise" {
            return None;
        }
        let current = self.reading.current.as_ref()?;
        Some(Prompt {
            text: conditions_prompt(current),
            title: "Today outside".into(),
            source: "from today's conditions".into(),
        })
    }

    fn proposal(&self, _suggestion: &str, answer: &str) -> Proposal {
        // An answer to read: no impact, nothing to apply, Close goes to dismiss.
        Proposal {
            title: "Today outside".into(),
            body: answer.into(),
            source: "from today's conditions".into(),
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
