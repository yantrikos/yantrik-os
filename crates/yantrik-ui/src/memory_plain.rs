//! What the Memory screen says for a memory the machine wrote for itself in its own codes.
//!
//! UI punch list, 12 October: the Memory screen's first rows read "Security event
//! [information_leak]: llm_response — Response contained sensitive internal information", the
//! line `SecurityGuard::record_event` (yantrik-companion/src/security.rs) stores. The stored
//! memory is left exactly as it is — nothing here rewrites or deletes one — and the screen shows a
//! plain sentence (what happened, whether anything changed, what to do) with the stored line under
//! Details.

/// The sentence to show for a stored memory, and the stored line for Details when the two differ.
/// Anything this does not recognise is shown as stored, with nothing under Details.
pub fn plain(text: &str) -> (String, String) {
    match security_event(text) {
        Some((code, source)) => (security_sentence(code, source), text.to_string()),
        None => (text.to_string(), String::new()),
    }
}

/// `Security event [code]: source — detail` → `(code, source)`.
fn security_event(text: &str) -> Option<(&str, &str)> {
    let rest = text.trim_start().strip_prefix("Security event [")?;
    let (code, rest) = rest.split_once("]:")?;
    let source = rest.split(" \u{2014} ").next().unwrap_or("").trim();
    Some((code.trim(), source))
}

fn security_sentence(code: &str, source: &str) -> String {
    let tool = source.strip_prefix("tool:").map(|t| t.replace(['_', '.'], " "));
    match code {
        "information_leak" => "A reply mentioned private details of how Yantrik works, so they were taken out \
             before you saw it. Nothing else changed, and there is nothing for you to do."
            .into(),
        "injection_attempt" | "learned_pattern_match" => "A message looked like an attempt to give Yantrik hidden \
             instructions, so they were not followed. Nothing changed, and there is nothing for you to do."
            .into(),
        "burst_attack" => "Many unusual messages arrived within a minute, so Yantrik slowed down. Nothing changed, \
             and there is nothing for you to do."
            .into(),
        "data_poisoning" => format!(
            "What {} returned looked like hidden instructions, so they were not followed. Nothing changed. \
             If you did not expect this, check where that information came from.",
            tool.map(|t| format!("the {t} tool")).unwrap_or_else(|| "a tool".into())
        ),
        "harmful_command" => "A command that could have harmed this computer was stopped before it ran. Nothing \
             changed. If you meant to run it, run it yourself in Terminal."
            .into(),
        "permission_escalation" => "A mind tried to use a tool its mode does not allow, so it was stopped. Nothing \
             changed. To allow it, choose a different mode."
            .into(),
        _ => "Yantrik flagged something for safety. Nothing was changed by it; the record is under Details.".into(),
    }
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_security_event_is_a_plain_sentence_and_the_code_is_kept_for_details() {
        // The punch list's row, verbatim.
        let stored = "Security event [information_leak]: llm_response \u{2014} Response contained sensitive internal information";
        let (shown, details) = plain(stored);
        assert!(shown.starts_with("A reply mentioned private details"), "{shown}");
        assert!(shown.contains("Nothing else changed") && shown.contains("nothing for you to do"), "{shown}");
        assert!(!shown.contains(['[', '_']) && !shown.contains("llm_response"), "{shown}");
        assert_eq!(details, stored, "the stored line is kept, whole, for Details");

        for code in ["injection_attempt", "data_poisoning", "harmful_command", "burst_attack",
                     "permission_escalation", "learned_pattern_match", "something_new"] {
            let stored = format!("Security event [{code}]: tool:run_command \u{2014} whatever");
            let (shown, details) = plain(&stored);
            assert!(!shown.contains(['[', '_']) && !shown.contains(code), "{code}: {shown}");
            assert!(shown.contains("changed"), "says whether anything changed: {shown}");
            assert_eq!(details, stored);
        }
    }

    #[test]
    fn an_ordinary_memory_is_shown_as_it_was_kept() {
        let (shown, details) = plain("Pranab prefers window seats.");
        assert_eq!(shown, "Pranab prefers window seats.");
        assert_eq!(details, "");
    }
}
