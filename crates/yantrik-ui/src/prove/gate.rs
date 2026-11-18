//! When the companion is not asked: Private mode, and no provider.

/// What stops a proof before anything is asked or written.
pub trait Gate {
    /// The words saying why not, or `None` when the proofs may run.
    fn refusal(&self) -> Option<String>;
}

/// The shell's own answer, from the switches it already keeps.
pub struct ShellGate;

impl Gate for ShellGate {
    fn refusal(&self) -> Option<String> {
        if crate::private_mode::is_on() {
            return Some("Private mode is on, so the companion was not asked anything and nothing was written. Turn it off to prove the companion.".into());
        }
        if !crate::wire::ai_status::companion_has_provider() {
            return Some("Yantrik Companion has no provider to think with yet, so it was not asked anything and nothing was written. Choose who does the thinking in Settings \u{2192} AI.".into());
        }
        None
    }
}
