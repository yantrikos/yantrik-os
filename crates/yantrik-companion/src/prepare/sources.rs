//! Sources: the pages a job read, kept as a URL and a title. Fetched text is untrusted and is only
//! ever handed to the writer as quoted material; only these two fields are kept (#1416, slice 5).

use super::boundary::https_host;

/// At most this many sources a job keeps.
pub const MAX_SOURCES: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub title: String,
    pub url: String,
}

fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().trim_matches(|c: char| "*#-—–[]():| ".contains(c)).chars().take(max).collect()
}

/// The pages a search answer names, in order, without repeats: every `https://` word on a public
/// name, with the words before it on its line (or the line above) as the title.
pub fn from_search(answer: &str, have: &[Source]) -> Vec<Source> {
    let mut out: Vec<Source> = Vec::new();
    let mut prev = "";
    for line in answer.lines() {
        if let Some(at) = line.find("https://") {
            let url: String = line[at..].chars().take_while(|c| !c.is_whitespace() && !"<>\")]".contains(*c)).collect();
            let url = url.trim_end_matches(['.', ',', ';']).to_string();
            let before = clean(&line[..at], 120);
            let title = if before.chars().filter(|c| c.is_alphabetic()).count() >= 4 { before } else { clean(prev, 120) };
            let seen = |s: &Source| s.url == url;
            if https_host(&url).is_some() && !out.iter().any(seen) && !have.iter().any(seen) {
                let title = if title.is_empty() { https_host(&url).unwrap_or("page").to_string() } else { title };
                out.push(Source { title, url });
            }
        }
        prev = line;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_titles_come_out_and_only_public_https_ones() {
        let answer = "1. How to record a demo — https://example.com/demo\n   snippet text\nShot lists https://example.org/shots.\nhttp://plain.example/x\nhttps://localhost/admin\nhttps://10.0.0.1/a\nhttps://user:pw@example.net/x\nhttps://example.com/demo";
        let got = from_search(answer, &[]);
        let urls: Vec<&str> = got.iter().map(|s| s.url.as_str()).collect();
        assert_eq!(urls, ["https://example.com/demo", "https://example.org/shots"]);
        assert_eq!(got[0].title, "1. How to record a demo");
        assert!(from_search(answer, &got).is_empty(), "no repeats of what is held");
    }
}
