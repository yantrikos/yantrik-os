//! What the agent-facing `set_mind_mode` description says about each mode (#1357 round 5), kept in
//! one place so a test can hold it to what the person is told on the red confirmation.

/// What `bypass_all` means to an agent reading the action's description. Full bypass asks nothing
/// (decision D on #1353): a card an agent counts on never comes, so the sentence must not promise one.
pub(crate) const BYPASS_ALL: &str = "`bypass_all` (nothing is asked; everything under the ceiling runs and is recorded)";

/// The whole description of `set_mind_mode`, as published to agents.
pub(crate) fn set_mind_mode_description() -> String {
    format!(
        "Tighten what you may do on this desktop without being asked. Five modes, loosest first: \
         {BYPASS_ALL}, `bypass` (nothing is asked except before what an app marks as impossible to \
         undo), `auto` (only destructive actions are asked about), `ask` (anything that matters is \
         asked about), `plan` (read only — every change is refused). You can only move DOWN this \
         list, and never INTO a bypass: from `bypass_all` you may step down to `bypass`, but neither \
         is entered from here. A request to loosen it is refused: that is the person's decision, made \
         at the keyboard, and `plan` is the useful one to set yourself before a long piece of work \
         you want checked first."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mind_mode::Mode;

    /// The agent is told what the person is told: nothing is asked, and what runs is recorded. Neither
    /// sentence keeps a list of things that still ask, which would be a card an agent waits for.
    #[test]
    fn the_agents_description_and_the_confirmation_say_the_same_of_full_bypass() {
        let confirmation = Mode::BypassAll.meaning().to_lowercase();
        let description = set_mind_mode_description().to_lowercase();
        assert!(confirmation.starts_with("nothing will ask") && confirmation.contains("recorded"), "{confirmation}");
        assert!(description.contains("nothing is asked; everything under the ceiling runs and is recorded"), "{description}");
        for text in [&confirmation, &description] {
            assert!(!text.contains("still asks") && !text.contains("except a connected mind"), "{text}");
        }
        let bypass_all = &description[description.find("`bypass_all`").unwrap()..description.find("`bypass` (").unwrap()];
        assert_eq!(bypass_all.trim_end_matches([',', ' ']), BYPASS_ALL.to_lowercase());
    }
}
