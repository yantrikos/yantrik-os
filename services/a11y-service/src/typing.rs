//! What a focused element's role says about what is typed into it (#1257 review, M2).
//!
//! A person's typing is the most private thing the accessibility bus carries, so the rules about
//! it are kept apart and plain: which roles are a secret, which are somewhere text is written.

/// The role a toolkit gives a field whose text is hidden as it is typed.
pub const PASSWORD: &str = "password text";

/// The role of a focused element whose role could not be learned. It is treated as a password:
/// not knowing is not a reason to tell.
pub const UNKNOWN_ROLE: &str = "unknown";

/// Whether what is typed into an element of this role is a secret: a password, or something that
/// might be.
pub fn is_secret(role: &str) -> bool {
    role == PASSWORD || role == UNKNOWN_ROLE
}

/// Whether text is written into an element of this role: the changes around it, in the same
/// application, are a person typing, however the toolkit announces them. Rich editors announce
/// each keystroke as children being added, not as text changing.
pub fn is_editor(role: &str) -> bool {
    matches!(role, "text" | "entry" | "paragraph" | "terminal") || role.starts_with("document") || is_secret(role)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_and_what_is_not_known_are_secret_and_other_roles_are_not() {
        assert!(is_secret("password text") && is_secret("unknown"));
        assert!(!is_secret("text") && !is_secret("push button") && !is_secret(""));
    }

    #[test]
    fn editors_are_where_text_is_written() {
        for role in ["text", "entry", "paragraph", "terminal", "document web", "document frame", "document text", "password text", "unknown"] {
            assert!(is_editor(role), "{role}");
        }
        for role in ["push button", "menu item", "frame", "dialog", "list item", "label", "page tab"] {
            assert!(!is_editor(role), "{role}");
        }
    }
}
