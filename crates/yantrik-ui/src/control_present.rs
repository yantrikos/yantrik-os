//! `present_file {path, title?}`: a Mind offers the person something it made, and the person
//! decides whether to look (#1068).
//!
//! "Build a page and show me" had no route: `files_open` is refused for a Mind (it would move the
//! person's view, and open a file in an app that is not Yantrik's own), and `go` took only web
//! addresses. This is the route that asks. It raises a toast on the person's desktop, "<Mind> made
//! <name>", with the full path and an Open button, and does nothing else: no window, screen,
//! selection or clipboard is touched until the person presses. The press opens the file the way a
//! double-click in Files does (`wire::open_with::launch`: the browser for HTML, the image viewer
//! for a picture), as the person.
//!
//! - **Graded `standard`**: it only asks. Nothing opens, so there is nothing to approve first.
//! - **The same gates as `files_move` / `files_rename`** (`control_files_mind`): a file in the
//!   person's home, outside its protected places, links followed; `..` refused.
//! - **The toast is the shell's own** (`wire::toast`), not a second card system. A repeat for the
//!   same file replaces its card, and only the last few stay openable, so a Mind cannot bury the
//!   desktop in cards.
//! - **A press opens what was shown.** The file's real path is kept with the toast; if the path
//!   now leads somewhere else (a link swapped in since), the press opens nothing and says so.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{json, Value};
use slint::{ComponentHandle, ModelRc, VecModel};
use yantrik_app_runtime::control::{Action, Param};

use crate::control_files_cursor::{entry_in, spelled, text};
use crate::control_files_mind as mind;
use crate::mind_view::{requester_now, Requester};
use crate::{App, ToastActionData, ToastData};

/// The id every presentation's toast starts with. `local:` keeps it out of the notification store.
pub const TOAST: &str = "local:present-file";

/// How many presentations stay openable; older ones are forgotten.
const KEPT: usize = 8;

/// What was offered, by toast id: the real path at the time of the offer.
static OFFERED: Mutex<Vec<(String, PathBuf)>> = Mutex::new(Vec::new());

/// The action's declaration.
pub(crate) fn action() -> Action {
    Action::new(
        "present_file",
        "Offer the person a file you made. A card appears on their desktop - \"<you> made <name>\" \
         with the full path and an Open button - and nothing else happens: their screen does not \
         move until they press it. Their press opens it in the right app (the browser for HTML). \
         Use it for \"show me\"; to look at your own page yourself, use web_go with a file:// URL \
         in the browser. Answers whether the card is up.",
    )
    .risk("standard")
    .arg(Param::text("path").describe("The file, e.g. ~/Sites/index.html; in the person's home"))
    .arg(Param::text("title").describe("A few words for the card, e.g. \"your website\"").optional())
}

/// A file decided on.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Offer {
    pub real: PathBuf,
    pub name: String,
}

/// Decide the offer from `args`, with `home` as the person's home.
pub(crate) fn plan(args: &Value, home: &Path) -> Result<Offer, String> {
    let asked = spelled(text(args, "path"))?;
    let (parent, name) = entry_in(&json!({ "path": asked }))?;
    crate::fileops::name(&name)?;
    crate::control_files_move::no_dots(&parent)?;
    mind::entry_verdict(&parent, &name, home)?;
    let real = crate::filebrowser::expand_home(&parent)
        .join(&name)
        .canonicalize()
        .map_err(|e| format!("{asked} cannot be read: {e}"))?;
    // Where the path leads now, links followed, must itself be somewhere the Mind may name.
    let (real_parent, real_name) = (
        real.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        real.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
    );
    mind::entry_verdict(&real_parent, &real_name, home)?;
    if !real.is_file() {
        return Err(format!("{} is not a file; present_file offers a file", real.display()));
    }
    Ok(Offer { real, name })
}

/// Who made it, as the card says: the Mind's name, cleaned, or a plain word.
fn maker(who: &Requester) -> String {
    let name = match who {
        Requester::Mind(name) => name.as_str(),
        Requester::Person => "Yantrik",
    };
    let clean: String = name.chars().filter(|c| !c.is_control()).take(40).collect();
    if clean.trim().is_empty() {
        "A Mind".into()
    } else {
        clean.trim().to_string()
    }
}

/// The toast id for a file: the same file replaces its own card.
fn id_for(real: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    real.hash(&mut h);
    format!("{TOAST}:{:016x}", h.finish())
}

/// The card: "<maker> made <name or title>", the whole path, and Open.
pub(crate) fn card(id: &str, who: &Requester, offer: &Offer, title: &str) -> ToastData {
    let what: String = if title.trim().is_empty() {
        offer.name.clone()
    } else {
        title.chars().filter(|c| !c.is_control()).take(60).collect()
    };
    ToastData {
        id: id.into(),
        app_name: maker(who).into(),
        summary: format!("{} made {}", maker(who), what.trim()).into(),
        body: offer.real.display().to_string().into(),
        urgency: 1,
        icon_char: "+".into(),
        actions: ModelRc::new(VecModel::from(vec![ToastActionData { id: "open".into(), label: "Open".into() }])),
        thumbnail: Default::default(),
        has_thumbnail: false,
    }
}

/// Remember what the card offers, newest last, forgetting the oldest past [`KEPT`].
fn remember(id: &str, real: &Path) {
    let mut offered = OFFERED.lock().unwrap_or_else(|e| e.into_inner());
    offered.retain(|(i, _)| i != id);
    offered.push((id.to_string(), real.to_path_buf()));
    while offered.len() > KEPT {
        offered.remove(0);
    }
}

/// The file a press on `id` opens, if the card is still one of ours and its path still leads to
/// the file that was offered.
pub(crate) fn what_a_press_opens(id: &str) -> Result<PathBuf, String> {
    let real = OFFERED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(i, _)| i == id)
        .map(|(_, p)| p.clone())
        .ok_or("that card is no longer one that can be opened")?;
    match real.canonicalize() {
        Ok(now) if now == real => Ok(real),
        _ => Err(format!("{} is not where it was offered; nothing was opened", real.display())),
    }
}

/// The handler: checks, then one toast. UI thread.
pub(crate) fn run(weak: &slint::Weak<App>, args: &Value) -> Result<Value, String> {
    let ui = weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let offer = plan(args, &home)?;
    let id = id_for(&offer.real);
    remember(&id, &offer.real);
    crate::wire::toast::push(&ui, card(&id, &requester_now(), &offer, text(args, "title")), 1);
    Ok(json!({ "presented": offer.real.display().to_string(), "card": "up", "opens_when": "the person presses Open" }))
}

/// The Open button. UI thread, from `wire::notifications`' toast handler; the caller is the
/// person's press.
pub fn press(ui: &App, id: &str, action_id: &str) {
    if action_id == "open" {
        match what_a_press_opens(id) {
            Ok(path) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                crate::wire::open_with::launch(&crate::mime_dispatch::classify(&name), &name, &path, Some(ui));
            }
            Err(why) => crate::wire::toast::local(&ui.as_weak(), "Desktop", "Could not open that", &why, 1),
        }
    }
    crate::wire::toast::remove(ui, id);
}

#[cfg(test)]
#[path = "control_present_tests.rs"]
mod tests;
