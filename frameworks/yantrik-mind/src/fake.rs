//! A Mind for tests: it answers what it was told to, remembers what it was asked, and its reach
//! is whatever the test says.

use std::collections::VecDeque;
use std::sync::Mutex;

use crate::{AskError, Mind, Reach, Recalled};

#[derive(Debug, Default)]
pub struct FakeMind {
    answers: Mutex<VecDeque<Result<String, AskError>>>,
    memories: Mutex<Vec<Recalled>>,
    reach: Mutex<Option<Reach>>,
    asked: Mutex<Vec<String>>,
    tools: Mutex<Vec<(String, serde_json::Value)>>,
}

impl FakeMind {
    pub fn new() -> FakeMind {
        FakeMind::default()
    }

    /// The next asks are answered with these, in order; past them, `AskError::Failed`.
    pub fn answers(self, answers: impl IntoIterator<Item = Result<String, AskError>>) -> FakeMind {
        self.answers.lock().unwrap_or_else(|p| p.into_inner()).extend(answers);
        self
    }

    pub fn answering(self, text: &str) -> FakeMind {
        self.answers([Ok(text.to_string())])
    }

    pub fn remembering(self, memories: impl IntoIterator<Item = Recalled>) -> FakeMind {
        self.memories.lock().unwrap_or_else(|p| p.into_inner()).extend(memories);
        self
    }

    pub fn reaching(self, reach: Reach) -> FakeMind {
        *self.reach.lock().unwrap_or_else(|p| p.into_inner()) = Some(reach);
        self
    }

    pub fn set_reach(&self, reach: Reach) {
        *self.reach.lock().unwrap_or_else(|p| p.into_inner()) = Some(reach);
    }

    /// Every prompt asked so far, in order.
    pub fn prompts(&self) -> Vec<String> {
        self.asked.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Every tool call so far, in order.
    pub fn tool_calls(&self) -> Vec<(String, serde_json::Value)> {
        self.tools.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl Mind for FakeMind {
    fn ask(&self, prompt: &str) -> Result<String, AskError> {
        self.asked
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(prompt.to_string());
        self.answers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .pop_front()
            .unwrap_or_else(|| Err(AskError::Failed("the fake has no answer left".into())))
    }

    fn recall_relevant(&self, _query: &str, floor: f64, want: usize) -> Vec<Recalled> {
        self.memories
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|m| m.score >= floor)
            .take(want)
            .cloned()
            .collect()
    }

    fn tool(&self, name: &str, args: serde_json::Value) -> Result<String, String> {
        self.tools
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((name.to_string(), args));
        Ok(format!("{name}: done"))
    }

    fn reach(&self) -> Reach {
        self.reach
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .unwrap_or(Reach::NoShell)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fake_answers_in_order_and_remembers_what_it_was_asked() {
        let mind = FakeMind::new()
            .answers([Ok("one".into()), Err(AskError::NoModel)])
            .reaching(Reach::Ready)
            .remembering([
                Recalled { rid: "a".into(), text: "high".into(), score: 0.9 },
                Recalled { rid: "b".into(), text: "low".into(), score: 0.1 },
            ]);
        assert_eq!(mind.ask("first?"), Ok("one".into()));
        assert_eq!(mind.ask("second?"), Err(AskError::NoModel));
        assert!(matches!(mind.ask("third?"), Err(AskError::Failed(_))));
        assert_eq!(mind.prompts(), vec!["first?", "second?", "third?"]);
        assert_eq!(mind.reach(), Reach::Ready);
        assert_eq!(mind.recall_relevant("x", 0.35, 5).len(), 1);
        mind.set_reach(Reach::NoShell);
        assert_eq!(mind.reach().hint(), Some(crate::OFFLINE_HINT));
    }
}
