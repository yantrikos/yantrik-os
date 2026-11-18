//! The Mind, as an app reaches it (design/app-framework-2026-10-21.md, §5).
//!
//! The model, the memory and the bond live in the shell process; an app talks to them over the
//! socket bus like any service. This crate is that conversation, typed: [`ask`] runs from a job
//! only (it may take a minute on a cold model), [`recall_relevant`] filters the memory to what
//! is worth a person's eye, [`reach`] says without waiting whether the shell can answer at all,
//! and its sentences are the ones every app says. [`Mind`] is the same surface as a trait, so a
//! feature that asks the Mind is tested against [`fake::FakeMind`] with no shell.

mod live;

pub use live::*;

pub mod fake;

/// What a Mind answers. The live one is [`Live`]; a test's is [`fake::FakeMind`].
pub trait Mind: Send + Sync + std::fmt::Debug {
    /// Ask and wait for the finished answer. From a job, never the UI thread.
    fn ask(&self, prompt: &str) -> Result<String, AskError>;
    /// The memories relevant to `query`, at most `want`, none under `floor`.
    fn recall_relevant(&self, query: &str, floor: f64, want: usize) -> Vec<Recalled>;
    /// One of the companion's tools by name, with no model in the loop.
    fn tool(&self, name: &str, args: serde_json::Value) -> Result<String, String>;
    /// Whether the shell can answer, as last heard; never waits on it.
    fn reach(&self) -> Reach;
}

/// The shell's Mind, over the socket bus.
#[derive(Debug, Clone, Copy, Default)]
pub struct Live;

impl Mind for Live {
    fn ask(&self, prompt: &str) -> Result<String, AskError> {
        live::ask(prompt)
    }

    fn recall_relevant(&self, query: &str, floor: f64, want: usize) -> Vec<Recalled> {
        live::recall_relevant(query, floor, want)
    }

    fn tool(&self, name: &str, args: serde_json::Value) -> Result<String, String> {
        live::tool(name, args)
    }

    fn reach(&self) -> Reach {
        live::reach()
    }
}

/// The live Mind, shared.
pub fn live() -> std::sync::Arc<dyn Mind> {
    std::sync::Arc::new(Live)
}
