//! The agent layer: the rail's two suggestions, the AI panel's three controls, and the card.
//!
//! Every row is something this document or the companion actually holds, and says where it came
//! from: the document's own headings are the context, because a heading list is the one thing here
//! that is true without asking anybody, and what the Mind remembers about its title is added when
//! it is reachable (update/memory.rs). The panel's Summarise and Improve buttons, its free prompt,
//! and the rail's two suggestions are one path: the rail asks the Mind from a job and the answer
//! comes back as a card that says what applying it would do.
//!
//! A rewrite is applied through the app's own `set_content`, as an agent, so the authorised intent
//! path and the agent rules hold. A summary is shown beside the document and there is nothing to
//! apply: swapping a five-bullet summary in for the thing it summarises is losing the document,
//! whatever the card calls itself.
//!
//! TODO(#951): when the window is drawn for a Mind (`Model::for_mind`), the headings and the card
//! carry document text. Mask them with the shared helper once #951 lands; until then the memory
//! rows are not asked for in Mind View (update/memory.rs) and nothing else is hidden. The same
//! follow-up must stop the rail's apply passing as the person's press in a Mind's window: the core
//! marks it `is_person_applying` without looking at `Boot::for_mind` (cx/rail_hooks.rs), so either
//! the core skips the mark there or yDoc refuses `RailCommand::Apply` and `Activate` in Mind View.

use serde_json::json;
use yantrik_app::effects::Effects;
use yantrik_app::mind::Reach;
use yantrik_app::rail::{self, Apply, ContextItem, HasRail, Prompt, Proposal, Rail, RailCommand, Source, Suggestion};

use crate::markdown;
use crate::model::Model;
use crate::update::say;
use crate::YDoc;

/// The rail's suggestions, and the panel's buttons.
pub const SUMMARIZE: &str = "summarize";
pub const TIGHTEN: &str = "tighten";
/// A free prompt from the panel: the person's words follow.
pub const ASK: &str = "ask:";

/// How many headings the rail's context lists.
const HEADINGS: usize = 6;

const SUMMARY_PROMPT: &str = "Summarise this document in at most five bullet points.";
const TIGHTEN_PROMPT: &str = "Rewrite this document to be shorter and clearer, keeping every fact.";
const FROM: &str = "from this document";

/// What kind of answer an ask is for, which decides both the prompt's closing line and what the
/// proposal offers to do with the answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// The answer is a new version of the document, and applying it replaces what is on screen.
    Rewrite,
    /// The answer is about the document: a summary. There is nothing to apply.
    Summary,
}

fn kind_of(id: &str) -> Option<Kind> {
    match id {
        SUMMARIZE => Some(Kind::Summary),
        TIGHTEN => Some(Kind::Rewrite),
        other if other.starts_with(ASK) => Some(Kind::Rewrite),
        _ => None,
    }
}

/// The ask, built. The closing line is the one that used to be wrong: every ask, summaries
/// included, ended with "Reply with the rewritten document only", so the model answered a summary
/// request with a rewrite and the card offered to swap it in.
pub fn ask_prompt(prompt: &str, body: &str, kind_is_summary: bool) -> String {
    let closing = if kind_is_summary { "Reply with the summary only." } else { "Reply with the rewritten document only." };
    format!("{prompt}\n\nHere is the document. Use only what it says; invent nothing. {closing}\n\n{body}")
}

impl Source for Model {
    fn context(&self) -> Vec<ContextItem> {
        let mut rows: Vec<ContextItem> = markdown::outline(self.text())
            .into_iter()
            .take(HEADINGS)
            .map(|h| ContextItem {
                id: format!("heading:{}", h.offset),
                label: h.title,
                detail: format!("H{}", h.level),
                source: "outline".into(),
            })
            .collect();
        rows.extend(self.memory.iter().cloned());
        rows
    }

    fn suggestions(&self, reach: Reach) -> Vec<Suggestion> {
        if reach != Reach::Ready || markdown::word_count(self.text()) == 0 {
            return Vec::new();
        }
        vec![
            Suggestion {
                id: SUMMARIZE.into(),
                label: "Summarise it".into(),
                detail: "five bullets, from what it says".into(),
                icon: "template".into(),
                proposes: true,
            },
            Suggestion {
                id: TIGHTEN.into(),
                label: "Make it shorter".into(),
                detail: "clearer, keeping every fact".into(),
                icon: "spark".into(),
                proposes: true,
            },
        ]
    }

    fn prompt(&self, suggestion: &str) -> Option<Prompt> {
        let kind = kind_of(suggestion)?;
        if self.text().trim().is_empty() {
            return None;
        }
        let asking = match suggestion {
            SUMMARIZE => SUMMARY_PROMPT,
            TIGHTEN => TIGHTEN_PROMPT,
            other => other.strip_prefix(ASK).unwrap_or_default(),
        };
        Some(Prompt {
            text: ask_prompt(asking, self.text(), kind == Kind::Summary),
            title: if kind == Kind::Summary { "Summarising" } else { "Rewriting" }.into(),
            source: FROM.into(),
        })
    }

    fn proposal(&self, suggestion: &str, answer: &str) -> Proposal {
        match kind_of(suggestion) {
            Some(Kind::Rewrite) => {
                let words = markdown::word_count(answer);
                Proposal {
                    title: "Rewritten document".into(),
                    body: answer.into(),
                    source: FROM.into(),
                    // This one really does replace what is on screen, so it says so and says how
                    // big the replacement is.
                    impact: format!("Replaces the document with {words} words, unsaved"),
                    destructive: false,
                    verb: "Replace".into(),
                    applies: Some(Apply { intent: "set_content".into(), args: json!({ "text": answer }) }),
                }
            }
            _ => Proposal {
                title: "Summary".into(),
                body: answer.into(),
                source: FROM.into(),
                impact: "The document itself is left alone".into(),
                destructive: false,
                verb: "Close".into(),
                applies: None,
            },
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

/// The card the panel puts up itself when the rail cannot ask: an empty document, or no model.
fn card(title: &str, body: impl Into<String>) -> Proposal {
    Proposal { title: title.into(), body: body.into(), verb: "Close".into(), ..Default::default() }
}

/// A question about the document, from the panel or the rail. An empty document is said, and a
/// Mind that cannot answer is said in its own words, on the same card.
pub fn ask(m: &mut Model, fx: &mut Effects<YDoc>, id: String) {
    m.card = None;
    if m.text().trim().is_empty() {
        m.card = Some(card("Nothing to work on", "This document is empty."));
        return;
    }
    if let Err(refused) = rail::update::<YDoc>(m, RailCommand::Activate(id), fx) {
        match refused.kind {
            "no_mind" => m.card = Some(card("The companion did not answer", refused.sentence)),
            _ => say(m, refused.sentence),
        }
    }
}

/// The rail's own commands, with the asks routed through the document's checks and the card put
/// away with the rail's.
pub fn command(m: &mut Model, fx: &mut Effects<YDoc>, command: RailCommand) {
    match command {
        RailCommand::Activate(id) => ask(m, fx, id),
        other => {
            if matches!(other, RailCommand::Dismiss) {
                m.card = None;
            }
            if let Err(refused) = rail::update::<YDoc>(m, other, fx) {
                say(m, refused.sentence);
            }
        }
    }
}
