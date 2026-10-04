//! Text a caller wrote, made safe to draw on one line.
//!
//! One rule, here where both the services and the shell can reach it: a caller's words (a claimed
//! name, an argv, an executable's file name) reach a card, a toast or a notification title,
//! and none of them may add a line or reorder what the line seems to say. A Slint Text breaks on
//! `\n`, so an embedded newline forged a second line under "(verified)", and a bidi override
//! could reverse the end of it (security reviews of #614).

/// `text` as one plain line: control characters and Unicode's bidirectional formatting
/// characters become spaces, and runs of whitespace collapse to one space.
pub fn one_line(text: &str) -> String {
    let spaced: String = text
        .chars()
        .map(|c| if c.is_control() || is_bidi_control(c) { ' ' } else { c })
        .collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Unicode's bidirectional formatting characters: marks, embeddings, overrides and isolates.
pub fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newlines_tabs_and_bidi_controls_become_one_space() {
        assert_eq!(one_line("a\r\n\tb\u{200F}c"), "a b c");
        assert_eq!(one_line("evil\u{202E}gnp.exe"), "evil gnp.exe");
        assert_eq!(one_line("  plain words  "), "plain words");
        assert!(!one_line("x\u{0007}\u{2066}y").chars().any(|c| c.is_control() || is_bidi_control(c)));
    }
}
