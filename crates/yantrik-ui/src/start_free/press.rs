//! What the press does, in order, each step the one an existing path already takes:
//!
//! 1. allow sharing for that one account (Settings → AI accounts' switch);
//! 2. give the companion Yantrik models (Settings → Minds, "Use Yantrik models", Apply);
//! 3. pick that account's model (the ask bar's model menu);
//! 4. ask "Can it answer?" (the first run's own check) and say what came back.
//!
//! A step that fails is said, with why, and every step before it is undone: the person is left
//! where they were, and every card is usable. A check that fails (offline, rate-limited) is not
//! a failure of the choice: it is kept, and the page says the provider did not answer.

/// The three steps that change something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Sharing,
    Models,
    Pick,
}

/// What the press came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The provider answered; the words say what it said.
    Answered(String),
    /// The choice is kept; the provider did not answer, and the reason is the plain one.
    Unanswered(String),
    /// A step failed and the ones before it were undone; the sentence says which and why.
    Failed(String),
}

/// A step that did not work: why, and what it could not put back (nothing, when it put back all it wrote).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Failed {
    pub why: String,
    pub left: Vec<String>,
}

impl From<String> for Failed {
    fn from(why: String) -> Failed {
        Failed { why, left: Vec::new() }
    }
}

/// What each step does and undoes. A step that fails puts back whatever it half wrote itself (only
/// the second can: `apply` writes a file, then a token, then a marker) and says in [`Failed::left`]
/// what it could not; `undo_*` is for a step that succeeded, and undoes only what that step changed,
/// never what was there before the press. The real one calls the existing functions (`real.rs`).
pub trait Steps {
    fn allow_sharing(&self) -> Result<(), Failed>;
    fn give_models(&self) -> Result<(), Failed>;
    fn pick_model(&self) -> Result<(), Failed>;
    fn undo_sharing(&self) -> Result<(), String>;
    fn undo_models(&self) -> Result<(), String>;
    /// "Can it answer?": what the provider said, or why it did not.
    fn check(&self) -> Result<String, String>;
}

/// One sentence for a failed step: which, why, and that nothing was changed; or, when something
/// could not be put back, what that was, so "Nothing was changed" is never said when it is not true.
pub fn failure_words(step: Step, provider: &str, why: &str, left: &[String]) -> String {
    let what = match step {
        Step::Sharing => format!("Allowing sharing with {provider}"),
        Step::Models => "Giving Yantrik Companion Yantrik models".to_string(),
        Step::Pick => format!("Choosing {provider}'s model"),
    };
    let why = why.trim().trim_end_matches('.');
    if left.is_empty() {
        return format!("{what} did not work: {why}. Nothing was changed.");
    }
    let left: Vec<&str> = left.iter().map(|l| l.trim().trim_end_matches('.')).collect();
    format!(
        "{what} did not work: {why}. Putting it back also failed: {}. Look at Settings \u{2192} AI & Intelligence and Settings \u{2192} Minds.",
        left.join("; ")
    )
}

/// The sentence for a provider that did not answer, with the choice kept.
pub fn unanswered_words(provider: &str, why: &str) -> String {
    format!(
        "{provider} did not answer: {}. Your choice is kept; try again, or choose another card.",
        why.trim().trim_end_matches('.')
    )
}

pub fn answered_words(provider: &str, said: &str) -> String {
    format!("{provider} answered. {said}")
}

pub fn press(steps: &dyn Steps, provider: &str) -> Outcome {
    if let Err(f) = steps.allow_sharing() {
        return fail(Step::Sharing, provider, f, &[]);
    }
    if let Err(f) = steps.give_models() {
        return fail(Step::Models, provider, f, &[&|| steps.undo_sharing()]);
    }
    if let Err(f) = steps.pick_model() {
        return fail(Step::Pick, provider, f, &[&|| steps.undo_models(), &|| steps.undo_sharing()]);
    }
    check(provider, &|| steps.check())
}

/// Undo what came before, all of it even when one undo fails, and say what could not be put back.
fn fail(step: Step, provider: &str, mut failed: Failed, undos: &[&dyn Fn() -> Result<(), String>]) -> Outcome {
    for undo in undos {
        if let Err(e) = undo() {
            failed.left.push(e);
        }
    }
    Outcome::Failed(failure_words(step, provider, &failed.why, &failed.left))
}

/// Only the check: the press's last step, and "Try again".
pub fn check(provider: &str, ask: &dyn Fn() -> Result<String, String>) -> Outcome {
    match ask() {
        Ok(said) => Outcome::Answered(answered_words(provider, &said)),
        Err(why) => Outcome::Unanswered(unanswered_words(provider, &why)),
    }
}
