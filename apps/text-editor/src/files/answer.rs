//! The part of an answer that is read off the disk: "it worked" is what the file says afterwards,
//! never the editor's opinion of its own handler. The rest of the answer is built from the model
//! when the call's own work committed; this job adds what the disk holds and decides whether the
//! call got what it was owed.

use std::path::PathBuf;

use serde_json::{json, Value};

use super::read::read_text;

/// One thing to read back into an answer.
#[derive(Debug, Clone)]
pub enum Fill {
    /// A tab as `document_now` reports it, at `at` in the answer (the top level when `None`):
    /// whether its file is on disk, and whether the file holds exactly the tab's text.
    Tab { at: Option<&'static str>, path: Option<PathBuf>, text: String },
    /// A file just written: its length and lines as the disk has them.
    Written { path: PathBuf },
}

/// What the call is refused with when the disk did not hold what it was owed.
#[derive(Debug, Clone)]
pub enum Require {
    Nothing,
    /// The tab's file must hold exactly its text; else this sentence (or the notice).
    Matches(String),
    /// The file just written must be there to read; else this sentence (or the notice).
    Written(String),
}

/// An answer waiting on the disk.
#[derive(Debug, Clone)]
pub struct ReadBack {
    pub value: Value,
    pub fills: Vec<Fill>,
    pub require: Require,
}

impl ReadBack {
    pub fn new(value: Value) -> ReadBack {
        ReadBack { value, fills: Vec::new(), require: Require::Nothing }
    }

    pub fn fill(mut self, fill: Fill) -> ReadBack {
        self.fills.push(fill);
        self
    }

    pub fn require(mut self, require: Require) -> ReadBack {
        self.require = require;
        self
    }
}

/// Reads the disk into `r`. Returns the answer, and the refusal's default sentence when the disk
/// did not hold what the call was owed.
pub fn read_back(r: ReadBack) -> (Value, Option<String>) {
    let ReadBack { mut value, fills, require } = r;
    let mut matched = true;
    let mut written = true;
    for fill in fills {
        match fill {
            Fill::Tab { at, path, text } => {
                let disk = path.as_deref().and_then(|p| read_text(p).ok());
                let matches = disk.as_deref() == Some(text.as_str());
                matched &= matches;
                let target = match at {
                    Some(key) => &mut value[key],
                    None => &mut value,
                };
                target["on_disk"] = json!(disk.is_some());
                target["matches_disk"] = json!(matches);
            }
            Fill::Written { path } => {
                let disk = read_text(&path).ok();
                written &= disk.is_some();
                value["bytes"] = json!(disk.as_deref().map(str::len));
                value["lines"] = json!(disk.as_deref().map(crate::text::lines));
            }
        }
    }
    let failed = match require {
        Require::Matches(why) if !matched => Some(why),
        Require::Written(why) if !written => Some(why),
        _ => None,
    };
    (value, failed)
}
