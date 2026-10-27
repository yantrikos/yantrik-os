//! What a launch asks for, typed and versioned (design/app-framework-2026-10-21.md, §2.2 step 4,
//! §10 deep links).
//!
//! A launch is the command line after the program name: files to open, or one deep link
//! `yantrik://<app>/<intent>?arg=value`. It is parsed once, as untrusted input, into a [`Launch`]
//! with a request id. The first copy of an app is given it as `Event::Launch` after `start`; a
//! second copy finds the single-instance slot held and hands its launch to the first over the
//! control surface (the `launch` action), which applies it once (de-duplicating by request id,
//! so a retry never opens a document twice), shows its window, and answers. The second copy
//! then exits. A copy that does not answer may have died: the contender re-checks the slot.
//!
//! A deep-linked intent enters the app's own intents through `Event::Launch`: the app turns it
//! into a command, or ignores it. Nothing here runs an intent on the app's behalf.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::Startup;
use crate::control::{Action, Param};
use crate::present::{self, Presented};

/// The shape of a launch on the wire; bumped when it changes.
pub const VERSION: u32 = 1;

/// The action a second launch hands its launch to.
pub const ACTION: &str = "launch";

/// The scheme of a deep link.
pub const SCHEME: &str = "yantrik://";

/// How many request ids the running copy remembers, to answer a retry without acting twice.
pub const REMEMBERED: usize = 64;

/// An intent named by a deep link, with its arguments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaunchIntent {
    pub name: String,
    /// From a link, every value is text; the intent's declared parameters type them when it is
    /// admitted (`intents::admit`).
    pub args: Value,
}

/// One launch of the app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Launch {
    pub version: u32,
    /// Unique to this launch; a retry carries the same one.
    pub request: String,
    /// The profile the launch is for; a running copy of another profile refuses it.
    pub profile: String,
    /// Files to open, absolute. The app decides what a missing or unreadable one means.
    pub files: Vec<PathBuf>,
    /// The deep link's intent, if there was one.
    pub intent: Option<LaunchIntent>,
}

static NEXT: AtomicU64 = AtomicU64::new(1);

/// A request id no other launch of this session has: the process, the clock and a counter.
pub fn new_request() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{:x}-{:x}-{}",
        std::process::id(),
        nanos,
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

impl Launch {
    /// A launch asking for nothing: the app opens as it does.
    pub fn empty(profile: &str) -> Launch {
        Launch {
            version: VERSION,
            request: new_request(),
            profile: profile.into(),
            files: Vec::new(),
            intent: None,
        }
    }

    /// A launch opening `files` (made absolute against `cwd`).
    pub fn of_files(profile: &str, cwd: &Path, files: impl IntoIterator<Item = PathBuf>) -> Launch {
        let mut launch = Launch::empty(profile);
        launch.files = files.into_iter().map(|f| absolute(cwd, &f)).collect();
        launch
    }

    /// The command line after the program name, for `app_id` in `profile`, with relative paths
    /// made absolute against `cwd`. An option this app does not take is a startup failure, said.
    pub fn parse(app_id: &str, profile: &str, cwd: &Path, args: &[OsString]) -> Result<Launch, Startup> {
        let mut launch = Launch::empty(profile);
        for arg in args {
            let text = arg.to_string_lossy();
            if let Some(link) = text.strip_prefix(SCHEME) {
                if launch.intent.is_some() {
                    return Err(Startup::new("A launch takes one link, and two were given."));
                }
                launch.intent = Some(parse_link(app_id, link)?);
            } else if text.starts_with('-') && text.len() > 1 {
                return Err(Startup::new(format!(
                    "`{text}` is not an option this app takes. It opens the files named on its \
                     command line, or one yantrik://{app_id}/<intent> link."
                )));
            } else if text.is_empty() {
                return Err(Startup::new("An empty argument is not a file to open."));
            } else {
                launch.files.push(absolute(cwd, Path::new(arg)));
            }
        }
        Ok(launch)
    }

    /// Nothing asked for beyond opening the app.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.intent.is_none()
    }

    /// A launch as the wire delivered it, held to the same rules as one parsed here.
    pub fn from_value(value: &Value) -> Result<Launch, String> {
        let launch: Launch =
            serde_json::from_value(value.clone()).map_err(|e| format!("not a launch: {e}"))?;
        launch.validate()?;
        Ok(launch)
    }

    /// The rules every launch is held to, parsed or received.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != VERSION {
            return Err(format!(
                "a launch of version {} from another build; this one speaks version {VERSION}",
                self.version
            ));
        }
        if self.request.is_empty() || self.request.len() > 128 || !self.request.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err("a launch needs a request id of up to 128 letters, digits and dashes".into());
        }
        if !is_name(&self.profile, 64) {
            return Err("a launch names a profile of letters, digits, dashes and underscores".into());
        }
        if self.files.len() > 256 {
            return Err("a launch opens at most 256 files".into());
        }
        for file in &self.files {
            if !file.is_absolute() {
                return Err(format!("a launch names files by absolute path, and `{}` is not", file.display()));
            }
        }
        if let Some(intent) = &self.intent {
            if !is_intent_name(&intent.name) {
                return Err(format!("`{}` is not an intent name", intent.name));
            }
            if !(intent.args.is_object() || intent.args.is_null()) {
                return Err("an intent's arguments are an object of named values".into());
            }
        }
        Ok(())
    }
}

fn absolute(cwd: &Path, file: &Path) -> PathBuf {
    if file.is_absolute() {
        file.to_path_buf()
    } else {
        cwd.join(file)
    }
}

fn is_name(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn is_intent_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// `<app>/<intent>?k=v&k2=v2`, after the scheme.
fn parse_link(app_id: &str, link: &str) -> Result<LaunchIntent, Startup> {
    let (app, rest) = link.split_once('/').unwrap_or((link, ""));
    if app != app_id {
        return Err(Startup::new(format!(
            "That link is for `{app}`, and this is {app_id}."
        )));
    }
    let (name, query) = rest.split_once('?').unwrap_or((rest, ""));
    let name = name.trim_end_matches('/');
    if !is_intent_name(name) {
        return Err(Startup::new(format!(
            "`{name}` is not an intent name; a link is yantrik://{app_id}/<intent>?arg=value."
        )));
    }
    let mut args = serde_json::Map::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = decode(key)?;
        if !is_name(&key, 64) {
            return Err(Startup::new(format!("`{key}` is not an argument name.")));
        }
        if args.contains_key(&key) {
            return Err(Startup::new(format!("The link gives `{key}` twice.")));
        }
        // Kept as text: the declared parameter types it when the intent is raised
        // (`Param` integer, number, boolean). Guessing here turned `02134` into 2134.
        args.insert(key, Value::String(decode(value)?));
    }
    Ok(LaunchIntent {
        name: name.into(),
        args: Value::Object(args),
    })
}

/// Percent-decoding, with `+` as a space; bytes that are not UTF-8 are refused.
fn decode(s: &str) -> Result<String, Startup> {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
                let byte = hex.and_then(|h| u8::from_str_radix(h, 16).ok());
                match byte {
                    Some(b) => out.push(b),
                    None => return Err(Startup::new("The link has a `%` that does not start a %XX escape.")),
                }
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| Startup::new("The link's text is not UTF-8."))
}

// ── The hand-over ───────────────────────────────────────────────────

/// The action as every surface publishes it: the desktop's own hand-over, never an agent's.
pub(crate) fn action() -> Action {
    Action::new(
        ACTION,
        "Hand a launch to the running copy: it opens the files or takes the intent, and shows \
         its window; for the desktop's own launches",
    )
    .risk("safe")
    .arg(Param::object("launch"))
    .arg(Param::text("display").optional())
    .arg(Param::flag("for_mind").optional())
    .arg(Param::text("view").optional())
}

/// What the running copy, `instance`, made of `launch`.
pub fn hand_over(instance: &str, launch: &Launch) -> Presented {
    hand_over_with(instance, launch, None)
}

/// [`hand_over`], from a Mind's launch that will show the Mind's window: `view` is the socket its
/// helper listens on (`crate::displays::helper`), for the running copy to draw the window for.
pub fn hand_over_with(instance: &str, launch: &Launch, view: Option<&std::path::Path>) -> Presented {
    let mut args = present::from_here();
    args["launch"] = serde_json::json!(launch);
    if let Some(view) = view {
        args["view"] = view.display().to_string().into();
    }
    match present::call(instance, ACTION, args) {
        // A copy from a build before this action: it can still show its window.
        Presented::Refused(why) if why.starts_with("unknown action `launch`") => {
            present::ask(instance)
        }
        answered => answered,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn files_are_made_absolute_and_a_link_becomes_an_intent() {
        let cwd = Path::new("/home/p/docs");
        let l = Launch::parse("notes", "default", cwd, &os(&["a.md", "/tmp/b.md"])).unwrap();
        assert_eq!(l.files, vec![PathBuf::from("/home/p/docs/a.md"), PathBuf::from("/tmp/b.md")]);
        assert!(l.intent.is_none() && !l.is_empty());
        assert!(l.validate().is_ok());
        let l = Launch::parse("notes", "default", cwd, &os(&["yantrik://notes/new_note?title=Hello+there&pinned=true&n=3"])).unwrap();
        let i = l.intent.unwrap();
        assert_eq!(i.name, "new_note");
        assert_eq!(i.args, serde_json::json!({"title": "Hello there", "pinned": "true", "n": "3"}));
        assert!(Launch::parse("notes", "default", cwd, &[]).unwrap().is_empty());
    }

    /// Query values are text until the intent's declared parameters type them: a guess here lost
    /// a leading zero and turned `nan` into null.
    #[test]
    fn query_values_stay_text_until_the_intent_types_them() {
        let l = Launch::parse(
            "notes",
            "default",
            Path::new("/"),
            &os(&["yantrik://notes/find?zip=02134&q=nan&n=007&on=true&big=18446744073709551615"]),
        )
        .unwrap();
        assert_eq!(
            l.intent.unwrap().args,
            serde_json::json!({"zip": "02134", "q": "nan", "n": "007", "on": "true", "big": "18446744073709551615"})
        );
    }

    #[test]
    fn what_is_refused_at_the_command_line() {
        let cwd = Path::new("/");
        let bad = |args: &[&str]| Launch::parse("notes", "default", cwd, &os(args)).unwrap_err().sentence;
        assert!(bad(&["--fast"]).contains("not an option"));
        assert!(bad(&["yantrik://weather/refresh"]).contains("for `weather`"));
        assert!(bad(&["yantrik://notes/New-Note"]).contains("not an intent name"));
        assert!(bad(&["yantrik://notes/x?a=1&a=2"]).contains("twice"));
        assert!(bad(&["yantrik://notes/x?a=%zz"]).contains("%XX"));
        assert!(bad(&["yantrik://notes/x", "yantrik://notes/y"]).contains("one link"));
        assert!(bad(&[""]).contains("empty"));
        // `-` alone is a file name some tools use for stdin; here it is a file like any other.
        assert_eq!(Launch::parse("notes", "default", cwd, &os(&["-"])).unwrap().files.len(), 1);
    }

    #[test]
    fn the_wire_holds_a_launch_to_the_same_rules() {
        let mut l = Launch::of_files("default", Path::new("/w"), [PathBuf::from("x.txt")]);
        assert_eq!(Launch::from_value(&serde_json::to_value(&l).unwrap()).unwrap(), l);
        l.version = 2;
        assert!(Launch::from_value(&serde_json::to_value(&l).unwrap()).unwrap_err().contains("version 2"));
        l.version = VERSION;
        l.files = vec![PathBuf::from("relative")];
        assert!(l.validate().unwrap_err().contains("absolute"));
        l.files.clear();
        l.request = "../x".into();
        assert!(l.validate().unwrap_err().contains("request id"));
        l.request = new_request();
        l.profile = "work/evil".into();
        assert!(l.validate().unwrap_err().contains("profile"));
        assert!(Launch::from_value(&serde_json::json!({"nope": 1})).unwrap_err().starts_with("not a launch"));
        assert_ne!(new_request(), new_request());
    }
}
