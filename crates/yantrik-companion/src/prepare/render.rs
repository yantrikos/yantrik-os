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

/// The text of a `.ydeck` file for `slides`, in the Presentation app's own format (version 1:
/// `{version, title, theme, slides: [{title, body, notes, layout}]}`; layout 0 is the title slide,
/// 1 a titled body). Written by the job itself into its folder, so no open deck is touched.
pub fn ydeck(title: &str, slides: &[Slide]) -> String {
    let slides: Vec<serde_json::Value> = slides
        .iter()
        .enumerate()
        .map(|(i, s)| serde_json::json!({ "title": s.title, "body": slide_body(s), "notes": "", "layout": if i == 0 { 0 } else { 1 } }))
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({ "version": 1, "title": title, "theme": 0, "slides": slides })).unwrap_or_default()
}

/// Markdown for the folder's `sources.md`.
pub fn sources_md(item: &str, sources: &[Source], none_because: Option<&str>) -> String {
    let mut s = format!("# Sources for {item}\n\nPages read for this preparation. Their text is not kept here.\n\n");
    if sources.is_empty() {
        s.push_str(&format!("No web sources: {}.\n", none_because.unwrap_or("none were found")));
    }
    for x in sources {
        s.push_str(&format!("- [{}]({})\n", x.title.replace(['[', ']'], ""), x.url));
    }
    s
}

/// The first blank the writer left in square brackets (`[Product Name]`, `[website]`), if any.
/// A markdown link `[x](y)`, a checkbox `[ ]` / `[x]` and a lone letter are not blanks.
pub fn placeholder(text: &str) -> Option<String> {
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else { return None };
        let inner = &after[..close];
        let linked = after[close + 1..].starts_with('(');
        if !linked && inner.len() <= 40 && !inner.contains('\n') && inner.chars().filter(|c| c.is_alphabetic()).count() >= 2 {
            return Some(format!("[{inner}]"));
        }
        rest = &after[close + 1..];
    }
    None
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
    fn blanks_in_brackets_are_found_and_links_and_checkboxes_are_not() {
        assert_eq!(placeholder("Hello from [Product Name]."), Some("[Product Name]".into()));
        assert_eq!(placeholder("Visit [website] now"), Some("[website]".into()));
        assert_eq!(placeholder("See [the guide](https://example.com/g) and - [ ] item - [x] done"), None);
        assert_eq!(placeholder("Plain text, no blanks."), None);
    }

    #[test]
    fn sections_split_on_markers() {
        let t = "SHOT LIST:\n1. a\n2. b\nSCRIPT:\nHello.";
        assert_eq!(section(t, "SHOT LIST:", &["SCRIPT:"]), Some("1. a\n2. b"));
        assert_eq!(section(t, "SCRIPT:", &["SHOT LIST:"]), Some("Hello."));
        assert_eq!(section(t, "NOPE:", &[]), None);
    }
}
