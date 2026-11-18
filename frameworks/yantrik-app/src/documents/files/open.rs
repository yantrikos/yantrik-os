//! Opening a file as a document.

use std::path::{Path, PathBuf};

use super::agent;
use super::read::{inode, read_text, Inode};
use super::FileRules;

/// A file read for a document.
#[derive(Debug, Clone)]
pub struct Opened {
    /// Where it is, every link in the path resolved: what the document is named by and saved to.
    pub path: PathBuf,
    pub text: String,
    /// It is in a place no agent is shown.
    pub hidden: bool,
    pub inode: Option<Inode>,
}

/// Opens the file at `path`. An agent may name only a file in the person's home, outside its
/// protected places; the person may open any text file they can read.
pub fn open(rules: &dyn FileRules, path: &Path, agent: bool) -> Result<Opened, String> {
    agent::may_read(agent, path)?;
    let path = std::fs::canonicalize(path).map_err(|e| rules.cannot_open(path, &e))?;
    let text = read_text(rules, &path)?;
    Ok(described(path, text))
}

/// A file found at `path` holding `text`, with what the document needs to know of where it is.
pub fn described(path: PathBuf, text: String) -> Opened {
    Opened { hidden: agent::hidden_from_agents(&path), inode: inode(&path), path, text }
}
