//! Whether a whole tree may land somewhere: a pasted, moved or renamed folder.
//!
//! Checking only the top of it let everything below arrive wherever the top went: a folder
//! `cfg` holding `autostart/evil.desktop`, renamed to `.config`, creates ~/.config/autostart;
//! a folder of projects pasted into the home brings their .git/config with it. And a file in
//! the tree with a second hard link is a copy of whatever that other name is, which may be a key.

use std::path::Path;

use super::{is_protected, where_programs_look, HIDDEN_RULE};

/// A tree larger than this is not checked entry by entry; the caller refuses it.
const MAX_CHECKED: usize = 20_000;

const RULE: &str = "an agent moves and copies only what it may read, to where it may write";

/// Refuse `src` - a file, or a folder and everything below it, links not followed (they are
/// moved and copied as links) - landing at `landing`, with `home` as the person's home. Each
/// path below `landing` must be neither protected nor hidden, and no file may have a second
/// name. The top of `landing` itself is the caller's to check (`may_create`).
pub fn may_land(src: &Path, landing: &Path, home: &Path) -> Result<(), String> {
    let mut budget = MAX_CHECKED;
    let mut stack = vec![(src.to_path_buf(), landing.to_path_buf())];
    while let Some((from, to)) = stack.pop() {
        let meta = std::fs::symlink_metadata(&from).map_err(|e| format!("{} cannot be read: {e}", from.display()))?;
        if meta.is_file() && links(&meta) > 1 {
            return Err(format!("{RULE}; {} is hard_link", from.display()));
        }
        if !meta.is_dir() {
            continue;
        }
        let entries = std::fs::read_dir(&from).map_err(|e| format!("{} cannot be looked through: {e}", from.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("{} cannot be looked through: {e}", from.display()))?;
            budget = budget
                .checked_sub(1)
                .ok_or_else(|| format!("more than {MAX_CHECKED} entries below {}; too many to check", src.display()))?;
            let (child, lands) = (entry.path(), to.join(entry.file_name()));
            if is_protected(&child) || is_protected(&lands) {
                return Err(format!("{RULE}; {} is protected", lands.display()));
            }
            if where_programs_look(&lands, home) {
                return Err(format!("{HIDDEN_RULE}; {} is hidden_place", lands.display()));
            }
            stack.push((child, lands));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn links(meta: &std::fs::Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::nlink(meta)
}

#[cfg(not(unix))]
fn links(_meta: &std::fs::Metadata) -> u64 {
    1
}
