//! `files_new_folder` / `files_new_file`: the checks on the UI thread, the disk on the socket's
//! side of `answer_later`, so a slow or hung mount cannot freeze the desktop. The answer is the
//! file-system fact (`created` / `existed`) and is settled; only the listing catches up after, by
//! a refresh queued onto the UI thread once the disk has changed.
//!
//! Where the entry goes (#1033): `path` (the entry itself), or `parent` and `name`, or `name` in
//! a folder. That folder is the person's Files screen for the person, whose screen comes forward
//! as it always did; for a Mind it is the Mind's own cursor (`control_files_cursor`), and the
//! person's screen is never read or touched. `path` and `parent` touch nobody's screen.

use serde_json::Value;

use crate::control_files_cursor::{self as cursor, Make};
use crate::control_files_create::{is_nested, make_checked, make_nested_checked};
use crate::control_files_mind as mind;
use crate::App;

pub(crate) fn make_here(weak: &slint::Weak<App>, args: &Value, folder: bool) -> Result<Value, String> {
    use slint::ComponentHandle;
    let ui = weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
    let who = cursor::caller_key();
    let given = |k: &str| args[k].as_str().is_some_and(|v| !v.trim().is_empty());
    let explicit = given("path") || given("parent");
    let name = args["name"].as_str().unwrap_or_default().to_string();
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
    // The folder a bare `name` is made in: the Mind's own, or the one on the person's screen.
    let tilde = !explicit && name.starts_with("~/");
    let fallback = match &who {
        cursor::Caller::Person => ui.get_file_browser_path().to_string(),
        // A Mind with no cursor names the folder; a `~/…` name does.
        mind if !explicit && !tilde => mind.folder().ok_or(cursor::NEEDS_PATH)?,
        mind => mind.folder().unwrap_or_default(),
    };
    // A `~/…` name does not depend on the folder; anything else is made inside one, which a mind
    // must be allowed to see. A nested path's levels are each judged on the worker, where they
    // exist one after another (`make_nested_checked`).
    let target = if tilde { Make::Levels(name.clone()) } else { cursor::make_target(args, folder, &fallback, &home)? };
    if let Make::In { label, name, .. } = &target {
        if is_nested(name) && !folder {
            return Err(format!(
                "`{name}` has folders in it; make the folder first with files_new_folder, then create the file there"
            ));
        }
        mind::here(label)?;
        if !is_nested(name) {
            mind::may_make(label, name)?;
        }
    } else if !folder {
        return Err(format!(
            "`{name}` has folders in it; make the folder first with files_new_folder, then create the file there"
        ));
    }
    // Only the person's own bare call goes to the Files screen: a Mind's never does, and a call
    // that names its folder needs no screen.
    if !who.is_mind() && !explicit {
        crate::control_files::ensure_files_screen(&ui);
    }
    let weak = ui.as_weak();
    // Who is asking is known here; the worker may not be able to tell, so it is carried in.
    let mind = who.is_mind();
    let work = move || {
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
        let answer = match target {
            Make::Levels(rel) => make_nested_checked(&home, &rel, mind, &home)?,
            Make::In { dir, name, .. } if is_nested(&name) => make_nested_checked(&dir, &name, mind, &home)?,
            Make::In { dir, name, .. } => make_checked(&dir, &name, folder, mind, &home)?,
        };
        if answer.get("created").is_some() {
            let _ = weak.upgrade_in_event_loop(|ui| ui.invoke_file_refresh());
        }
        Ok(answer)
    };
    cursor::deferred(work)
}
