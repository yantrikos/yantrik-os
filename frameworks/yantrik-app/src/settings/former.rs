//! Settings an app kept before they had a directory of their own, brought across once
//! (design/app-framework-2026-10-21.md, §6 and decision 3: imported once, backed up, never
//! deleted).
//!
//! Until this rule, an app's settings were `<config>/yantrik/<id>.json`, beside the services'
//! own files; for Email that was email-service's account file, password and all. Now they are
//! `<config>/yantrik/apps/<id>/settings.json`. An app whose settings were at the old place says
//! so by name (`Settings::FORMER_FILE`); an app that does not is never given the old place to
//! look at, and a name that is another component's file is refused whatever the app says
//! (`Paths::former_config_file`).
//!
//! The old file is read once, and only taken if it is one of two shapes: the framework's own
//! envelope (`schema` and `settings`, as it was written before the mark, or with this app's
//! mark), or the app's shape from before the framework (`Settings::legacy`). It is copied to the
//! new place in the marked envelope and left exactly where it was, as the backup: never
//! rewritten, never removed. A file of any other shape is not this app's: it is not taken, its
//! path alone is logged, and it is not looked at again. Either way `imported.json` beside the new
//! file records that the look was taken, so a person who later removes their settings gets the
//! defaults, not the old file a second time.

use std::path::Path;

use serde_json::Value;
use yantrik_foundation::json_store::write_atomically;

use super::file::{private_dir, read_bounded, render_raw, KIND};
use super::{Settings, SettingsError};
use crate::paths::Paths;

/// What came of the look at the old place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Imported {
    /// The app names no old file, or the new file or the record of an earlier look is there.
    Nothing,
    /// Nothing at the old place.
    NoFile,
    /// Copied to the new place; the old file is as it was.
    Copied,
    /// The old file is not in a shape this app wrote: not taken, left as it is.
    NotOurs,
    /// The copy could not be written; tried again at the next start.
    Failed(String),
}

/// Brings `paths`' app's settings from the old place, once. Before the store opens the new file.
pub fn import<T: Settings>(paths: &Paths) -> Imported {
    let Some(name) = T::FORMER_FILE else {
        return Imported::Nothing;
    };
    let Some(former) = paths.former_config_file(name) else {
        tracing::warn!(app = paths.id(), former = name, "the old settings file named is not one an app may read; it is not looked at");
        return Imported::Nothing;
    };
    let new = paths.config_file();
    let record = paths.imported_record();
    if exists(new) || exists(&record) {
        return Imported::Nothing;
    }
    // Read as the settings file is (`crate::quiet`): a link, a FIFO, a device or a file over the
    // limit is not opened past its metadata, and nothing of it is parsed.
    let bytes = match read_bounded(&former) {
        Ok(Some(bytes)) => Some(bytes),
        Ok(None) => return Imported::NoFile,
        Err(SettingsError::NotOurs) => None,
        Err(e) => {
            // Unreadable now is not a decision; the look is taken again at the next start.
            tracing::warn!(app = paths.id(), file = %former.display(), error = %e, "the settings from before could not be looked at");
            return Imported::Failed(e.to_string());
        }
    };
    let recognised = bytes.and_then(|bytes| recognise::<T>(&bytes, paths.id()));
    let written = |rendered: Vec<u8>| {
        new.parent().map_or(Ok(()), private_dir).and_then(|()| write_atomically(new, &rendered))
    };
    let outcome = match recognised.map(|(schema, settings)| render_raw(paths.id(), schema, settings)) {
        Some(Ok(rendered)) => match written(rendered) {
            Ok(()) => {
                tracing::info!(app = paths.id(), from = %former.display(), to = %new.display(), "the settings from before are brought across; the old file is kept as it was");
                Imported::Copied
            }
            Err(e) => {
                tracing::warn!(app = paths.id(), from = %former.display(), to = %new.display(), error = %e, "the settings from before could not be brought across; the old file is as it was");
                return Imported::Failed(e.to_string());
            }
        },
        Some(Err(e)) => return Imported::Failed(e.to_string()),
        None => {
            tracing::warn!(app = paths.id(), file = %former.display(), "the file at the old settings place is not this app's; it is left as it is, unread, and not looked at again");
            Imported::NotOurs
        }
    };
    let said = serde_json::json!({ "from": former.display().to_string(), "taken": outcome == Imported::Copied });
    let recorded = record.parent().map_or(Ok(()), private_dir).and_then(|()| write_atomically(&record, said.to_string().as_bytes()));
    if let Err(e) = recorded {
        tracing::warn!(app = paths.id(), file = %record.display(), error = %e, "the look at the old settings could not be recorded");
    }
    outcome
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// The settings in the old file, as `(schema, document)`, when it is one of the two shapes this
/// app wrote; `None` for anything else. Nothing of the file is kept or said otherwise.
fn recognise<T: Settings>(bytes: &[u8], app: &str) -> Option<(u32, Value)> {
    let raw: Value = serde_json::from_slice(bytes).ok()?;
    let object = raw.as_object()?;
    if object.contains_key("schema") {
        // The envelope, and nothing beside it but this app's own mark.
        let shape = object.keys().all(|k| matches!(k.as_str(), "schema" | "settings" | "kind" | "app"));
        let kind = object.get("kind").is_none_or(|k| k.as_str() == Some(KIND));
        let named = object.get("app").is_none_or(|a| a.as_str() == Some(app));
        if !(shape && kind && named) {
            return None;
        }
        let schema = u32::try_from(object.get("schema")?.as_u64()?).ok()?;
        return Some((schema, object.get("settings")?.clone()));
    }
    let value = T::legacy(raw)?;
    serde_json::from_value::<T>(value.clone()).ok()?;
    Some((T::SCHEMA, value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::DEFAULT_PROFILE;
    use crate::settings::{Health, Store};

    #[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
    struct Old {
        n: i32,
    }
    impl Settings for Old {
        const SCHEMA: u32 = 2;
        const FORMER_FILE: Option<&'static str> = Some("old.json");
        fn legacy(raw: Value) -> Option<Value> {
            raw.get("n").is_some().then_some(raw)
        }
    }

    /// Names email-service's account file as its own: refused, whatever it says.
    #[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
    struct Greedy {
        password: String,
    }
    impl Settings for Greedy {
        const SCHEMA: u32 = 1;
        const FORMER_FILE: Option<&'static str> = Some("email.json");
        fn legacy(raw: Value) -> Option<Value> {
            Some(raw)
        }
    }

    #[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
    struct Quiet {
        n: i32,
    }
    impl Settings for Quiet {
        const SCHEMA: u32 = 1;
    }

    const ACCOUNT: &str = r#"[{"id":"default","email":"p@example.org","password":"SENTINEL-PASSWORD-2"}]"#;

    fn fresh(name: &str, id: &str) -> Paths {
        let root = std::env::temp_dir().join(format!("yantrik-former-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::under(&root, id, DEFAULT_PROFILE);
        std::fs::create_dir_all(root.join("config/yantrik")).unwrap();
        paths
    }

    fn shared(paths: &Paths, name: &str) -> std::path::PathBuf {
        paths.config_file().parent().unwrap().parent().unwrap().parent().unwrap().join(name)
    }

    #[test]
    fn the_apps_own_shape_from_before_is_copied_once_and_the_old_file_kept() {
        let paths = fresh("bare", "old");
        let former = shared(&paths, "old.json");
        std::fs::write(&former, r#"{"n": 7}"#).unwrap();
        assert_eq!(import::<Old>(&paths), Imported::Copied);
        assert_eq!(std::fs::read_to_string(&former).unwrap(), r#"{"n": 7}"#, "the old file is the backup, as it was");
        let store = Store::<Old>::open(paths.config_file().to_path_buf(), "old");
        assert_eq!(store.get(), &Old { n: 7 });
        assert_eq!(store.health(), &Health::Fine);
        assert!(paths.imported_record().exists());
        // Once: the person's later removal of the settings is the defaults, not the old file again.
        std::fs::remove_file(paths.config_file()).unwrap();
        assert_eq!(import::<Old>(&paths), Imported::Nothing);
        assert!(!paths.config_file().exists());
    }

    #[test]
    fn the_frameworks_envelope_from_before_the_mark_is_copied_at_its_schema_and_migrated() {
        let paths = fresh("envelope", "old");
        let former = shared(&paths, "old.json");
        let before = r#"{"schema":1,"settings":{"n":3}}"#;
        std::fs::write(&former, before).unwrap();
        assert_eq!(import::<Old>(&paths), Imported::Copied);
        let store = Store::<Old>::open(paths.config_file().to_path_buf(), "old");
        assert_eq!(store.get(), &Old { n: 3 });
        assert!(store.is_pending(), "schema 1, owed the rewrite at 2");
        assert_eq!(std::fs::read_to_string(&former).unwrap(), before);
    }

    #[test]
    fn a_file_of_another_shape_at_the_old_place_is_not_taken_and_not_looked_at_again() {
        let paths = fresh("foreign", "old");
        let former = shared(&paths, "old.json");
        for (n, content) in [ACCOUNT, r#"{"schema":1,"settings":{},"password":"SENTINEL"}"#, r#"{"other":1}"#].iter().enumerate() {
            let _ = std::fs::remove_file(paths.imported_record());
            std::fs::write(&former, content).unwrap();
            assert_eq!(import::<Old>(&paths), Imported::NotOurs, "shape {n}");
            assert!(!paths.config_file().exists(), "nothing of it is copied");
            assert_eq!(std::fs::read_to_string(&former).unwrap(), *content, "left as it is");
            let record = std::fs::read_to_string(paths.imported_record()).unwrap();
            assert!(!record.contains("SENTINEL") && record.contains("\"taken\":false"), "{record}");
            assert_eq!(import::<Old>(&paths), Imported::Nothing, "not looked at again");
        }
    }

    #[test]
    fn another_components_file_is_never_an_apps_former_file() {
        let paths = fresh("greedy", "email");
        let account = shared(&paths, "email.json");
        std::fs::write(&account, ACCOUNT).unwrap();
        assert_eq!(import::<Greedy>(&paths), Imported::Nothing);
        assert!(!paths.config_file().exists() && !paths.imported_record().exists());
        let store = Store::<Greedy>::open(paths.config_file().to_path_buf(), "email");
        assert_eq!(store.get(), &Greedy::default());
        assert_eq!(std::fs::read_to_string(&account).unwrap(), ACCOUNT);
    }

    #[test]
    fn an_app_that_names_no_former_file_never_looks_at_the_old_place() {
        let paths = fresh("quiet", "quiet");
        let old = shared(&paths, "quiet.json");
        std::fs::write(&old, r#"{"n": 5}"#).unwrap();
        assert_eq!(import::<Quiet>(&paths), Imported::Nothing);
        assert_eq!(import::<()>(&paths), Imported::Nothing);
        assert!(!paths.config_file().exists() && !paths.imported_record().exists());
    }

    /// A link, a FIFO or a file over the limit at the old place is not this app's: nothing of it
    /// is read or parsed, the look is recorded once, and the app starts without waiting on it.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_link_or_a_fifo_at_the_old_place_is_not_followed_read_or_waited_on() {
        use crate::quiet::tests::{bytes_read_by, fifo, within, BIG, NOTHING_OF_IT};
        use std::time::Duration;
        let makers: [fn(&Path, &Path); 3] = [
            |at: &Path, d: &Path| {
                let secret = d.join("email.json");
                std::fs::write(&secret, format!("[{}]", "SENTINEL,".repeat(BIG / 9))).unwrap();
                std::os::unix::fs::symlink(&secret, at).unwrap();
            },
            |at: &Path, _: &Path| fifo(at),
            |at: &Path, _: &Path| std::os::unix::fs::symlink("/dev/zero", at).unwrap(),
        ];
        for (n, make) in makers.into_iter().enumerate() {
            let paths = fresh(&format!("hostile-{n}"), "old");
            let former = shared(&paths, "old.json");
            make(&former, former.parent().unwrap());
            let (imported, read) = within(Duration::from_secs(5), {
                let paths = paths.clone();
                move || bytes_read_by(|| import::<Old>(&paths))
            });
            assert_eq!(imported, Imported::NotOurs, "case {n}");
            assert!(read < NOTHING_OF_IT, "case {n}: {read} bytes read");
            assert!(!paths.config_file().exists());
            let record = std::fs::read_to_string(paths.imported_record()).unwrap();
            assert!(record.contains("\"taken\":false"), "{record}");
            assert_eq!(import::<Old>(&paths), Imported::Nothing, "case {n}: not looked at again");
        }
    }

    #[test]
    fn settings_already_at_the_new_place_are_never_replaced_from_the_old() {
        let paths = fresh("both", "old");
        std::fs::write(shared(&paths, "old.json"), r#"{"n": 1}"#).unwrap();
        let mine = crate::settings::render("old", &Old { n: 9 }).unwrap();
        write_atomically(paths.config_file(), &mine).unwrap();
        assert_eq!(import::<Old>(&paths), Imported::Nothing);
        assert_eq!(std::fs::read(paths.config_file()).unwrap(), mine);
        assert_eq!(import::<Old>(&fresh("missing", "old")), Imported::NoFile);
    }
}
