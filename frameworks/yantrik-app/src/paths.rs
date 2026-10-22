//! Where an app keeps its files: one rule, for every app
//! (design/app-framework-2026-10-21.md, §6).
//!
//! | kind   | path                                              | holds              |
//! |--------|---------------------------------------------------|--------------------|
//! | config | `$XDG_CONFIG_HOME/yantrik/apps/<id>/settings.json` | preferences        |
//! | data   | `$XDG_DATA_HOME/yantrik/<id>/`                    | the app's own data |
//! | state  | `$XDG_STATE_HOME/yantrik/<id>/`                   | drafts and recovery|
//! | cache  | `$XDG_CACHE_HOME/yantrik/<id>/`                   | disposable         |
//!
//! The settings live in a directory only the framework writes, `yantrik/apps/`, and not at
//! `yantrik/<id>.json` beside the services' own files: there, Email's settings were
//! email-service's account file, which holds the account password. Nothing but the framework
//! uses `apps/` under any of the four roots ([`others`] lists what the rest of the system keeps
//! there, and a test holds every app id clear of it). Settings an app kept at the old place are
//! brought across once (`settings::former`), and the old file is left where it was.
//!
//! `$YANTRIK_HOME` roots all four for a second profile; `$YANTRIK_PROFILE` names the profile,
//! and a profile other than the default is a second instance of the app (its own claim, its
//! own socket). Tests build a [`Paths`] under a directory of their own instead of setting the
//! environment, which parallel tests cannot share.

use std::path::{Path, PathBuf};

use yantrik_foundation::xdg;

pub mod others;

/// The directory under `$XDG_CONFIG_HOME/yantrik/` that holds every app's settings, and nothing
/// of any other component's.
pub const APPS_DIR: &str = "apps";

/// An app's settings file, inside its own directory under [`APPS_DIR`].
pub const SETTINGS_FILE: &str = "settings.json";

/// Beside the settings: the record that the settings from before were looked at, so they are
/// looked at once (`settings::former`).
pub const IMPORTED_FILE: &str = "imported.json";

/// The profile every app runs under unless the session says otherwise.
pub const DEFAULT_PROFILE: &str = "default";

/// The four places an app's files go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    id: String,
    profile: String,
    /// `…/yantrik/`: shared with every other component, so the framework only ever reads a file
    /// here that an app names as its own from before (`former_config_file`), and never writes.
    shared_config: PathBuf,
    config_file: PathBuf,
    data_dir: PathBuf,
    state_dir: PathBuf,
    cache_dir: PathBuf,
}

impl Paths {
    /// The paths for `id` under the session's profile and base directories.
    pub fn for_app(id: &str) -> Paths {
        let home = std::env::var_os("YANTRIK_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        let profile = std::env::var("YANTRIK_PROFILE").ok();
        Paths::resolve(id, home.as_deref(), profile.as_deref())
    }

    /// [`Paths::for_app`] from the two variables, so the rule has a test: a profile other than
    /// the default lives only under `$YANTRIK_HOME` (the design's "roots all four for a second
    /// profile"). Named without a home, it would be a second instance on the same files, so it
    /// is refused with a warning and the default profile runs. A relative home is refused too.
    pub fn resolve(id: &str, home: Option<&Path>, profile: Option<&str>) -> Paths {
        let home = match home {
            Some(h) if h.is_absolute() => Some(h),
            Some(h) => {
                tracing::warn!(app = id, home = %h.display(), "$YANTRIK_HOME is not an absolute path; ignored");
                None
            }
            None => None,
        };
        let asked = profile
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string);
        let profile = match (asked, home) {
            (None, _) => DEFAULT_PROFILE.to_string(),
            (Some(p), None) => {
                tracing::warn!(app = id, profile = %p, "$YANTRIK_PROFILE without $YANTRIK_HOME would share the default profile's files; running the default profile");
                DEFAULT_PROFILE.to_string()
            }
            (Some(p), Some(_)) if p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') && p.len() <= 64 => p,
            (Some(p), Some(_)) => {
                tracing::warn!(app = id, profile = %p, "$YANTRIK_PROFILE is not a name (letters, digits, - and _, up to 64); running the default profile");
                DEFAULT_PROFILE.to_string()
            }
        };
        match home {
            Some(root) => Paths::under(root, id, &profile),
            None => Paths::from_bases(
                &xdg::config_home(),
                &xdg::data_home(),
                &xdg::state_home(),
                &xdg::cache_home(),
                id,
                &profile,
            ),
        }
    }

    /// A root of its own for one test's app, under the system's temporary directory, so no test
    /// touches the person's files and no two tests touch each other's.
    pub fn temporary(id: &str) -> Paths {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir()
            .join("yantrik-tests")
            .join(format!("{}-{n}", std::process::id()));
        Paths::under(&root, id, DEFAULT_PROFILE)
    }

    /// Every kind under one root: `root/config/yantrik/apps/<id>/settings.json`, `root/data/yantrik/<id>/`, and
    /// so on. For `$YANTRIK_HOME`, and for a test's temporary directory.
    pub fn under(root: &Path, id: &str, profile: &str) -> Paths {
        Paths::from_bases(
            &root.join("config"),
            &root.join("data"),
            &root.join("state"),
            &root.join("cache"),
            id,
            profile,
        )
    }

    fn from_bases(config: &Path, data: &Path, state: &Path, cache: &Path, id: &str, profile: &str) -> Paths {
        let shared_config = config.join("yantrik");
        Paths {
            id: id.into(),
            profile: profile.into(),
            config_file: shared_config.join(APPS_DIR).join(id).join(SETTINGS_FILE),
            shared_config,
            data_dir: data.join("yantrik").join(id),
            state_dir: state.join("yantrik").join(id),
            cache_dir: cache.join("yantrik").join(id),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }

    /// The name the instance claim and the control socket are keyed by: the id, or
    /// `<id>.<profile>` for a profile other than the default (the socket is then
    /// `app-<id>.<profile>.sock`), so two profiles are two instances. The `.` is outside both
    /// charsets (ids are `[a-z0-9-]`, profiles `[A-Za-z0-9_-]`), so two names never collide; the
    /// design's §2.2 wrote `-`, under which `notes-work` and `notes` in profile `work` were one.
    pub fn instance_name(&self) -> String {
        if self.profile == DEFAULT_PROFILE {
            self.id.clone()
        } else {
            format!("{}.{}", self.id, self.profile)
        }
    }

    /// `…/yantrik/apps/<id>/settings.json`: the settings.
    pub fn config_file(&self) -> &Path {
        &self.config_file
    }

    /// `…/yantrik/apps/<id>/imported.json`: that the settings from before were looked at.
    pub fn imported_record(&self) -> PathBuf {
        self.config_file.with_file_name(IMPORTED_FILE)
    }

    /// `…/yantrik/<name>`: where this app kept its settings before they had a directory of their
    /// own (`Settings::FORMER_FILE`). `None` for a name that is not a plain `.json` file name, or
    /// that is another component's file ([`others::ENTRIES`]): that file is never read as an
    /// app's, whatever the app says.
    pub fn former_config_file(&self, name: &str) -> Option<PathBuf> {
        let plain = name.ends_with(".json")
            && name.len() > ".json".len()
            && !name.starts_with('.')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        (plain && !others::is_others(others::Root::Config, name, &self.id)).then(|| self.shared_config.join(name))
    }

    /// `…/yantrik/<id>/`: the app's own data.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// `…/yantrik/<id>/`: drafts and recovery.
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// `…/yantrik/<id>/`: disposable.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// `<data>/yantrik/problems/`: where this app's problem records go. Under `$YANTRIK_HOME`
    /// for a second profile or a scratch run, like every other path here, so a run that is not
    /// the person's own never files into the person's records.
    pub fn problems_dir(&self) -> PathBuf {
        self.data_dir.parent().map_or_else(|| PathBuf::from("problems"), |yantrik| yantrik.join("problems"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_rule_and_a_profile_is_its_own_instance() {
        let p = Paths::under(Path::new("/r"), "weather", DEFAULT_PROFILE);
        assert_eq!(p.config_file(), Path::new("/r/config/yantrik/apps/weather/settings.json"));
        assert_eq!(p.imported_record(), Path::new("/r/config/yantrik/apps/weather/imported.json"));
        assert_eq!(p.data_dir(), Path::new("/r/data/yantrik/weather"));
        assert_eq!(p.state_dir(), Path::new("/r/state/yantrik/weather"));
        assert_eq!(p.cache_dir(), Path::new("/r/cache/yantrik/weather"));
        assert_eq!(p.instance_name(), "weather");
        let q = Paths::under(Path::new("/r"), "weather", "work");
        assert_eq!(q.instance_name(), "weather.work");
    }

    #[test]
    fn a_profile_lives_only_under_a_home_of_its_own() {
        let p = Paths::resolve("notes", Some(Path::new("/h")), Some("work"));
        assert_eq!(p.profile(), "work");
        assert_eq!(p.config_file(), Path::new("/h/config/yantrik/apps/notes/settings.json"));
        // Without a home, a profile would be a second instance on the default's files: refused.
        let p = Paths::resolve("notes", None, Some("work"));
        assert_eq!(p.profile(), DEFAULT_PROFILE);
        assert_eq!(p.instance_name(), "notes");
        // A name that is not one, or a home that is relative, is refused too.
        assert_eq!(Paths::resolve("notes", Some(Path::new("/h")), Some("../x")).profile(), DEFAULT_PROFILE);
        assert_eq!(Paths::resolve("notes", Some(Path::new("rel")), None).config_file(), Paths::resolve("notes", None, None).config_file());
    }

    #[test]
    fn problem_records_follow_the_home_like_every_other_path() {
        let p = Paths::resolve("weather", Some(Path::new("/h")), Some("scratch"));
        assert_eq!(p.problems_dir(), Path::new("/h/data/yantrik/problems"));
        let q = Paths::under(Path::new("/r"), "notes", DEFAULT_PROFILE);
        assert_eq!(q.problems_dir(), Path::new("/r/data/yantrik/problems"));
        // The session's own: the same folder the records always went to.
        let own = Paths::resolve("weather", None, None);
        assert_eq!(own.problems_dir(), yantrik_foundation::xdg::data_home().join("yantrik").join("problems"));
    }

    #[test]
    fn two_temporary_roots_never_meet() {
        let a = Paths::temporary("x");
        let b = Paths::temporary("x");
        assert_ne!(a.config_file(), b.config_file());
        assert!(a.config_file().ends_with("yantrik/apps/x/settings.json"));
    }

    /// The old place is read only by a name that is a file of the app's own: never a service's.
    #[test]
    fn a_former_file_is_never_another_components() {
        let p = Paths::under(Path::new("/r"), "email", DEFAULT_PROFILE);
        assert_eq!(p.former_config_file("email.json"), None, "email-service's account file");
        for odd in ["", ".json", "../email.json", "a/b.json", ".hidden.json", "weather.yaml"] {
            assert_eq!(p.former_config_file(odd), None, "{odd:?}");
        }
        assert_eq!(
            Paths::under(Path::new("/r"), "weather", DEFAULT_PROFILE).former_config_file("weather.json"),
            Some(PathBuf::from("/r/config/yantrik/weather.json"))
        );
    }
}
