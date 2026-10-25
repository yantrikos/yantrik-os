//! What the agent rail knows and offers here: the events on the day on screen, the one question
//! worth asking about a day, and the way back to today.
//!
//! Events come off the model the app already renders; nothing is added to fill the column -- a day
//! with nothing on it and no shell behind it gets an empty rail, and the calendar is wider for it.
//! A row is not a button: it is an event on the day already on screen, so there is nowhere for a
//! click to go (`context-opens` stays false on the screen).

use yantrik_app::mind::Reach;
use yantrik_app::rail::{ContextItem, HasRail, Prompt, Proposal, Rail, Source, Suggestion};

use crate::events;
use crate::model::Model;
use crate::update::{EXPLAIN, TODAY};

/// What to ask the companion about the day on screen: its events, as the agenda lists them.
///
/// One place, because two controls ask it: the rail's suggestion and the header's AI button.
pub fn day_question(m: &Model) -> String {
    let day = &m.ai.day;
    let listed: Vec<String> = m
        .on_selected_day()
        .into_iter()
        .map(|e| format!("- {} ({})", e.title, events::time_text(e)))
        .collect();
    if listed.is_empty() {
        format!(
            "My calendar for {day} is empty. In two sentences, say so plainly and suggest one \
             useful thing to do with an open day. Do not invent appointments."
        )
    } else {
        format!(
            "Here is my calendar for {day}:\n{}\n\nIn at most four short lines, tell me what the \
             shape of this day is and what to prepare. Use only what is listed; invent nothing.",
            listed.join("\n")
        )
    }
}

impl Source for Model {
    fn context(&self) -> Vec<ContextItem> {
        self.on_selected_day()
            .into_iter()
            .enumerate()
            .map(|(row, e)| ContextItem {
                id: format!("event:{row}"),
                label: e.title.clone(),
                detail: events::time_text(e),
                source: "calendar".into(),
            })
            .collect()
    }

    fn suggestions(&self, reach: Reach) -> Vec<Suggestion> {
        let mut next = Vec::new();
        if reach == Reach::Ready {
            next.push(Suggestion {
                id: EXPLAIN.into(),
                label: "What does this day look like?".into(),
                detail: "shape of the day, and what to prepare".into(),
                icon: "spark".into(),
                // An answer to read; nothing is moved, so nothing is proposed.
                proposes: false,
            });
        }
        if self.selected_day > 0 {
            next.push(Suggestion {
                id: TODAY.into(),
                label: "Back to today".into(),
                detail: String::new(),
                icon: "search".into(),
                proposes: false,
            });
        }
        next
    }

    fn prompt(&self, suggestion: &str) -> Option<Prompt> {
        (suggestion == EXPLAIN).then(|| Prompt {
            text: day_question(self),
            title: "Your day".into(),
            source: format!("from {}", self.ai.day),
        })
    }

    fn proposal(&self, _suggestion: &str, answer: &str) -> Proposal {
        crate::update::answer_card(&self.ai.day, answer.to_string())
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
