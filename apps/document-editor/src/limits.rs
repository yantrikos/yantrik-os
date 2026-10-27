//! What one document may hold.

/// One document is read and written whole, so the bound is on the whole of it.
///
/// The same 1 MiB the Text Editor uses. A megabyte of Markdown is about 170,000 words; past that
/// this app would be lying about being able to edit it in a single `TextInput`.
pub const MAX_BYTES: usize = 1024 * 1024;

/// What this app will hold, said before anything is written or replaced. Every way text reaches
/// the document is held to it: a file opened, a draft recovered, typing, `append`, `set_content`,
/// a replace, a format button.
pub fn validate(text: &str) -> Result<(), String> {
    if text.len() > MAX_BYTES {
        return Err(format!("A document is limited to 1 MiB; this one is {} bytes.", text.len()));
    }
    if text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
        return Err("That is not a text document: it contains control bytes. yDoc edits UTF-8 Markdown.".into());
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
        assert!(validate(&"\n".repeat(30_000)).is_ok(), "there is no line bound, only the 1 MiB one");
    }
}
