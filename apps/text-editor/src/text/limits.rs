//! What one document may hold.

/// The most one document may hold.
pub const MAX_BYTES: usize = 1024 * 1024;

/// The most lines one document may hold (its newlines).
pub const MAX_LINES: usize = 20_000;

/// Whether `text` may be a document: within 1 MiB and 20,000 lines, and text, with no control
/// character but newline, return and tab. Every way text reaches a tab is held to this: a file
/// opened, a draft recovered, typing, `append`, `set_content`, a replace.
pub fn validate(text: &str) -> Result<(), String> {
    if text.len() > MAX_BYTES {
        return Err("Editing is limited to 1 MiB per document.".into());
    }
    if text.bytes().filter(|b| *b == b'\n').count() > MAX_LINES {
        return Err("Editing is limited to 20,000 lines per document.".into());
    }
    if text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
        return Err("This appears to be a binary file. Choose a UTF-8 text file.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_is_bounded_text() {
        assert!(validate("binary\u{1}control").is_err());
        assert!(validate("tabs\tand\r\nreturns").is_ok());
        assert!(validate(&"a".repeat(MAX_BYTES + 1)).is_err());
        assert!(validate(&"\n".repeat(MAX_LINES + 1)).is_err());
        assert!(validate(&"\n".repeat(MAX_LINES)).is_ok());
    }
}
