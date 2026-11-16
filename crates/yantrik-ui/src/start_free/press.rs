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

/// What each step does and undoes. A step that fails puts back whatever it half wrote itself (only
/// the second can: `apply` writes a file, then a token, then a marker); `undo_*` is for a step that
/// succeeded, and undoes only what that step changed, never what was there before the press. The real
/// one calls the existing functions (`real.rs`).
pub trait Steps {
    fn allow_sharing(&self) -> Result<(), String>;
    fn give_models(&self) -> Result<(), String>;
    fn pick_model(&self) -> Result<(), String>;
    fn undo_sharing(&self);
    fn undo_models(&self);
    /// "Can it answer?": what the provider said, or why it did not.
    fn check(&self) -> Result<String, String>;
}

/// One sentence for a failed step: which, why, and that nothing was changed.
pub fn failure_words(step: Step, provider: &str, why: &str) -> String {
    let what = match step {
        Step::Sharing => format!("Allowing sharing with {provider}"),
        Step::Models => "Giving Yantrik Companion Yantrik models".to_string(),
        Step::Pick => format!("Choosing {provider}'s model"),
    };
    format!("{what} did not work: {}. Nothing was changed.", why.trim().trim_end_matches('.'))
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
    if let Err(why) = steps.allow_sharing() {
        return Outcome::Failed(failure_words(Step::Sharing, provider, &why));
    }
    if let Err(why) = steps.give_models() {
        steps.undo_sharing();
        return Outcome::Failed(failure_words(Step::Models, provider, &why));
    }
    if let Err(why) = steps.pick_model() {
        steps.undo_models();
        steps.undo_sharing();
        return Outcome::Failed(failure_words(Step::Pick, provider, &why));
    }
    check(provider, &|| steps.check())
}

/// Only the check: the press's last step, and "Try again".
pub fn check(provider: &str, ask: &dyn Fn() -> Result<String, String>) -> Outcome {
    match ask() {
        Ok(said) => Outcome::Answered(answered_words(provider, &said)),
        Err(why) => Outcome::Unanswered(unanswered_words(provider, &why)),
    }
}
