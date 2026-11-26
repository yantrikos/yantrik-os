//! What a preparation makes, chosen by plain code from the item's own words (#1416, slice 5).
//! The model never chooses: it writes the content of the parts this picks.

/// The kind of work an item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A first deck: 5 to 8 slides, titles and bullets.
    Deck,
    /// A shot list and a 60 s script, as text files.
    Video,
    /// Research and a notes file only.
    Notes,
}

const VIDEO_WORDS: [&str; 6] = ["video", "demo", "record", "recording", "screencast", "film"];
const DECK_WORDS: [&str; 9] = ["talk", "slides", "slide", "deck", "ppt", "pptx", "pitch", "presentation", "launch"];

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Deck => "deck",
            Kind::Video => "video",
            Kind::Notes => "notes",
        }
    }
}

fn words(title: &str) -> Vec<String> {
    title.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// The kind of an item, from its title only (a note's words say what the person is busy with, not
/// what this item is). Video words win: "Record the 60 s demo video" for a launch is a video.
pub fn kind_of(title: &str) -> Kind {
    let w = words(title);
    let has = |set: &[&str]| w.iter().any(|x| set.contains(&x.as_str()));
    if has(&VIDEO_WORDS) {
        Kind::Video
    } else if has(&DECK_WORDS) {
        Kind::Deck
    } else {
        Kind::Notes
    }
}

/// "Record the 60 s demo video" becomes `record-the-60-s-demo-video`: letters and digits, single
/// hyphens, at most 48 characters, never empty.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for w in words(title) {
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&w);
    }
    let mut out: String = out.chars().take(48).collect();
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() { "item".into() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_comes_from_the_items_words() {
        assert_eq!(kind_of("Record the 60 s demo video"), Kind::Video);
        assert_eq!(kind_of("Launch Yantrik OS on Show HN"), Kind::Deck);
        assert_eq!(kind_of("Pitch to the board: slides"), Kind::Deck);
        assert_eq!(kind_of("Talk at the meetup"), Kind::Deck);
        assert_eq!(kind_of("Renew the car insurance"), Kind::Notes);
        assert_eq!(kind_of("Videography course fees"), Kind::Notes, "whole words only");
    }

    #[test]
    fn the_slug_is_the_titles_words() {
        assert_eq!(slug("Record the 60 s demo video"), "record-the-60-s-demo-video");
        assert_eq!(slug("../../etc/passwd"), "etc-passwd");
        assert_eq!(slug("???"), "item");
        assert!(slug(&"word ".repeat(40)).len() <= 48);
    }
}
