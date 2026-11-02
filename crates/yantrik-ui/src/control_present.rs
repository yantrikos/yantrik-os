//! `present_file {path, title?}`: a Mind offers the person something it made, and the person
//! decides whether to look (#1068).
//!
//! "Build a page and show me" had no route: `files_open` is refused for a Mind (it would move the
//! person's view, and open a file in an app that is not Yantrik's own), and `go` took only web
//! addresses. This is the route that asks. It raises a toast on the person's desktop, "<Mind> made
//! <name>", with the full path and an Open button, and does nothing else: no window, screen,
//! selection or clipboard is touched until the person presses.
//!
//! - **Graded `standard`**: it only asks. Nothing opens, so there is nothing to approve first. A
//!   taint (`yos-mcp`) does not close it: a Mind that has read private state can still say "it is
//!   done, here it is", because the press is safe to give (next point).
//! - **The press is consent to view, not to run.** A page, a picture-with-script (SVG) or a PDF
//!   opens in a browser apart and offline (`open_url::open_file_offline`): its own profile, none
//!   of the person's sign-ins, no network, no debugging port. A Mind's HTML is the Mind's code;
//!   in a browser with a profile and a network it would run with both, and a taint's
//!   outbound gate would mean nothing. The card says so, and the answer tells the Mind what will
//!   not load. Other types open as a double-click in Files opens them (`wire::open_with::launch`).
//!   A type whose handler is an installed app of the person's choosing is not offered.
//! - **The same gates as `files_move` / `files_rename`** (`control_files_mind`): a file in the
//!   person's home, outside its protected places, links followed; `..` refused.
//! - **What the card says is what opens.** The card names the file as it really is (a link is
//!   followed), with control and direction characters refused, not hidden; the Mind is named as the
//!   shell knows it, or as "Something on this computer".
//! - **The toast is the shell's own** (`wire::toast`), not a second card system. A repeat for the
//!   same file replaces its card, only a few presentation cards stay up (the oldest goes), and
//!   they never push another toast off the screen (`toast::push`).
//! - **A press opens what was shown.** The file's real path, size, modification time and inode are
//!   kept with the card; if any changed, or the path now leads elsewhere, the press opens nothing.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{json, Value};
use slint::{ComponentHandle, ModelRc, VecModel};
use yantrik_app_runtime::control::{Action, Param};

use crate::control_files_cursor::{entry_in, spelled, text};
use crate::control_files_mind as mind;
use crate::mime_dispatch::FileAction;
use crate::mind_view::{requester_now, Requester};
use crate::{App, ToastActionData, ToastData};

/// The id every presentation's toast starts with. `local:` keeps it out of the notification store.
pub const TOAST: &str = "local:present-file";

/// How many presentation cards stay up and openable; the oldest is taken down for a new one.
const KEPT: usize = 4;

/// How long a name or title may be on the card before it is cut (the extension is kept).
const SHOWN: usize = 60;

/// What a press does with a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Opens {
    /// A browser apart: offline, a profile of its own, no debugging port.
    Apart,
    /// The app a double-click in Files uses.
    App(String),
}

/// What was offered: the real path, how the file looked then, and what a press does.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Offered {
    id: String,
    real: PathBuf,
    seen: (u64, i64, u64),
    opens: Opens,
}

static OFFERED: Mutex<Vec<Offered>> = Mutex::new(Vec::new());

/// The action's declaration.
pub(crate) fn action() -> Action {
    Action::new(
        "present_file",
        "Offer the person a file you made. A card appears on their desktop - \"<you> made <name>\" \
         with the full path and an Open button - and nothing else happens: their screen does not \
         move until they press it. A web page, SVG or PDF opens for them in a separate browser with \
         NO network and none of their sign-ins, so make it self-contained: fonts, scripts and \
         pictures from the internet will not load. To look at your own page yourself, use web_go with \
         a file:///home/... URL. Answers whether the card is up.",
    )
    .risk("standard")
    .arg(Param::text("path").describe("The file, e.g. ~/Sites/index.html; in the person's home"))
    .arg(Param::text("title").describe("A few words for the card, e.g. \"your website\"").optional())
}

/// A file decided on.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Offer {
    pub real: PathBuf,
    /// The real file's name (a link followed), clean.
    pub name: String,
    pub opens: Opens,
    pub seen: (u64, i64, u64),
}

/// A character that can make a name say what it is not: controls (a new line on the card),
/// direction overrides (`gpj.html` shown as `html.jpg`), zero-width and line separators.
fn deceptive(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202F}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}')
}

/// Text for the card with those characters taken out.
pub(crate) fn clean(text: &str) -> String {
    text.chars().filter(|c| !deceptive(*c)).collect()
}

/// A name for the card: cut at [`SHOWN`] characters with an ellipsis, the extension kept.
pub(crate) fn shown(name: &str) -> String {
    if name.chars().count() <= SHOWN {
        return name.to_string();
    }
    let ext = Path::new(name).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let ext: String = if ext.chars().count() > 12 { String::new() } else { ext };
    let stem: String = name.chars().take(SHOWN.saturating_sub(ext.chars().count() + 1)).collect();
    format!("{stem}…{ext}")
}

fn fingerprint(real: &Path) -> Result<(u64, i64, u64), String> {
    let m = std::fs::metadata(real).map_err(|e| format!("{} cannot be read: {e}", real.display()))?;
    Ok((m.len(), m.mtime() * 1_000_000_000 + m.mtime_nsec(), m.ino()))
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
    if clean(&real.to_string_lossy()) != real.to_string_lossy() {
        return Err("the file's name or folder has control or direction characters in it, which could make the card \
                    say something else; give it a plain name"
            .to_string());
    }
    let opens = match crate::mime_dispatch::classify(&real_name) {
        FileAction::Browser => Opens::Apart,
        FileAction::DesktopApp(id) => {
            return Err(format!(
                "a file like {real_name} opens in {}, an app the person chose, and present_file does not \
                 offer those; tell them where it is instead",
                id.strip_suffix(".desktop").unwrap_or(&id)
            ))
        }
        other => Opens::App(crate::mime_dispatch::app_name(&other)),
    };
    let seen = fingerprint(&real)?;
    Ok(Offer { real, name: real_name, opens, seen })
}

/// Who made it, as the card says. A caller the shell cannot tell from the person's own programs is
/// not "Yantrik": nothing the system made is announced this way.
fn maker(who: &Requester) -> String {
    let named = match who {
        Requester::Mind(name) => clean(name).trim().chars().take(40).collect::<String>(),
        Requester::Person => String::new(),
    };
    if named.is_empty() {
        "Something on this computer".into()
    } else {
        named
    }
}

/// The toast id for a file: the same file replaces its own card.
fn id_for(real: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    real.hash(&mut h);
    format!("{TOAST}:{:016x}", h.finish())
}

/// What the card's second line says a press will do.
fn opens_line(opens: &Opens, name: &str) -> String {
    let kind = Path::new(name).extension().map(|e| format!(".{} file. ", clean(&e.to_string_lossy()))).unwrap_or_default();
    match opens {
        Opens::Apart => format!(
            "{kind}Opens in a separate browser: no network, none of your sign-ins. Anything it loads from the internet will not show."
        ),
        Opens::App(app) => format!("{kind}Opens in {app}."),
    }
}

/// The card: "<maker> made <name or title>", the whole path, what a press does, and Open.
pub(crate) fn card(id: &str, who: &Requester, offer: &Offer, title: &str) -> ToastData {
    let title = clean(title);
    let what = if title.trim().is_empty() { shown(&offer.name) } else { shown(title.trim()) };
    ToastData {
        id: id.into(),
        app_name: maker(who).into(),
        summary: format!("{} made {what}", maker(who)).into(),
        body: format!(
            "{}\nMade by {}. {}",
            offer.real.display(),
            maker(who),
            opens_line(&offer.opens, &offer.name)
        )
        .into(),
        urgency: 1,
        icon_char: "+".into(),
        actions: ModelRc::new(VecModel::from(vec![ToastActionData { id: "open".into(), label: "Open".into() }])),
        thumbnail: Default::default(),
        has_thumbnail: false,
    }
}

/// Remember what the card offers, newest last. Returns the ids of cards that no longer fit, to be
/// taken down.
fn remember(id: &str, offer: &Offer) -> Vec<String> {
    remember_in(&mut OFFERED.lock().unwrap_or_else(|e| e.into_inner()), id, offer)
}

fn remember_in(offered: &mut Vec<Offered>, id: &str, offer: &Offer) -> Vec<String> {
    offered.retain(|o| o.id != id);
    offered.push(Offered { id: id.to_string(), real: offer.real.clone(), seen: offer.seen, opens: offer.opens.clone() });
    let mut gone = Vec::new();
    while offered.len() > KEPT {
        gone.push(offered.remove(0).id);
    }
    gone
}

/// What a press on `id` opens, if the card is still one of ours and the file is still the one that
/// was offered.
pub(crate) fn what_a_press_opens(id: &str) -> Result<(PathBuf, Opens), String> {
    let o = OFFERED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|o| o.id == id)
        .cloned()
        .ok_or("that card is no longer one that can be opened")?;
    let changed = || format!("{} is not as it was offered; nothing was opened", o.real.display());
    match o.real.canonicalize() {
        Ok(now) if now == o.real => {}
        _ => return Err(changed()),
    }
    if fingerprint(&o.real).ok() != Some(o.seen) {
        return Err(changed());
    }
    Ok((o.real, o.opens))
}

/// The handler: checks, then one toast. UI thread.
pub(crate) fn run(weak: &slint::Weak<App>, args: &Value) -> Result<Value, String> {
    let ui = weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let offer = plan(args, &home)?;
    let id = id_for(&offer.real);
    for old in remember(&id, &offer) {
        crate::wire::toast::remove(&ui, &old);
    }
    crate::wire::toast::push(&ui, card(&id, &requester_now(), &offer, text(args, "title")), 1);
    let mut answer = json!({
        "presented": offer.real.display().to_string(),
        "card": "up",
        "opens_when": "the person presses Open",
    });
    if offer.opens == Opens::Apart {
        answer["note"] = Value::from(
            "their press opens it in a separate browser with no network and none of their sign-ins: \
             fonts, scripts, styles and pictures from the internet will not load, so say so if the page needs them",
        );
    }
    Ok(answer)
}

/// The Open button. UI thread, from `wire::notifications`' toast handler; the caller is the
/// person's press.
pub fn press(ui: &App, id: &str, action_id: &str) {
    // A toast's buttons draw on the lock screen too; a press there opens nothing.
    if crate::control::locked_screen(ui.get_current_screen()) {
        return;
    }
    if action_id == "open" {
        let said = what_a_press_opens(id).and_then(|(path, opens)| match opens {
            Opens::Apart => crate::open_url::open_file_offline(&path),
            Opens::App(_) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                crate::wire::open_with::launch(&crate::mime_dispatch::classify(&name), &name, &path, Some(ui));
                Ok(())
            }
        });
        if let Err(why) = said {
            crate::wire::toast::local(&ui.as_weak(), "Desktop", "Could not open that", &why, 1);
        }
    }
    crate::wire::toast::remove(ui, id);
}

#[cfg(test)]
#[path = "control_present_tests.rs"]
mod tests;
