//! Opening a file into a tab.

use std::path::{Path, PathBuf};

use super::read::{inode, read_text};
use crate::agent_rule;
use crate::tab::Inode;

/// A file read for a tab.
#[derive(Debug, Clone)]
pub struct Opened {
    /// Where it is, every link in the path resolved: what the tab is named by and saved to.
    pub path: PathBuf,
    pub text: String,
    /// It is in a place no agent is shown.
    pub hidden: bool,
    pub inode: Option<Inode>,
}

/// Opens the file at `path` for a tab. An agent may name only a file in the person's home,
/// outside its protected places; the person may open any text file they can read.
pub fn open(path: &Path, agent: bool) -> Result<Opened, String> {
    agent_rule::may_read(agent, path)?;
    let path = std::fs::canonicalize(path).map_err(|e| format!("Cannot open: {e}"))?;
    let text = read_text(&path)?;
    Ok(described(path, text))
}

/// A file found at `path` holding `text`, with what the tab needs to know of where it is.
pub fn described(path: PathBuf, text: String) -> Opened {
    Opened { hidden: agent_rule::hidden_from_agents(&path), inode: inode(&path), path, text }
}
