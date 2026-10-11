//! What a run is called at the top of its pane.
//!
//! The heading was the prompt itself, cut at 200 characters: a role's whole brief, or a raw
//! command such as `:beliefs the user`, set in the page's largest type. A heading is a name, so
//! it is now the first sentence of the task, short enough to read at a glance, and the prompt as
//! it was sent goes under it in small secondary text for whoever needs the exact words.

use crate::notification_title::{clip_at_word, first_sentence_end};

/// The longest a heading may be, ellipsis included.
pub const HEADING_CHARS: usize = 60;

/// What a run with nothing to name it by is called.
pub const UNTITLED: &str = "Untitled run";

/// The heading for a run asked `prompt`.
///
/// The first line of the prompt, and of that the first sentence, cut at a word to
/// [`HEADING_CHARS`]. A prompt that starts with `:` is a command typed to the mind rather than a
/// task, and says so: `:beliefs the user` is "Command: beliefs the user".
pub fn heading(prompt: &str) -> String {
    let Some(line) = prompt.lines().map(str::trim).find(|l| !l.is_empty()) else {
        return UNTITLED.to_string();
    };
    let named = match line.strip_prefix(':') {
        Some(command) if command.trim().is_empty() => return UNTITLED.to_string(),
        Some(command) => format!("Command: {}", command.trim()),
        None => line[..first_sentence_end(line).unwrap_or(line.len())].trim().to_string(),
    };
    // Control and bidi characters go, by the notification card's rule: a heading is one line.
    let flat = crate::notification_sender::one_line(&named);
    if flat.chars().count() <= HEADING_CHARS {
        return flat;
    }
    // One under the limit, so the ellipsis `clip_at_word` adds still fits inside it.
    clip_at_word(&flat, HEADING_CHARS - 1)
}

/// The heading and the line under it, for a run asked `prompt`. The line is cut at `max`.
pub fn named(prompt: &str, max: usize) -> (String, String) {
    let heading = heading(prompt);
    let raw = raw_label(prompt, &heading, max);
    (heading, raw)
}

/// The prompt as it was sent, for the line under the heading; empty when the heading already
/// says all of it, so nothing is said twice.
pub fn raw_label(prompt: &str, heading: &str, max: usize) -> String {
    let raw = super::agents::one_line(prompt.trim(), max);
    if raw == heading || raw.is_empty() {
        String::new()
    } else {
        raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_raw_command_is_named_as_a_command() {
        assert_eq!(heading(":beliefs the user"), "Command: beliefs the user");
        assert_eq!(heading(":"), UNTITLED);
    }

    #[test]
    fn nothing_to_name_it_by_is_an_untitled_run() {
        assert_eq!(heading(""), UNTITLED);
        assert_eq!(heading("  \n\n "), UNTITLED);
    }

    #[test]
    fn the_heading_is_the_first_sentence_of_the_first_line() {
        assert_eq!(
            heading("You are the Researcher on this desktop.\n\nFind out what is true."),
            "You are the Researcher on this desktop."
        );
        assert_eq!(heading("Tidy the photos folder. Dupes into Trash."), "Tidy the photos folder.");
        // `v1.2` is not the end of a sentence.
        assert_eq!(heading("ship v1.2 on Friday"), "ship v1.2 on Friday");
    }

    #[test]
    fn a_long_sentence_is_cut_at_a_word_within_sixty_characters() {
        let h = heading(&"word ".repeat(40));
        assert!(h.chars().count() <= HEADING_CHARS, "{h}");
        assert!(h.ends_with('…'), "{h}");
    }

    #[test]
    fn the_raw_label_is_only_shown_when_it_adds_something() {
        let prompt = ":beliefs the user";
        assert_eq!(raw_label(prompt, &heading(prompt), 200), ":beliefs the user");
        let plain = "tidy the photos folder";
        assert_eq!(raw_label(plain, &heading(plain), 200), "");
    }
}
