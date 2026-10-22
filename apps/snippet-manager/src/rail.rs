//! What the agent rail knows and offers here: the open snippet, and the one question worth asking
//! about it.

use yantrik_app::mind::Reach;
use yantrik_app::rail::{ContextItem, HasRail, Prompt, Proposal, Rail, Source, Suggestion};

use crate::model::Model;

/// The rail's one suggestion, and what the header's AI and the pane's AI Explain ask.
pub const EXPLAIN: &str = "explain";

/// How much of a snippet's code the question carries.
const PROMPT_CHARS: usize = 4000;

/// Where the answer comes from, on the card.
const SOURCE: &str = "from the open snippet";

fn about(title: &str) -> String {
    format!("About \u{201c}{title}\u{201d}")
}

impl Source for Model {
    /// The open snippet, read off the editor, so the rail says what the person is looking at: by
    /// title and language, with no measure of its code.
    fn context(&self) -> Vec<ContextItem> {
        if self.editor.id < 0 {
            return Vec::new();
        }
        let language = &self.editor.language;
        vec![ContextItem {
            id: self.editor.id.to_string(),
            label: self.editor.title.clone(),
            detail: if language.is_empty() { "snippet".into() } else { language.clone() },
            source: "file".into(),
        }]
    }

    fn suggestions(&self, reach: Reach) -> Vec<Suggestion> {
        if reach != Reach::Ready || self.open().is_none() {
            return Vec::new();
        }
        vec![Suggestion {
            id: EXPLAIN.into(),
            label: "What does this do?".into(),
            detail: "reads the open snippet".into(),
            icon: "spark".into(),
            proposes: false,
        }]
    }

    /// The snippet as it is kept: the question is about the code, and only the code.
    fn prompt(&self, suggestion: &str) -> Option<Prompt> {
        if suggestion != EXPLAIN {
            return None;
        }
        let snippet = self.open()?;
        let language = if snippet.language.is_empty() { "code" } else { &snippet.language };
        Some(Prompt {
            text: format!(
                "Explain what this {language} snippet does, in at most four short lines. Use only the code.\n\n{}",
                snippet.code.chars().take(PROMPT_CHARS).collect::<String>()
            ),
            title: about(&snippet.title),
            source: SOURCE.into(),
        })
    }

    /// The card's verb is "Close": the companion's answer is an answer, not an edit, so there is
    /// nothing to apply.
    fn proposal(&self, _suggestion: &str, answer: &str) -> Proposal {
        Proposal {
            title: about(self.open().map(|s| s.title.as_str()).unwrap_or_default()),
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
