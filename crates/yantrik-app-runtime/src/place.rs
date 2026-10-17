//! Where a file is, and where it came from, as a list row says it: "in Downloads", "in Pictures ›
//! Studio", "from hn.algolia.com".
//!
//! Downloads and Studio printed every row's full path (`/home/yantrik/hn_research/q_mcp`,
//! `~/Pictures/Studio/2026-09-23/064441-32278.png`) and Downloads a cut-off URL; the sweep of 17
//! October (design/ui-sweep-2026-10-17.md, root cause 5) found the same thing in both, because
//! each app wrote its own row. The path is still true and still shown, in the row's Details. A row's
//! own line is these words, from this one place, so the next app with a list of files says it the
//! same way.

use std::path::{Component, Path};

/// The folders a person knows by name, as their home directory has them.
const KNOWN: &[&str] = &["Desktop", "Documents", "Downloads", "Music", "Pictures", "Videos", "Templates", "Public"];

/// Where a folder is, in words. `home` is the person's home directory.
///
/// - `~/Downloads` → "in Downloads"; `~/Pictures/Studio/2026-09-23` → "in Pictures › Studio"
///   (a folder named for a date says nothing a row's own date does not);
/// - `~` → "in your home folder"; `~/hn_research/q_mcp` → "in the q_mcp folder";
/// - outside home: `/srv/media` → "in the media folder"; `/` or nothing → "on this computer".
pub fn place_of(dir: &Path, home: &Path) -> String {
    if let Ok(rest) = dir.strip_prefix(home) {
        let parts: Vec<String> = rest
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_string_lossy().to_string()),
                _ => None,
            })
            .collect();
        return match parts.as_slice() {
            [] => "in your home folder".to_string(),
            [first, rest @ ..] if KNOWN.contains(&first.as_str()) => match rest.iter().find(|p| !dated(p)) {
                Some(next) => format!("in {first} \u{203a} {next}"),
                None => format!("in {first}"),
            },
            [.., last] => format!("in the {last} folder"),
        };
    }
    match dir.file_name() {
        Some(name) if !name.is_empty() => format!("in the {} folder", name.to_string_lossy()),
        _ => "on this computer".to_string(),
    }
}

/// The site a URL is from, without the scheme, the path or a `www.`: "hn.algolia.com". Empty for
/// something that is not a URL.
pub fn site_of(url: &str) -> String {
    let Some((_, rest)) = url.trim().split_once("://") else { return String::new() };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

/// The person's home directory, as `place_of` wants it.
pub fn home() -> std::path::PathBuf {
    std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from("/root"))
}

/// "2026-09-23", "20260923": a folder named for a day.
fn dated(name: &str) -> bool {
    let digits = name.chars().filter(|c| c.is_ascii_digit()).count();
    digits >= 6 && name.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_said_by_name_not_by_path() {
        let home = Path::new("/home/yantrik");
        let at = |p: &str| place_of(Path::new(p), home);
        assert_eq!(at("/home/yantrik/Downloads"), "in Downloads");
        assert_eq!(at("/home/yantrik/Pictures/Studio/2026-09-23"), "in Pictures \u{203a} Studio");
        assert_eq!(at("/home/yantrik/hn_research/q_mcp"), "in the q_mcp folder");
        assert_eq!(at("/home/yantrik"), "in your home folder");
        assert_eq!(at("/srv/media"), "in the media folder");
        assert_eq!(at("/"), "on this computer");
        for p in ["/home/yantrik/Downloads", "/home/yantrik/hn_research/q_mcp", "/srv/media"] {
            assert!(!at(p).contains('/'), "{p} is said without a slash");
        }
    }

    #[test]
    fn a_url_is_said_by_its_site() {
        assert_eq!(site_of("https://hn.algolia.com/api/v1/search_by_date?query=agentic&t=1"), "hn.algolia.com");
        assert_eq!(site_of("https://www.example.org/file.zip"), "example.org");
        assert_eq!(site_of("http://user:pw@10.0.0.2:8000/x"), "10.0.0.2:8000");
        assert_eq!(site_of("not a url"), "");
    }
}
