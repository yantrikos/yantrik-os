//! A notification's title and body: the first sentence over the whole text.
//!
//! Sign-off, 4 October: a companion card was titled "Okay, this one's actually good: your
//! calendar shows WebGL debugging at 9:30 and…" over a body reading "a 'browser/arcade session'
//! at 10:00 — and your hobby memories keep…". One sentence sawn in two: the title its first 80
//! characters, the body whatever was left. The saw was the shell's own poster for unprompted
//! companion thoughts (`headline_and_rest` in wire/notifications.rs), which sent every thought
//! that way, and Today, which draws "name · body" on one elided line, showed the body starting in
//! the middle of a sentence.
//!
//! A title is a title: the first sentence, short enough to read at a glance, cut at a word and
//! marked with an ellipsis only when the sentence is longer. The body is the whole text, so it
//! reads from its first word. [`title_of`] is that rule; [`shown`] applies it to what is in the
//! store, for the notification centre and Today alike (`notification_groups::display_copy`).

use crate::notification_sender::one_line;

/// The longest a title may be, ellipsis included.
pub const TITLE_CHARS: usize = 70;

/// The title for `text`: the first sentence of its first line, cut at a word to
/// [`TITLE_CHARS`]. A line break is the writer saying where a thought stops, so a first line
/// with no full stop in it is the sentence.
pub fn title_of(text: &str) -> String {
    let Some(line) = text.lines().map(str::trim).find(|l| !l.is_empty()) else {
        return String::new();
    };
    let flat = one_line(line);
    let flat = flat.trim();
    let sentence = flat[..first_sentence_end(flat).unwrap_or(flat.len())].trim();
    if sentence.chars().count() <= TITLE_CHARS {
        return sentence.to_string();
    }
    // One under the limit, so the ellipsis `clip_at_word` adds still fits inside it.
    clip_at_word(sentence, TITLE_CHARS - 1)
}

/// The title and body a card shows for what a sender filed as `title` and `body`.
///
/// A title the sender wrote short is a title, and it is left as sent, body and all. A long one,
/// or none at all, is rebuilt: the whole text is the title and the body joined, the title is
/// [`title_of`] it, and the body is all of it. A title that ends in "…" with a body after it is
/// the old cut, and the body picks up at the word the title stopped before, so the two join back
/// into what was said.
pub fn shown(title: &str, body: &str) -> (String, String) {
    let head = one_line(title);
    let head = head.trim();
    if !head.is_empty() && head.chars().count() <= TITLE_CHARS {
        return (title.to_string(), body.to_string());
    }
    let body = body.trim();
    let whole = match (head.is_empty(), body.is_empty()) {
        (true, _) => body.to_string(),
        (false, true) => head.to_string(),
        (false, false) => format!("{} {body}", head.strip_suffix('\u{2026}').unwrap_or(head).trim_end()),
    };
    let title = title_of(&whole);
    // Nothing is said twice: a text that is all title has no body.
    let body = if one_line(&whole).trim() == title { String::new() } else { whole };
    (title, body)
}

/// Where the first sentence of a line ends, as a byte index just past its full stop.
///
/// A full stop is `.`, `!` or `?` followed by a space or the end of the line, together with
/// anything that closes with it — `?!`, a quote, a bracket. `3.5` and `v1.2` are not sentence
/// ends, because what follows them is not a space. That is as much sentence detection as a
/// notification title has any use for.
pub fn first_sentence_end(line: &str) -> Option<usize> {
    let mut chars = line.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if !matches!(c, '.' | '!' | '?') {
            continue;
        }
        let mut end = i + c.len_utf8();
        while let Some(&(j, next)) = chars.peek() {
            if matches!(next, '.' | '!' | '?' | '"' | '\'' | '\u{2019}' | '\u{201d}' | ')' | ']') {
                end = j + next.len_utf8();
                chars.next();
            } else {
                break;
            }
        }
        match chars.peek() {
            None => return Some(end),
            Some(&(_, next)) if next.is_whitespace() => return Some(end),
            _ => {}
        }
    }
    None
}

/// `text` cut to at most `max` characters at a word, with an ellipsis to say so; whole when it
/// fits. A word that runs to exactly `max` is kept: the cut looks one character past the limit
/// to see whether it falls on a space.
pub fn clip_at_word(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let hard: String = text.chars().take(max).collect();
    let past: String = text.chars().take(max + 1).collect();
    let cut = past.rsplit_once(' ').map(|(head, _)| head).unwrap_or(&hard);
    format!("{}…", cut.trim_end_matches([',', ';', ':', '.', ' ']))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sign-off card, as the old poster filed it.
    const CUT_TITLE: &str = "Okay, this one's actually good: your calendar shows WebGL debugging at 9:30 and\u{2026}";
    const CUT_BODY: &str = "a 'browser/arcade session' at 10:00 \u{2014} and your hobby memories keep pointing at games.";

    #[test]
    fn a_long_one_sentence_text_is_titled_by_its_start_and_the_body_is_all_of_it() {
        let (title, body) = shown(CUT_TITLE, CUT_BODY);
        assert!(title.chars().count() <= TITLE_CHARS, "{title}");
        assert!(title.ends_with('\u{2026}'), "the sentence is longer than the title: {title}");
        assert!(title.starts_with("Okay, this one's actually good: your calendar"), "{title}");
        assert_eq!(
            body,
            "Okay, this one's actually good: your calendar shows WebGL debugging at 9:30 and a \
             'browser/arcade session' at 10:00 \u{2014} and your hobby memories keep pointing at games.",
            "the body reads from the first word, not from where the title stopped"
        );
    }

    #[test]
    fn a_multi_sentence_text_is_titled_by_its_first_sentence() {
        let text = "Your backup finished. It took nine minutes, twice as long as usual, because two disks were busy.";
        let (title, body) = shown(text, "");
        assert_eq!(title, "Your backup finished.", "a whole sentence that fits has no ellipsis");
        assert_eq!(body, text);
        // A body with no title is the same text.
        assert_eq!(shown("", text), (title.clone(), text.to_string()));
        // And the first line is where a first sentence stops looking.
        assert_eq!(title_of("Here's the read:\n\n2,114 memories. Most are noise."), "Here's the read:");
    }

    #[test]
    fn a_short_title_and_a_body_are_left_as_sent() {
        let (title, body) = shown("Stand-up in 20 minutes", "Room 4 and online. Bring the notes.");
        assert_eq!((title.as_str(), body.as_str()), ("Stand-up in 20 minutes", "Room 4 and online. Bring the notes."));
        // Even one with two sentences in it: the sender chose it.
        assert_eq!(shown("Done. All good.", "").0, "Done. All good.");
        // And a text that is only a short sentence has no body to repeat it in.
        assert_eq!(shown("", "One line."), ("One line.".to_string(), String::new()));
    }

    #[test]
    fn a_long_sentence_is_cut_at_a_word() {
        // Every word is the same eight letters, so a cut in the middle of one is visible.
        let long = "alphabet ".repeat(20);
        let title = title_of(&long);
        assert!(title.chars().count() <= TITLE_CHARS, "{title}");
        assert!(title.ends_with("alphabet\u{2026}"), "cut mid-word: {title}");
        assert!(title.trim_end_matches('\u{2026}').split(' ').all(|w| w == "alphabet"), "{title}");
        // A word that ends exactly at the limit is kept whole.
        assert_eq!(clip_at_word("abcd efgh ijkl", 9), "abcd efgh\u{2026}");
        // No space to cut at: a hard cut, still inside the limit.
        assert_eq!(title_of(&"x".repeat(100)).chars().count(), TITLE_CHARS);
    }

    #[test]
    fn a_version_number_is_not_a_full_stop() {
        assert_eq!(title_of("ship v1.2 on Friday"), "ship v1.2 on Friday");
        assert_eq!(title_of("Tidy the photos folder. Dupes into Trash."), "Tidy the photos folder.");
    }

    #[test]
    fn the_title_is_one_line_whatever_the_sender_wrote() {
        let (title, _) = shown("", &format!("Backups\u{202E}done {}", "and more ".repeat(10)));
        assert!(!title.chars().any(|c| c.is_control() || ('\u{202A}'..='\u{202E}').contains(&c)), "{title:?}");
    }
}
