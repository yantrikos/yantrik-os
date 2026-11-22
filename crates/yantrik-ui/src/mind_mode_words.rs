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
         undo, and before sending mail, typing into web pages and the rest of the always-asked list), `auto` (only destructive actions are asked about), `ask` (anything that matters is \
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

    /// Plain Bypass still asks about mail, typing into web pages and the rest of the always-asked list
    /// (decision A on #1353), and every place that describes it says so: the agent's text, the person's
    /// sentence, the refusal sentence, the Settings page and the menu under the mode chip.
    #[test]
    fn every_description_of_plain_bypass_says_what_it_still_asks() {
        let says = |text: &str| {
            let t = text.to_lowercase();
            t.contains("sending mail") && t.contains("typing into web pages") && t.contains("always-asked list")
        };
        assert!(says(&set_mind_mode_description()), "the agent's description");
        assert!(says(Mode::Bypass.meaning()), "the person's sentence");
        let refusal = crate::mind_mode::lower_from_socket("nonsense").unwrap_err();
        assert!(says(&refusal), "the socket's refusal: {refusal}");
        let settings = include_str!("../../yantrik-ui-slint/ui/settings.slint");
        let menu = include_str!("../../yantrik-ui-slint/ui/components/mind_mode_menu.slint");
        assert!(says(&crate::wire::agents_workroom::mode_line(Mode::Bypass)), "Start work's line");
        for (place, slint) in [("settings", settings), ("the mode menu", menu)] {
            assert!(says(&slint.replace('\n', " ")), "{place} leaves out what Bypass still asks");
        }
    }
}
