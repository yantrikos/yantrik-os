//! The words a search is made of: a title's meaningful words, and the ways a person is named.

/// Words that name the kind of event, not what it is about. Searching a file name or a note for
/// "meeting" finds every file and every note.
const STOP_WORDS: &[&str] = &[
    "meeting", "meet", "sync", "call", "the", "with", "and", "for", "from", "chat", "catch", "catchup",
    "weekly", "daily", "monthly", "standup", "invite", "zoom", "teams", "huddle", "touchbase", "update",
    "review", "discussion", "session", "our", "your", "you", "about", "into", "re",
];

/// The most keywords a title contributes.
pub const MAX_TITLE_WORDS: usize = 4;

/// A title's meaningful words, lower case, each once, in the order they appear: not a stop word,
/// and longer than two letters.
pub fn title_keywords(title: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in title.split(|c: char| !c.is_alphanumeric()) {
        let w = word.to_lowercase();
        if w.chars().count() <= 2 || STOP_WORDS.contains(&w.as_str()) || out.contains(&w) {
            continue;
        }
        out.push(w);
        if out.len() == MAX_TITLE_WORDS {
            break;
        }
    }
    out
}

/// A name for someone on an invitation: the name they were given, or, when there is none, the
/// part of their address before the `@` read as words ("maya.chen" is "Maya Chen").
pub fn display_name(name: &str, email: &str) -> String {
    let name = name.trim();
    if !name.is_empty() {
        return name.to_string();
    }
    let local = email.split('@').next().unwrap_or("");
    local
        .split(['.', '_', '-', '+'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The ways one person is written in a note or a promise: the whole name when it has more than
/// one word, and the first word when it is long enough to mean something.
pub fn person_terms(name: &str) -> Vec<String> {
    let name = name.trim();
    let mut out = Vec::new();
    if name.split_whitespace().count() > 1 {
        out.push(name.to_string());
    }
    if let Some(first) = name.split_whitespace().next() {
        if first.chars().count() >= 3 && !out.iter().any(|t| t.eq_ignore_ascii_case(first)) {
            out.push(first.to_string());
        }
    }
    out
}

/// The first word of a name, for a line that says "You told Ravi".
pub fn short_name(name: &str) -> String {
    name.split_whitespace().next().unwrap_or(name).to_string()
}
