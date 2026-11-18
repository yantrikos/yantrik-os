//! What a string from another application is cut to, at the door.
//!
//! Every title, name and role this service holds was written by some other program, and nothing
//! stops that program writing ten megabytes. A string that is kept in the model, copied into
//! every event and handed to every reader has to have a size someone chose (#1257 review, H4).
//!
//! [`name`] is also the one place every published title and element name passes through, so it
//! is where the secret-name filter of #1247 goes when it lands: a single hook, not seven.

/// Titles, application names and element names: longer than any real one, short enough to hold
/// a thousand of.
pub const NAME_MAX: usize = 256;

/// Role names: `push button`, `password text`.
pub const ROLE_MAX: usize = 64;

/// At most `max` bytes of `text`, ending on a character boundary.
pub fn cut(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

/// A window title, application name or element name, as this service keeps and publishes it.
pub fn name(text: &str) -> String {
    cut(text, NAME_MAX)
}

/// A role name, as this service keeps and publishes it.
pub fn role(text: &str) -> String {
    cut(text, ROLE_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_of_a_megabyte_is_cut_at_the_door() {
        let title = "x".repeat(1 << 20);
        assert_eq!(name(&title).len(), NAME_MAX);
        assert_eq!(role(&title).len(), ROLE_MAX);
    }

    #[test]
    fn an_ordinary_name_is_kept_whole() {
        assert_eq!(name("Expenses - March.ods"), "Expenses - March.ods");
        assert_eq!(name(""), "");
    }

    #[test]
    fn a_cut_never_splits_a_character() {
        // 100 three-byte characters: the cut at 256 falls inside one, and steps back to 255.
        let title = "日".repeat(100);
        let kept = name(&title);
        assert_eq!(kept.len(), 255);
        assert!(kept.chars().all(|c| c == '日'));
    }
}
