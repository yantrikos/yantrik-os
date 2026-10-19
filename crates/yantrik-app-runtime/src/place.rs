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
/// something that is not a URL, and for one `address_of` would not draw: the site is read from
/// the drawable host, so no part of its sign-in details is ever the site.
pub fn site_of(url: &str) -> String {
    if !url.contains("://") {
        return String::new();
    }
    let host = shown_host(url);
    host.strip_prefix("www.").map(str::to_string).unwrap_or(host)
}

// A URL as it may be drawn, and someone else's line with every address in it drawn that way: the
// one rule for the whole workspace, in a crate of its own so the harness host, yantrik-ml and the
// gateway use it too (review of #856).
pub use yantrik_shown_address::{address_of, shown_host, without_sign_in, NOT_SHOWN};

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
        // The site is read from the drawable address: no part of the sign-in details is a site.
        assert_eq!(site_of("https://user:pw@dl.example/f.zip?api-key=x#f"), "dl.example");
        for odd in ["https://user:p/w@dl.example/f.zip", "https://user:p?w@dl.example", "https://evil.example\\@dl.example/f"] {
            assert_eq!(site_of(odd), "", "{odd}");
        }
    }

    #[test]
    fn a_line_someone_else_wrote_is_drawn_without_the_sign_in_of_any_address_in_it() {
        assert_eq!(without_sign_in("Hermes 0.21.5 \u{b7} picked"), "Hermes 0.21.5 \u{b7} picked");
        assert_eq!(
            without_sign_in("qwen3.5:27b \u{b7} https://user:pw@aig.example/v1?api-key=x#f"),
            "qwen3.5:27b \u{b7} https://aig.example/v1"
        );
        assert_eq!(without_sign_in("x \u{b7} https://user:p/w@aig.example/v1"), "x \u{b7} (address not shown)");
        assert_eq!(without_sign_in("on u@127.0.0.1:11434"), "on 127.0.0.1:11434");
        for line in ["https://user:p/w@[::1]:4000/v1 now", "user:p/w@aig.example", "a\tuser:pw@[fd00::1]:8443"] {
            let shown = without_sign_in(line);
            for secret in ["user", "pw", "p/w", "api-key"] {
                assert!(!shown.contains(secret), "{line} shows {secret}: {shown}");
            }
        }
    }

    #[test]
    fn an_address_is_drawn_without_its_sign_in_query_or_fragment() {
        assert_eq!(address_of("https://user:pw@aig.example/v1?api-key=sk-123#frag"), "https://aig.example/v1");
        assert_eq!(address_of("http://127.0.0.1:8341/v1"), "http://127.0.0.1:8341/v1");
        assert_eq!(address_of("localhost:11434"), "localhost:11434");
        assert_eq!(address_of("https://key@[::1]:4000"), "https://[::1]:4000");
        // A key holding a character the split stops at is not guessed around: nothing is drawn.
        for odd in ["https://user:p/w@aig.example/v1", "https://user:p?w@aig.example", "https://user:p#w@aig.example", "https://@/v1", "https://evil.example\\@api.x.ai/v1"] {
            assert_eq!(address_of(odd), "", "{odd}");
        }
        for url in ["https://user:pw@aig.example/v1?api-key=sk-123#frag", "https://user:p/w@aig.example/v1"] {
            let shown = address_of(url);
            for secret in ["pw@", "user", "api-key", "sk-123", "frag", "p/w"] {
                assert!(!shown.contains(secret), "{url} shows {secret}: {shown}");
            }
        }
    }
}
