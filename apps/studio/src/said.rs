//! A refusal, in the two forms it is said in.
//!
//! The same refusal reaches a Mind that asked and the person at the window. The Mind's form may
//! name the action it should call instead; the person's may not, because the person cannot call
//! an action and a sentence that sends them to one sends them nowhere. Most refusals are one
//! sentence and say it once ([`Said::both`]).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Said {
    /// What a Mind that called an action is told.
    pub agent: String,
    /// What the notice bar says, and so what `describe` carries as the notice.
    pub person: String,
}

impl Said {
    pub fn both(sentence: impl Into<String>) -> Said {
        let sentence = sentence.into();
        Said { agent: sentence.clone(), person: sentence }
    }

    pub fn new(agent: impl Into<String>, person: impl Into<String>) -> Said {
        Said { agent: agent.into(), person: person.into() }
    }
}

impl From<String> for Said {
    fn from(sentence: String) -> Said {
        Said::both(sentence)
    }
}

impl From<&str> for Said {
    fn from(sentence: &str) -> Said {
        Said::both(sentence)
    }
}
