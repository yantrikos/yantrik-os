//! Where Email keeps its two kinds of file, by the framework's one path rule
//! (design/app-framework-2026-10-21.md, §6).
//!
//! The unsent message is the app's own data: `Paths::data_dir`/`draft.json`, which is
//! `~/.local/share/yantrik/email/draft.json` for a session that does not set `XDG_DATA_HOME`, the
//! file the app has always written. The HTML originals handed to a browser are copies of what
//! the mailbox already holds, and losing them loses nothing: `Paths::cache_dir`.
//!
//! A draft an older build left at the old fixed path, when the session's data folder is somewhere
//! else, is copied across once and the old file is kept beside itself as `draft.json.legacy.json`.
//! Nothing the person wrote is deleted.

use std::path::{Path, PathBuf};

use yantrik_app::paths::DEFAULT_PROFILE;
use yantrik_app::Paths;

/// The draft file's name inside the data folder.
pub const DRAFT: &str = "draft.json";

/// What an imported file is kept as.
pub const LEGACY_SUFFIX: &str = ".legacy.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Files {
    pub draft: PathBuf,
    pub originals: PathBuf,
}

impl Files {
    /// The files for these paths: the per-app overrides when the session sets them (never under
    /// `cargo test`, so no test can touch the person's own), the framework's folders otherwise.
    pub fn for_app(paths: &Paths) -> Files {
        Files {
            draft: overridden("YANTRIK_EMAIL_DRAFT").unwrap_or_else(|| paths.data_dir().join(DRAFT)),
            originals: overridden("YANTRIK_EMAIL_HTML_DIR").unwrap_or_else(|| paths.cache_dir().to_path_buf()),
        }
    }
}

/// A per-app override, said once in the log: the design keeps them for one release.
fn overridden(variable: &str) -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let value = std::env::var_os(variable).filter(|v| !v.is_empty())?;
    tracing::warn!(variable, "a per-app path override is set; it is honoured for one release and then removed");
    Some(PathBuf::from(value))
}

/// The mark beside the draft that says whether its message was already sent is not known.
pub fn unsure_mark(draft: &Path) -> PathBuf {
    draft.with_extension("json.unsure")
}

/// Whether the draft at `draft` carries the mark.
pub fn is_unsure(draft: &Path) -> bool {
    unsure_mark(draft).is_file()
}

/// Puts the mark beside the draft, or takes it away. From a job.
pub fn set_unsure(draft: &Path, unsure: bool) -> Result<(), String> {
    let mark = unsure_mark(draft);
    if unsure {
        // Written as the draft is: whole, 0600, never through a link.
        return crate::state::write_whole(&mark, b"");
    }
    match std::fs::remove_file(&mark) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("could not remove {}: {e}", mark.display())),
        _ => Ok(()),
    }
}

/// Where builds before the framework kept the draft, whatever the session's data folder was.
pub fn legacy_draft() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join(".local/share/yantrik/email").join(DRAFT))
}

/// Brings an older build's draft to where this one reads it, once.
///
/// Only for the default profile (a second profile is a new place, and the old file was the
/// default's), only for the data folder's own draft (an override was the old build's file too),
/// only when this build has no draft of its own, and only when the two paths differ. The old
/// file is renamed beside itself, not removed, so it is still there if anything goes wrong; a
/// copy that fails leaves both as they were and says why in the log.
pub fn import_legacy_draft(paths: &Paths, files: &Files, legacy: Option<&Path>) {
    let Some(legacy) = legacy else { return };
    let own = files.draft == paths.data_dir().join(DRAFT);
    if !own || paths.profile() != DEFAULT_PROFILE || legacy == files.draft || files.draft.exists() || !legacy.is_file() {
        return;
    }
    let copied = files
        .draft
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::copy(legacy, &files.draft).map(|_| ()));
    if let Err(e) = copied {
        tracing::warn!(error = %e, from = %legacy.display(), "the draft from before could not be brought across");
        return;
    }
    let kept = PathBuf::from(format!("{}{LEGACY_SUFFIX}", legacy.display()));
    if let Err(e) = std::fs::rename(legacy, &kept) {
        tracing::warn!(error = %e, from = %legacy.display(), "the draft was brought across and the old file could not be renamed");
    }
}
