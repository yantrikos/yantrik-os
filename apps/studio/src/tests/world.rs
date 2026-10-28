//! A world of a test's own: a gallery, a Trash folder and a settings file, all inside one
//! temporary directory, handed to the app instead of arranged through the environment.
//!
//! `HOME`, `XDG_DATA_HOME` and `XDG_CONFIG_HOME` are process-wide and the tests in one binary run at
//! the same time, so a test that moved one of them would change what every other test saw,
//! including which folder a picture was written into. The app asks for its world in `Studio::model`
//! (`world` in lib.rs), and under test that is answered from here, by the folder its framework files
//! are in, which is one per test.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use yantrik_app::Paths;

use crate::config::Config;
use crate::model::Places;
use crate::Studio;

static WORLDS: Mutex<Option<HashMap<PathBuf, (Config, Places)>>> = Mutex::new(None);

/// What one test works in.
#[derive(Clone)]
pub(crate) struct World {
    pub dir: PathBuf,
    pub places: Places,
    pub paths: Paths,
}

/// A world with this configuration, registered for the app that starts at its `paths`.
pub(crate) fn make(name: &str, config: Config) -> World {
    let dir = std::env::temp_dir().join(format!(
        "studio-{name}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    // Canonicalised, because the app canonicalises what it finds and the Trash records the
    // canonical parent: on a machine whose /tmp is a symlink, comparing the two would otherwise
    // compare different spellings of one place.
    let dir = dir.canonicalize().unwrap();
    let places = Places {
        gallery: dir.join("gallery"),
        trash: dir.join("trash-v2"),
        settings: Some(dir.join("studio.json")),
    };
    let paths = Paths::temporary(<Studio as yantrik_app::App>::ID);
    register(&paths, config, places.clone());
    World { dir, places, paths }
}

pub(crate) fn register(paths: &Paths, config: Config, places: Places) {
    WORLDS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(paths.config_file().to_path_buf(), (config, places));
}

/// The world registered for the app being started at `paths`. Kept, so a restart finds it again.
pub(crate) fn take(paths: &Paths) -> (Config, Places) {
    WORLDS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .and_then(|worlds| worlds.get(paths.config_file()))
        .cloned()
        .unwrap_or_else(|| panic!("no world was made for the app at {}", paths.config_file().display()))
}
