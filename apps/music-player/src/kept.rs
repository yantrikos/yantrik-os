//! What Music keeps between runs: the library folders and the volume. The library itself is not
//! kept: it is whatever the folders hold, and a scan on start reads it again.
//!
//! Before the framework this was `~/.config/yantrik/music.json`, written by this app alone and
//! read by nothing else, in the shape `{"folders": [...], "volume": n}`. The framework keeps the
//! settings in the app's own file now (`<config>/yantrik/apps/music/settings.json`); the old
//! file is named as this app's (`FORMER_FILE`), brought across once in that shape
//! (`Settings::LEGACY_KEYS`), and left exactly where it was as the backup.

use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::json;
use yantrik_app::Settings;

/// The volume a first run starts at.
pub const DEFAULT_VOLUME: f64 = 80.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Kept {
    /// The library folders, as the person listed them. `None` until they first change the list:
    /// a first run lists `~/Music`, if there is one, without writing anything.
    #[serde(default)]
    pub folders: Option<Vec<String>>,
    /// 0 to 100: what mpv starts at.
    #[serde(default = "default_volume")]
    pub volume: f64,
}

fn default_volume() -> f64 {
    DEFAULT_VOLUME
}

impl Default for Kept {
    fn default() -> Kept {
        Kept { folders: None, volume: DEFAULT_VOLUME }
    }
}

impl Settings for Kept {
    const SCHEMA: u32 = 1;
    const FORMER_FILE: Option<&'static str> = Some("music.json");
    /// The keys the old file began with: the framework judges its first bytes against these.
    const LEGACY_KEYS: &'static [&'static str] = &["folders", "volume"];
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn the_old_file_keys_are_the_keys_it_was_written_with() {
        let old = json!({"folders": ["/m"], "volume": 40.0});
        assert!(old.as_object().unwrap().keys().all(|k| Kept::LEGACY_KEYS.contains(&k.as_str())));
    }

    #[test]
    fn the_file_from_before_the_framework_reads_as_these_settings() {
        let read = |raw: Value| serde_json::from_value::<Kept>(raw).ok();
        assert_eq!(
            read(json!({"folders": ["/home/me/Music"], "volume": 40.0})),
            Some(Kept { folders: Some(vec!["/home/me/Music".into()]), volume: 40.0 })
        );
        assert_eq!(read(json!({"folders": []})), Some(Kept { folders: Some(vec![]), volume: 80.0 }), "an emptied list stays empty");
        assert_eq!(read(json!({"folders": [1, 2]})), None);
        assert_eq!(read(json!({"folders": ["/m"], "volume": "loud"})), None);
    }
}
