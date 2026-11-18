//! What a snippet may hold, and how what is typed is tidied before it is kept.

use super::Snippet;

/// Limits, so one enormous paste cannot make the store unloadable.
pub const MAX_CODE: usize = 256 * 1024;
pub const MAX_TITLE: usize = 200;
pub const MAX_TAGS: usize = 400;
pub const MAX_SNIPPETS: usize = 5000;
pub const MAX_FILE: usize = 64 * 1024 * 1024;

pub(super) fn validate(snippet: &Snippet) -> Result<(), String> {
    if snippet.code.len() > MAX_CODE {
        return Err(format!(
            "a snippet can hold up to {} KiB of code",
            MAX_CODE / 1024
        ));
    }
    if snippet.title.chars().count() > MAX_TITLE {
        return Err(format!("a title can be up to {MAX_TITLE} characters"));
    }
    if snippet.tags.chars().count() > MAX_TAGS {
        return Err(format!("tags can be up to {MAX_TAGS} characters"));
    }
    if snippet.title.contains('\0') || snippet.code.contains('\0') {
        return Err("a snippet must be text, and this one contains a null byte".into());
    }
    Ok(())
}

/// A title always says something: an empty one becomes the same words the New button uses.
pub(super) fn clean_title(title: &str) -> String {
    let title = title.replace(['\r', '\n'], " ");
    let title = title.trim();
    if title.is_empty() {
        "Untitled Snippet".to_string()
    } else {
        title.chars().take(MAX_TITLE).collect()
    }
}

/// Tags as the screen shows them: comma separated, trimmed, no empties, no repeats.
pub(super) fn clean_tags(tags: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    for tag in tags.replace(['\r', '\n'], " ").split(',') {
        let tag = tag.trim();
        if tag.is_empty() || kept.iter().any(|t| t.to_lowercase() == tag.to_lowercase()) {
            continue;
        }
        kept.push(tag.to_string());
    }
    kept.join(", ").chars().take(MAX_TAGS).collect()
}

pub(super) fn clean_collection_name(name: &str) -> Result<String, String> {
    let name: String = name
        .replace(['\r', '\n'], " ")
        .trim()
        .chars()
        .take(80)
        .collect();
    if name.is_empty() {
        return Err("a collection needs a name".into());
    }
    Ok(name)
}
