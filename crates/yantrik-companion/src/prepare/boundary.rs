//! The hard limits of a preparation job, enforced at the tool boundary (#1416, slice 5).
//!
//! Every call a job makes goes through [`Door::call`], which asks [`Boundary::check`] first and
//! only then `ToolRegistry::execute` (so the permission, reach, taint and audit rules of every
//! other call also apply). A call the boundary does not list is refused, never run, and counted
//! in `refused` so the person can see it. The plan is plain code: a model writes content, and no
//! word of fetched text ever becomes a tool name, a path or a URL of a call.
//!
//! What a job may call:
//! - `web_search` (a short query) and `web_fetch` (an `https` page on a public name);
//! - `write_file` to a file directly inside the job's folder, and nothing else of the disk;
//! - `app_action`: Notes `new_note` with the title `Prepared: <item>`, and Presentation's
//!   `new_deck`, `add_slide`, `set_slide` and `save_as` to a `.ydeck` inside the job's folder.
//!
//! Everything else is refused: a send, a delete, a run, a write anywhere else, any other app.

use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use yantrik_companion_core::tools::{ToolContext, ToolRegistry};

/// The longest search a job makes.
const QUERY_MAX: usize = 200;
/// The longest a note or file body may be (the file tool's own limit is 100 KB).
const BODY_MAX: usize = 60_000;

/// A call the boundary did not allow, and why in plain words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub tool: String,
    pub why: &'static str,
}

/// What one job may touch.
#[derive(Debug, Clone)]
pub struct Boundary {
    folder: PathBuf,
    note_title: String,
}

fn arg<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or("")
}

/// A host a job may fetch: a public name, never a number, `localhost` or a machine-local name.
fn public_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    let h = h.trim_end_matches('.');
    !h.is_empty()
        && h.contains('.')
        && h.parse::<std::net::IpAddr>().is_err()
        && !h.starts_with('[')
        && !h.chars().all(|c| c.is_ascii_digit() || c == '.')
        && !["localhost", ".local", ".localdomain", ".internal", ".lan", ".home", ".corp"].iter().any(|s| h == s.trim_start_matches('.') || h.ends_with(s))
        && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

/// The host of an `https://` URL with no credentials or port, or `None`.
pub fn https_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    (!authority.contains(['@', ':', '\\', ' ']) && public_host(authority)).then_some(authority)
}

impl Boundary {
    pub fn new(folder: PathBuf, item_title: &str) -> Self {
        Self { folder, note_title: note_title(item_title) }
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Whether `path` names a file directly in the job's folder: no `..`, no other directory, and
    /// where the folder exists on disk, no link that leaves it.
    fn in_folder(&self, path: &str, ext: &[&str]) -> bool {
        let p = Path::new(path);
        if !p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
            return false;
        }
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else { return false };
        if p.parent() != Some(self.folder.as_path()) || name.starts_with('.') {
            return false;
        }
        if !ext.is_empty() && !ext.iter().any(|e| name.ends_with(&format!(".{e}"))) {
            return false;
        }
        match (self.folder.canonicalize(), std::fs::symlink_metadata(p)) {
            (Ok(real), Ok(meta)) => !meta.file_type().is_symlink() && p.canonicalize().map_or(false, |c| c.starts_with(real)),
            (Ok(_), Err(_)) => true, // not there yet
            (Err(_), _) => true,     // the folder is made by the first write
        }
    }

    /// Whether the call may run.
    pub fn check(&self, tool: &str, args: &Value) -> Result<(), Refused> {
        let no = |why: &'static str| Err(Refused { tool: tool.to_string(), why });
        match tool {
            "web_search" => {
                let q = arg(args, "query");
                if q.trim().is_empty() || q.chars().count() > QUERY_MAX { return no("a search is a short query") }
                Ok(())
            }
            "web_fetch" => {
                if https_host(arg(args, "url")).is_none() { return no("only https pages on a public name") }
                Ok(())
            }
            "write_file" => {
                if args.get("append").and_then(Value::as_bool).unwrap_or(false) { return no("a job writes whole files") }
                if arg(args, "content").len() > BODY_MAX { return no("too long") }
                if !self.in_folder(arg(args, "path"), &["md", "txt"]) { return no("a job writes only inside its own folder") }
                Ok(())
            }
            "app_action" => self.check_app(args).or_else(|why| no(why)),
            _ => no("not a tool a preparation may use"),
        }
    }

    fn check_app(&self, call: &Value) -> Result<(), &'static str> {
        let inner = call.get("args").cloned().unwrap_or(json!({}));
        match (arg(call, "app"), arg(call, "action")) {
            ("notes", "new_note") if arg(&inner, "title") == self.note_title && arg(&inner, "body").len() <= BODY_MAX => Ok(()),
            ("notes", "new_note") => Err("the new note is named Prepared: and the item"),
            ("presentation", "new_deck" | "add_slide" | "set_slide") => Ok(()),
            ("presentation", "save_as") if self.in_folder(arg(&inner, "path"), &["ydeck"]) && !inner.get("overwrite").and_then(Value::as_bool).unwrap_or(false) => Ok(()),
            ("presentation", "save_as") => Err("a deck is saved only inside the job's folder, and never over a file"),
            _ => Err("that app action is not part of a preparation"),
        }
    }
}

/// The new note's title.
pub fn note_title(item_title: &str) -> String {
    let t: String = item_title.chars().filter(|c| !c.is_control()).take(100).collect();
    format!("Prepared: {}", t.trim())
}

/// The one door a job's calls go through.
pub struct Door<'a> {
    registry: &'a ToolRegistry,
    ctx: &'a ToolContext<'a>,
    boundary: Boundary,
    started: Instant,
    budget: Duration,
    /// Calls the boundary refused.
    pub refused: Vec<Refused>,
    /// Calls that ran.
    pub calls: usize,
}

impl<'a> Door<'a> {
    pub fn new(registry: &'a ToolRegistry, ctx: &'a ToolContext<'a>, boundary: Boundary, budget: Duration) -> Self {
        Self { registry, ctx, boundary, started: Instant::now(), budget, refused: Vec::new(), calls: 0 }
    }

    pub fn boundary(&self) -> &Boundary {
        &self.boundary
    }

    pub fn out_of_time(&self) -> bool {
        self.started.elapsed() >= self.budget
    }

    /// One call: refused by the boundary, or run through `ToolRegistry::execute` (audited).
    pub fn call(&mut self, tool: &str, args: Value) -> Result<String, Refused> {
        if let Err(r) = self.boundary.check(tool, &args) {
            tracing::warn!(tool, why = r.why, "a preparation job's call was refused at its boundary");
            self.refused.push(r.clone());
            return Err(r);
        }
        self.calls += 1;
        Ok(self.registry.execute(self.ctx, tool, &args))
    }
}
