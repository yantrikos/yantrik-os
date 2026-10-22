//! What Music keeps between runs: the library folders and the volume. The library itself is not
//! kept: it is whatever the folders hold, and a scan on start reads it again.
//!
//! Before the framework this was `~/.config/yantrik/music.json`, written by this app alone and
//! read by nothing else, in the shape `{"folders": [...], "volume": n}`. The framework keeps the
//! settings in the app's own file now (`<config>/yantrik/apps/music/settings.json`); the old
//! file is named as this app's (`FORMER_FILE`), brought across once in that shape
//! ([`Settings::legacy`]), and left exactly where it was as the backup.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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

    /// The file from before the framework: a list of folders, and the volume when it was set.
    fn legacy(raw: Value) -> Option<Value> {
        let folders = raw.get("folders")?.as_array()?;
        if !folders.iter().all(Value::is_string) {
            return None;
        }
        let volume = match raw.get("volume") {
            None => DEFAULT_VOLUME,
            Some(v) => v.as_f64()?,
        };
        Some(json!({ "folders": folders, "volume": volume }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_from_before_the_framework_is_read_in_its_own_shape() {
        let read = |raw: Value| Kept::legacy(raw).map(|v| serde_json::from_value::<Kept>(v).unwrap());
        assert_eq!(
            read(json!({"folders": ["/home/me/Music"], "volume": 40.0})),
            Some(Kept { folders: Some(vec!["/home/me/Music".into()]), volume: 40.0 })
        );
        assert_eq!(read(json!({"folders": []})), Some(Kept { folders: Some(vec![]), volume: 80.0 }), "an emptied list stays empty");
        assert_eq!(read(json!({"volume": 40.0})), None, "not this app's shape: recovery, not a guess");
        assert_eq!(read(json!({"folders": [1, 2]})), None);
        assert_eq!(read(json!({"folders": ["/m"], "volume": "loud"})), None);
    }
}
