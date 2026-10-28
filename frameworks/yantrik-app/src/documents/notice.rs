//! The notice line every document app keeps: what could not be done, or what happened that the person must know.
//!
//! It is drawn in the window and read back by every `describe` (`notice`, and the summary), so a
//! sentence about a document (or tab) an agent is not shown (#443) is kept in two forms: the person's, which
//! names the file where it helps them, and the one any agent reads, which names it `(hidden)` and
//! gives no path. Only a sentence about such a document differs; every other is the same for both.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Notice {
    text: String,
    /// What an agent reads instead, when the sentence is about a document it is not shown.
    for_agents: Option<String>,
}

impl Notice {
    /// A sentence for everyone.
    pub fn set(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.for_agents = None;
    }

    /// A sentence about a document: `hidden` when an agent is not shown it, and then `for_agents` is
    /// what an agent reads in its place.
    pub fn set_about(&mut self, hidden: bool, text: impl Into<String>, for_agents: impl FnOnce() -> String) {
        self.text = text.into();
        self.for_agents = hidden.then(for_agents);
    }

    pub fn clear(&mut self) {
        self.set(String::new());
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// What the window shows.
    pub fn window(&self) -> &str {
        &self.text
    }

    /// What this caller reads: an agent the agent-safe form.
    pub fn read(&self, agent: bool) -> &str {
        match &self.for_agents {
            Some(safe) if agent => safe,
            _ => &self.text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sentence_about_a_hidden_document_reads_hidden_to_an_agent_only() {
        let mut n = Notice::default();
        n.set_about(true, "secret.txt moved to /tmp/x", || "(hidden) moved".into());
        assert_eq!((n.window(), n.read(false), n.read(true)), ("secret.txt moved to /tmp/x", "secret.txt moved to /tmp/x", "(hidden) moved"));
        n.set_about(false, "shown.txt moved", || unreachable!("a shown document's sentence is everyone's"));
        assert_eq!(n.read(true), "shown.txt moved");
        n.set_about(true, "a", || "b".into());
        n.set("anyone's");
        assert_eq!(n.read(true), "anyone's", "a later sentence is everyone's again");
    }
}
