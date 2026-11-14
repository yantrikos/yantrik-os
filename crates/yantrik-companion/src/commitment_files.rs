//! The files a commitment names, and what kind of thing each one is.
//!
//! "I'm sending Maya the Q4 deck on Thursday. The numbers are in the budget spreadsheet." names two
//! files. The one being sent is what the email carries (`attach:`); the other is what the person
//! needs open while they check it (`file:`). Both are stored on the commitment, in its
//! `related_entities`, as `role:label`, so nothing new is added to the table.
//!
//! A label is the person's own words ("Q4 deck"). Which file on disk it is, is decided later, by
//! `commitment_prep`, with the tools the companion already has; this module only reads words.

use std::sync::OnceLock;

use regex::Regex;

/// What a named file is: decides the app that opens it and the extensions it may be found as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Deck,
    Spreadsheet,
    Document,
}

/// How the commitment uses a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The thing being sent: the draft carries it.
    Attach,
    /// Something the commitment points at: found for the person, who opens it. Not sent.
    Reference,
}

/// One file as the commitment names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedFile {
    pub role: Role,
    /// The person's words, e.g. "Q4 deck".
    pub label: String,
}

const ATTACH: &str = "attach:";
const REFERENCE: &str = "file:";

impl FileKind {
    /// The kind a noun names ("deck", "spreadsheet"), if it names one.
    pub fn from_noun(noun: &str) -> Option<Self> {
        match noun.to_lowercase().as_str() {
            "deck" | "slides" | "presentation" => Some(Self::Deck),
            "spreadsheet" | "sheet" | "workbook" => Some(Self::Spreadsheet),
            "document" | "doc" | "report" | "proposal" | "memo" | "contract" => Some(Self::Document),
            _ => None,
        }
    }

    /// The Yantrik app that opens this kind.
    pub fn app(self) -> &'static str {
        match self {
            Self::Deck => "presentation",
            Self::Spreadsheet => "spreadsheet",
            Self::Document => "document-editor",
        }
    }

    /// Every extension a file of this kind may be found with, best first.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Deck => &["ydeck", "pptx", "odp", "key", "pdf"],
            Self::Spreadsheet => &["csv", "xlsx", "ods"],
            Self::Document => &["md", "docx", "odt", "pdf", "txt"],
        }
    }

    /// The one extension the app opens: a file found with another is attachable, not openable.
    pub fn openable_extension(self) -> &'static str {
        match self {
            Self::Deck => "ydeck",
            Self::Spreadsheet => "csv",
            Self::Document => "md",
        }
    }
}

impl NamedFile {
    pub fn kind(&self) -> Option<FileKind> {
        self.label.split_whitespace().last().and_then(FileKind::from_noun)
    }

    /// The words that tell this file from another of its kind: "Q4" in "Q4 deck".
    pub fn distinguishing_words(&self) -> Vec<String> {
        let mut words: Vec<String> = self.label.split_whitespace().map(str::to_lowercase).collect();
        words.pop();
        words
    }

    /// As stored in `related_entities`.
    pub fn encode(&self) -> String {
        let role = match self.role {
            Role::Attach => ATTACH,
            Role::Reference => REFERENCE,
        };
        format!("{role}{}", self.label)
    }

    pub fn decode(entity: &str) -> Option<Self> {
        let (role, label) = if let Some(label) = entity.strip_prefix(ATTACH) {
            (Role::Attach, label)
        } else {
            (Role::Reference, entity.strip_prefix(REFERENCE)?)
        };
        let file = Self { role, label: label.trim().to_string() };
        file.kind().map(|_| file)
    }
}

/// The files a commitment names, read back from its stored entities.
pub fn named_files(related_entities: &[String]) -> Vec<NamedFile> {
    related_entities.iter().filter_map(|e| NamedFile::decode(e)).collect()
}

fn re_file() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:the|my|our)\s+((?:[a-z0-9][\w-]*\s+){0,3}?(?:deck|slides|presentation|spreadsheet|sheet|workbook|document|doc|report|proposal|memo|contract))\b",
        )
        .expect("file pattern")
    })
}

/// The verbs of getting a thing to someone, as a regex alternation: shared with the extractor, so
/// what it takes as "sending" and what decides a file is attached are one list.
pub const SEND_VERBS: &str = "send(?:ing)?|email(?:ing)?|forward(?:ing)?|giv(?:e|ing)";

/// "Share", which also takes "with" before its person, so it is listed apart.
pub const SHARE_VERBS: &str = "shar(?:e|ing)";

fn re_sends() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(&format!(r"(?i)\b(?:{SEND_VERBS}|{SHARE_VERBS})\b")).expect("send pattern"))
}

/// The files one sentence names. In a sentence about sending, they are what is sent.
pub fn in_sentence(sentence: &str) -> Vec<NamedFile> {
    let role = if re_sends().is_match(sentence) { Role::Attach } else { Role::Reference };
    let mut found: Vec<NamedFile> = Vec::new();
    for caps in re_file().captures_iter(sentence) {
        let label = caps[1].split_whitespace().collect::<Vec<_>>().join(" ");
        if !found.iter().any(|f| f.label.eq_ignore_ascii_case(&label)) {
            found.push(NamedFile { role, label });
        }
    }
    found
}

/// What a commitment from `sentence` names: its own files with their roles, then the files named
/// in the sentence right after it, which it points at ("The numbers are in the budget
/// spreadsheet"). Nothing from further off: a file named in another sentence is that sentence's.
pub fn for_commitment(sentence: &str, next: Option<&str>) -> Vec<String> {
    let mut files = in_sentence(sentence);
    for file in next.map(in_sentence).unwrap_or_default() {
        if !files.iter().any(|f| f.label.eq_ignore_ascii_case(&file.label)) {
            files.push(NamedFile { role: Role::Reference, label: file.label });
        }
    }
    files.iter().map(NamedFile::encode).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thing_sent_is_attached_and_the_thing_pointed_at_is_a_reference() {
        let sent = in_sentence("I'm sending Maya the Q4 deck on Thursday");
        assert_eq!(sent, vec![NamedFile { role: Role::Attach, label: "Q4 deck".into() }]);
        let pointed = in_sentence("The numbers are in the budget spreadsheet");
        assert_eq!(pointed, vec![NamedFile { role: Role::Reference, label: "budget spreadsheet".into() }]);
    }

    #[test]
    fn a_label_round_trips_through_the_stored_entity() {
        let file = NamedFile { role: Role::Attach, label: "Q4 deck".into() };
        assert_eq!(file.encode(), "attach:Q4 deck");
        assert_eq!(NamedFile::decode("attach:Q4 deck"), Some(file));
        assert_eq!(NamedFile::decode("person:Maya"), None, "other entities are not files");
        assert_eq!(NamedFile::decode("file:the weather"), None, "no kind, no file");
    }

    #[test]
    fn the_distinguishing_words_leave_out_the_noun() {
        let file = NamedFile { role: Role::Reference, label: "Q4 budget spreadsheet".into() };
        assert_eq!(file.distinguishing_words(), vec!["q4", "budget"]);
        assert_eq!(file.kind(), Some(FileKind::Spreadsheet));
    }
}
