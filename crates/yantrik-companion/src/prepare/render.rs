//! Turning what the writer wrote into the parts a job leaves: plain code, so the shape is fixed
//! whatever the model said (#1416, slice 5).

use super::sources::Source;

/// The fewest and most slides of a first deck.
pub const SLIDES: (usize, usize) = (5, 8);

/// One slide: a title and up to five bullets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slide {
    pub title: String,
    pub bullets: Vec<String>,
}

fn tidy(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().trim_start_matches(|c: char| "#-*• ".contains(c)).trim().chars().take(max).collect()
}

/// Slides from an outline: a line starting with `#` begins one, `-` or `*` lines are its bullets.
/// More than 8 are cut, and a deck of fewer than 5 is kept as it is (the report says so).
pub fn slides(outline: &str) -> Vec<Slide> {
    let mut out: Vec<Slide> = Vec::new();
    for line in outline.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line.starts_with('#') {
            let t = tidy(line, 80);
            if !t.is_empty() {
                out.push(Slide { title: t, bullets: Vec::new() });
            }
        } else if let (Some(s), true) = (out.last_mut(), line.starts_with(['-', '*', '•'])) {
            let b = tidy(line, 140);
            if !b.is_empty() && s.bullets.len() < 5 {
                s.bullets.push(b);
            }
        }
    }
    out.truncate(SLIDES.1);
    out
}

/// The text of a slide's body.
pub fn slide_body(s: &Slide) -> String {
    s.bullets.iter().map(|b| format!("- {b}")).collect::<Vec<_>>().join("\n")
}

/// Markdown for the folder's `sources.md`.
pub fn sources_md(item: &str, sources: &[Source]) -> String {
    let mut s = format!("# Sources for {item}\n\nPages read for this preparation. Their text is not kept here.\n\n");
    for x in sources {
        s.push_str(&format!("- [{}]({})\n", x.title.replace(['[', ']'], ""), x.url));
    }
    s
}

/// The note "Prepared: <item>": the summary, the sources, and the titles of the person's own open
/// items (their note itself is never edited).
pub fn note_body(summary: &str, sources: &[Source], open_items: &[String], note_title: Option<&str>) -> String {
    let mut s = format!("{}\n\n## Sources\n", summary.trim());
    if sources.is_empty() {
        s.push_str("(none read)\n");
    }
    for x in sources {
        s.push_str(&format!("- {} \u{2014} {}\n", x.title, x.url));
    }
    if !open_items.is_empty() {
        s.push_str(&format!("\n## Your open items in \u{201c}{}\u{201d}\n", note_title.unwrap_or("your note")));
        for i in open_items.iter().take(12) {
            s.push_str(&format!("- [ ] {i}\n"));
        }
    }
    s.push_str("\nPrepared by Yantrik OS as a draft. Nothing was sent or changed.\n");
    s
}

/// The part of `text` after a line that is exactly `marker` (case aside), up to the next marker.
pub fn section<'a>(text: &'a str, marker: &str, others: &[&str]) -> Option<&'a str> {
    let lower = text.to_lowercase();
    let start = lower.find(&marker.to_lowercase())? + marker.len();
    let end = others.iter().filter_map(|o| lower[start..].find(&o.to_lowercase())).min().map_or(text.len(), |e| start + e);
    Some(text[start..end].trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outline_becomes_at_most_eight_slides_of_five_bullets() {
        let mut o = String::new();
        for i in 0..11 {
            o.push_str(&format!("# Slide {i}\n"));
            for b in 0..7 {
                o.push_str(&format!("- point {b}\n"));
            }
        }
        let s = slides(&o);
        assert_eq!(s.len(), 8);
        assert!(s.iter().all(|x| x.bullets.len() == 5));
        assert_eq!(s[0].title, "Slide 0");
        assert!(slides("no outline at all").is_empty());
    }

    #[test]
    fn the_note_names_the_open_items_by_title_and_says_nothing_was_sent() {
        let src = [Source { title: "A page".into(), url: "https://example.com/a".into() }];
        let n = note_body("Summary.", &src, &["Demo video (not started)".into()], Some("Launch prep"));
        assert!(n.contains("https://example.com/a") && n.contains("- [ ] Demo video (not started)") && n.contains("Nothing was sent or changed"));
    }

    #[test]
    fn sections_split_on_markers() {
        let t = "SHOT LIST:\n1. a\n2. b\nSCRIPT:\nHello.";
        assert_eq!(section(t, "SHOT LIST:", &["SCRIPT:"]), Some("1. a\n2. b"));
        assert_eq!(section(t, "SCRIPT:", &["SHOT LIST:"]), Some("Hello."));
        assert_eq!(section(t, "NOPE:", &[]), None);
    }
}
