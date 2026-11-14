//! The chat turn for "turn off getting things ready" (#1184).
//!
//! The setting for getting promised things ready is the person's own click (the card's Stop, the
//! switch in Settings → Companion). A model turn that is asked to change it must not try: it ran
//! tools, deleted promises and wrote memories about it (VM 520, bar 11). So a sentence that names the
//! feature and a switch verb is answered here, before any model turn, with a fixed reply that points
//! at the switch and changes nothing: no tools, no learning, no commitment noting.

use yantrik_companion::commitment_consent::{asks_to_switch, SWITCH_REPLY};

/// Answer `own_text` without a model when it asks to change the setting: `say` is given the whole
/// reply, and the result is true. Anything else, and anything the system wrote rather than the
/// person, says nothing and gives false. `own_text` is the person's words alone, before any
/// threading context is put in front.
pub fn answered_without_model(own_text: &str, is_system_generated: bool, say: impl FnOnce(&str)) -> bool {
    if is_system_generated || !asks_to_switch(own_text) {
        return false;
    }
    say(SWITCH_REPLY);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, system: bool) -> (bool, Vec<String>) {
        let mut said = Vec::new();
        let answered = answered_without_model(text, system, |s| said.push(s.to_string()));
        (answered, said)
    }

    #[test]
    fn the_switch_sentence_is_answered_with_the_fixed_reply_and_nothing_else() {
        let (answered, said) = run("Turn off getting things ready.", false);
        assert!(answered, "no model turn is started for it");
        assert_eq!(said, vec![SWITCH_REPLY.to_string()]);
        assert!(SWITCH_REPLY.contains("Settings") && SWITCH_REPLY.contains("I haven't changed anything"));
    }

    #[test]
    fn other_sentences_and_the_systems_own_prompts_go_to_the_model() {
        for text in ["turn off the lights", "I'm getting things ready for the party", "Forget my promise to Maya."] {
            assert_eq!(run(text, false), (false, vec![]), "{text}");
        }
        // A prompt the companion wrote to itself is not the person asking, whatever it says.
        assert_eq!(run("Turn off getting things ready.", true), (false, vec![]));
    }
}
