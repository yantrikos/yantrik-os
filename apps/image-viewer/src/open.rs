//! Opening a picture: the file, the folder around it, what the file says about itself, and
//! whether an agent may be shown it. Every step reads the disk, so all of it runs in a job and
//! comes back as one [`Loaded`] (or the open's refusal) for `update` to put on screen at once.

use std::path::{Path, PathBuf};

use yantrik_image_core::{dimensions_from_size, read_info, Gallery, ImageInfo};

use crate::agent_rule;
use crate::model::OpenAs;
use crate::picture::{self, Picture};

/// What a picture's load found.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub path: PathBuf,
    pub info: ImageInfo,
    /// `None`: a file that is not a picture, or one that has gone.
    pub picture: Option<Picture>,
    /// Whether an agent may be shown this picture and the folder it is in.
    pub shown_to_agents: Result<(), String>,
}

/// Why an agent is not shown a picture whose load ended without a result.
pub const UNREADABLE: &str = "the picture could not be read, so an agent is not shown it";

impl Loaded {
    /// A load that ended without a result (its worker panicked): the name stays on screen with
    /// the window's own sentence for a file it cannot show, and an agent is shown nothing of it.
    pub fn unreadable(path: PathBuf) -> Loaded {
        Loaded {
            path,
            info: ImageInfo::default(),
            picture: None,
            shown_to_agents: Err(UNREADABLE.into()),
        }
    }
}

/// Read the picture at `path`: its facts, its pixels and the agent verdict. Never fails: a file
/// that will not load is a `Loaded` with no picture, and the window says so by name.
pub fn load(path: &Path) -> Loaded {
    let shown_to_agents = agent_rule::verdict(path);
    let mut info = read_info(path);
    let picture = decode(path);
    match &picture {
        Some(p) if info.dimensions.is_empty() => {
            info.dimensions = dimensions_from_size(p.width, p.height);
        }
        Some(_) => {}
        // A file that is not a picture, or one that has gone. Said on screen with its name; a
        // blank window with no explanation was the old behaviour for everything.
        None => tracing::warn!(path = %path.display(), "could not load image"),
    }
    Loaded {
        path: path.to_path_buf(),
        info,
        picture,
        shown_to_agents,
    }
}

#[cfg(not(test))]
fn decode(path: &Path) -> Option<Picture> {
    picture::decode(path)
}

/// Under test, a file of this name stands for a decoder that falls over.
#[cfg(test)]
pub const PANICS: &str = "decoder-panics.png";

#[cfg(test)]
fn decode(path: &Path) -> Option<Picture> {
    if path.file_name().is_some_and(|n| n == PANICS) {
        panic!("the decoder fell over");
    }
    picture::decode(path)
}

/// Open `path` within `gallery`: the agent's rule first, before the path is looked at at all,
/// so a refusal says nothing about whether a file is there; then, for an asked open, the file
/// must be there; then the folder around it (the one already open, if it holds the file) and
/// the picture itself.
pub fn open(path: PathBuf, gallery: Gallery, agent: bool, how: OpenAs) -> Result<(Gallery, Loaded), String> {
    if agent {
        agent_rule::may_open(&path)?;
    }
    // Checked here so the answer names the file rather than leaving a blank window and a caller
    // believing the picture is on screen. The first launch's missing file is named on screen
    // instead (`Gallery::open` keeps it selected).
    if how == OpenAs::Asked && !path.is_file() {
        return Err(format!("no file at {}", path.display()));
    }
    let mut gallery = gallery;
    if !gallery.select(&path) {
        gallery = Gallery::open(&path);
    }
    let loaded = load(&path);
    Ok((gallery, loaded))
}
