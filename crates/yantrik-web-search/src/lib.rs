//! The person's web search service, as one setting.
//!
//! Minds and tools search the web. Keyless DuckDuckGo works until a burst of research gets it
//! CAPTCHA'd or rate-limited, and then a run comes back empty. A SearXNG the person runs
//! themselves does not have that problem. Before this crate the address lived in three places
//! that did not know about each other (an env var in the companion tools, an env file in the
//! Mind, nothing at all for other harnesses); now it is the `web_search` section of the shell's
//! `~/.config/yantrik/settings.yaml`, and everything that searches reads it from here:
//!
//! - Settings writes it, after a test that found results ([`client::probe`]);
//! - the companion tools search through it ([`client::search`]) and say when they fell back;
//! - the harness host tells every attached harness about it, at attach and on change.
//!
//! The address is checked in one place, [`address::check`], and nothing reads a URL out of the
//! file without going through it.

pub mod address;
pub mod client;

use serde::{Deserialize, Serialize};

pub use address::{check, Place, SearxUrl};

/// Which service answers a search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Service {
    /// DuckDuckGo's HTML page, keyless. Can be rate-limited during heavy research.
    #[default]
    Builtin,
    /// The person's own SearXNG, at [`WebSearch::searxng_url`].
    Searxng,
}

impl Service {
    pub fn as_str(self) -> &'static str {
        match self {
            Service::Builtin => "builtin",
            Service::Searxng => "searxng",
        }
    }
}

/// The `web_search` section of the shell's settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearch {
    #[serde(default)]
    pub service: Service,
    /// Kept when the person switches back to built-in, so switching again does not mean typing
    /// it again. Read only while `service` is `searxng`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub searxng_url: String,
    /// When it was last saved, RFC 3339, for the "Saved … at …" line.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub saved_at: String,
}

/// What a search should use, once the setting has been read and its address checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Builtin,
    Searxng(SearxUrl),
    /// SearXNG is chosen, but the address in the file does not pass [`check`]: someone edited it
    /// by hand. Treated as built-in, and said, never quietly fixed up.
    Invalid { url: String, why: String },
}

impl WebSearch {
    /// What this setting means for a search. An address is only used once it has passed
    /// [`check`], however it got into the file.
    pub fn target(&self) -> Target {
        match self.service {
            Service::Builtin => Target::Builtin,
            Service::Searxng => match check(&self.searxng_url) {
                Ok(url) => Target::Searxng(url),
                Err(why) => Target::Invalid { url: self.searxng_url.clone(), why },
            },
        }
    }
}

/// `~/.config/yantrik/settings.yaml`, the shell's settings file, which holds this section.
pub fn settings_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    std::path::Path::new(&home).join(".config/yantrik/settings.yaml")
}

/// The `web_search` section out of a settings file's text. Anything that is not one — no file,
/// no section, a section that does not parse — is built-in: a broken setting must not send a
/// search somewhere the person did not choose.
pub fn from_settings_yaml(text: &str) -> WebSearch {
    #[derive(Deserialize)]
    struct Section {
        #[serde(default)]
        web_search: Option<WebSearch>,
    }
    serde_yaml::from_str::<Section>(text).ok().and_then(|s| s.web_search).unwrap_or_default()
}

/// The setting as saved, read from the settings file. Cheap enough to call per search: one small
/// file, at most 256 KiB, as the shell's own store bounds it.
pub fn load() -> WebSearch {
    load_from(&settings_path())
}

pub fn load_from(path: &std::path::Path) -> WebSearch {
    use std::io::Read;
    let mut text = String::new();
    match std::fs::File::open(path) {
        Ok(f) => {
            if f.take(256 * 1024).read_to_string(&mut text).is_err() {
                return WebSearch::default();
            }
        }
        Err(_) => return WebSearch::default(),
    }
    from_settings_yaml(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_section_is_builtin_and_a_section_reads_back() {
        assert_eq!(from_settings_yaml("dark_mode: true\n"), WebSearch::default());
        assert_eq!(from_settings_yaml("not: [yaml"), WebSearch::default());
        let ws = from_settings_yaml(
            "dark_mode: true\nweb_search:\n  service: searxng\n  searxng_url: http://192.168.4.42:8888\n",
        );
        assert_eq!(ws.service, Service::Searxng);
        assert_eq!(ws.searxng_url, "http://192.168.4.42:8888");
        match ws.target() {
            Target::Searxng(u) => assert_eq!(u.url, "http://192.168.4.42:8888"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_hand_edited_address_that_fails_the_check_is_not_used() {
        let ws = WebSearch { service: Service::Searxng, searxng_url: "http://search.example.com".into(), saved_at: String::new() };
        assert!(matches!(ws.target(), Target::Invalid { .. }));
        let builtin = WebSearch { service: Service::Builtin, searxng_url: "http://127.0.0.1:8888".into(), saved_at: String::new() };
        assert_eq!(builtin.target(), Target::Builtin, "a kept address is not used while built-in is chosen");
    }
}
