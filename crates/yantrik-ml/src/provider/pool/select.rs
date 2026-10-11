//! Which free model takes a request.
//!
//! 1. Only providers the person switched on, with a key when one is needed.
//! 2. Only models that can do the job: tools, JSON, enough context, good enough at code.
//! 3. Never a provider that may train on prompts for a private turn; never one whose terms do not
//!    allow it for an answer the public will see.
//! 4. A task keeps the model it started on while that model has room (`sticky`): an agent loop
//!    that changes model every step gets worse answers and wastes the providers' caches.
//! 5. Otherwise the model with the most room left, weighted toward stronger coders when the
//!    work is code; ties are taken in turn, so equal providers share the load.

/// What a request needs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Need {
    pub tools: bool,
    pub json: bool,
    /// Tokens the conversation needs to fit.
    pub min_context: u32,
    /// It is writing or reading code: prefer, and require, a good coder.
    pub coding: bool,
    /// It holds nothing of the person's, so it may go to a provider that may train on what it is
    /// sent. Off unless a caller says so: a caller that forgets keeps the person's context away
    /// from those providers (security review of #535, 1 Oct 2026), rather than sending it.
    pub shareable: bool,
    /// The answer goes to the public (a visitor's question on the live stream).
    pub public: bool,
    /// The task it belongs to, to keep it on one model.
    pub sticky: Option<String>,
}

/// One candidate: a provider's model, and how much room it has.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub provider: &'static str,
    pub model: &'static str,
    pub coding: u8,
    pub room: f64,
}

/// The candidate to use, given the ones that fit and still have room. `turn` breaks ties in turn.
pub fn choose<'a>(need: &Need, fits: &'a [Candidate], sticky: Option<(&str, &str)>, turn: usize) -> Option<&'a Candidate> {
    if let Some((p, m)) = sticky {
        if let Some(c) = fits.iter().find(|c| c.provider == p && c.model == m) {
            return Some(c);
        }
    }
    let weight = if need.coding { 1.0 } else { 0.25 };
    let score = |c: &Candidate| c.room * (1.0 + weight * f64::from(c.coding));
    let best = fits.iter().map(score).fold(f64::NEG_INFINITY, f64::max);
    if !best.is_finite() {
        return None;
    }
    // Everything within a tenth of the best shares the load, in turn.
    let near: Vec<&Candidate> = fits.iter().filter(|c| score(c) >= best * 0.9).collect();
    near.get(turn % near.len().max(1)).copied()
}
