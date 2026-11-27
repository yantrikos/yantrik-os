//! The person's web search, as one setting (#1439).
//!
//! There is one switch. With the person's SearXNG **on**, it answers every search, whoever asks,
//! and nothing else is tried: a SearXNG that does not answer is an error, never a quiet detour
//! through another engine, because the person chose where their search terms go. With it **off**
//! (the default) the OS's own headless browser does the searching; that is the companion tools'
//! business and not this crate's.
//!
//! The setting lives in one file, `~/.config/yantrik/web_search.yaml`, and everything that searches
//! reads it from here ([`load`]). Settings writes it ([`save`]) after a test that found results
//! ([`client::probe`]). The address is checked in one place, [`address::check`], and nothing reads
//! a URL out of the file without going through it.

pub mod address;
pub mod client;
pub mod render;

use serde::{Deserialize, Serialize};

pub use address::{check, Place, SearxUrl};

/// Which service answers a search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Service {
    /// No SearXNG: the OS's own browser searches. (`builtin` in files an earlier build wrote.)
    #[default]
    #[serde(alias = "builtin")]
    Off,
    /// The person's own SearXNG, at [`WebSearch::searxng_url`].
    Searxng,
}

impl Service {
    pub fn as_str(self) -> &'static str {
        match self {
            Service::Off => "off",
            Service::Searxng => "searxng",
        }
    }
}

/// The saved setting.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearch {
    #[serde(default)]
    pub service: Service,
    /// Kept when the person switches SearXNG off, so switching it on again does not mean typing
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
    /// SearXNG is off: the OS's own browser.
    Browser,
    Searxng(SearxUrl),
    /// SearXNG is on, but the address in the file does not pass [`check`]: someone edited it by
    /// hand. A search is an error that says so; it is never sent anywhere else.
    Invalid { why: String },
}

impl WebSearch {
    /// What this setting means for a search. An address is only used once it has passed [`check`],
    /// however it got into the file.
    pub fn target(&self) -> Target {
        match self.service {
            Service::Off => Target::Browser,
            Service::Searxng => match check(&self.searxng_url) {
                Ok(url) => Target::Searxng(url),
                Err(why) => Target::Invalid { why },
            },
        }
    }
}

/// `~/.config/yantrik/web_search.yaml`: the one store of the setting.
pub fn store_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    std::path::Path::new(&home).join(".config/yantrik/web_search.yaml")
}

/// The setting out of a file's text. Anything that is not one (no file, a file that does not
/// parse) is "off": a broken file must not send a search somewhere the person did not choose.
pub fn from_yaml(text: &str) -> WebSearch {
    serde_yaml::from_str::<WebSearch>(text).unwrap_or_default()
}

/// The setting as saved. Cheap enough to call per search: one small file, at most 16 KiB.
pub fn load() -> WebSearch {
    load_from(&store_path())
}

pub fn load_from(path: &std::path::Path) -> WebSearch {
    use std::io::Read;
    let mut text = String::new();
    match std::fs::File::open(path) {
        Ok(f) => {
            if f.take(16 * 1024).read_to_string(&mut text).is_err() {
                return WebSearch::default();
            }
        }
        Err(_) => return WebSearch::default(),
    }
    from_yaml(&text)
}

/// Write the setting.
pub fn save(setting: &WebSearch) -> Result<(), String> {
    save_to(&store_path(), setting)
}

/// [`save`] to a named file: written beside it and renamed over it, so a reader sees the old
/// setting or the new one and never half of either.
pub fn save_to(path: &std::path::Path, setting: &WebSearch) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let yaml = serde_yaml::to_string(setting).map_err(|_| "cannot serialise the setting".to_string())?;
    let dir = path.parent().ok_or("no folder for the setting")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let tmp = path.with_extension("yaml.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| format!("cannot write the setting: {e}"))?;
    file.write_all(yaml.as_bytes()).and_then(|()| file.sync_all()).map_err(|e| format!("cannot write the setting: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("cannot write the setting: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setting(service: Service, url: &str) -> WebSearch {
        WebSearch { service, searxng_url: url.into(), saved_at: String::new() }
    }

    #[test]
    fn no_file_is_off_and_a_file_reads_back() {
        assert_eq!(from_yaml(""), WebSearch::default());
        assert_eq!(from_yaml("not: [yaml"), WebSearch::default());
        assert_eq!(WebSearch::default().service, Service::Off);
        let ws = from_yaml("service: searxng\nsearxng_url: http://192.168.4.42:8888\n");
        assert_eq!(ws.service, Service::Searxng);
        match ws.target() {
            Target::Searxng(u) => assert_eq!(u.url, "http://192.168.4.42:8888"),
            other => panic!("{other:?}"),
        }
        assert_eq!(from_yaml("service: builtin\n").service, Service::Off, "an earlier build's word for off");
    }

    #[test]
    fn a_hand_edited_address_that_fails_the_check_is_not_used_and_not_replaced() {
        let ws = setting(Service::Searxng, "http://search.example.com");
        assert!(matches!(ws.target(), Target::Invalid { .. }), "an error, never the browser");
        assert_eq!(setting(Service::Off, "http://127.0.0.1:8888").target(), Target::Browser, "a kept address is not used while off");
    }

    #[test]
    fn it_saves_and_reads_back_privately() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("yws-{}", std::process::id()));
        let path = dir.join("nested/web_search.yaml");
        let ws = setting(Service::Searxng, "http://127.0.0.1:8888");
        save_to(&path, &ws).unwrap();
        assert_eq!(load_from(&path), ws);
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        save_to(&path, &setting(Service::Off, "http://127.0.0.1:8888")).unwrap();
        assert_eq!(load_from(&path).service, Service::Off);
        assert!(!path.with_extension("yaml.tmp").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
