//! Yantrik Spreadsheet (ySheets) — one sheet, typed into, worked out, opened from and saved to
//! CSV, with undo and redo, and a control surface that does the same things by name.
//!
//! A path on the command line (`%f` in the .desktop file) is opened at start; a second launch
//! with a path hands it to the window that is already open, through that window's `open`.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use slint::ComponentHandle;
use yantrik_app_runtime::control::{App, View};
use yantrik_app_runtime::prelude::*;

mod address;
mod agent_rule;
mod csv;
mod document;
mod files;
mod formula;
mod history;
mod picker;
mod render;
mod session;
mod sheet;
mod surface;
mod wire;

#[cfg(test)]
mod security_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod window_tests;

slint::include_modules!();

/// The id the window publishes its surface under, and its pid file.
const APP_ID: &str = "spreadsheet";

fn main() {
    init_tracing("yantrik-spreadsheet");

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let path = std::env::args_os().nth(1).map(|p| document::resolve(&p.to_string_lossy(), &cwd));

    // One window per app. `claim` has already asked the running one to show itself; a second
    // launch with a file hands the file over too, and says what the window answered.
    let Some(_instance) = instance::claim(APP_ID) else {
        if let Some(p) = path {
            hand_over(&p);
        }
        return;
    };

    let ui = SpreadsheetApp::new().unwrap();
    // The window's title bar is the app's own (#256).
    yantrik_app_runtime::window_chrome!(ui);
    let theme = theme::load();
    ui.global::<ThemeMode>().set_dark(theme.dark);
    ui.global::<AccentPreset>().set_index(theme.accent_index);

    let shared: session::Shared = Rc::new(RefCell::new(session::Session::new()));
    files::SESSION.with(|cell| *cell.borrow_mut() = Some(shared.clone()));
    wire::wire(&ui, &shared);
    serve(&ui, &shared);

    if let Some(p) = path {
        let mut s = shared.borrow_mut();
        // A failure is on the status line and in `describe.notice`.
        let _ = surface::open_path(&mut s, &p);
    }
    files::redraw(&ui, &shared.borrow());
    // Typing goes into A1 from the first key, without a click first.
    ui.invoke_edit_cell();

    run_until_closed(&ui, "yantrik-spreadsheet");
}

/// Publish `app.describe` / `app.act`. Each action runs on the window's thread and the window is
/// redrawn from the session after it, so what a mind did is what the person sees.
fn serve(ui: &SpreadsheetApp, shared: &session::Shared) {
    let (weak, s) = (ui.as_weak(), shared.clone());
    let mut app = App::new(APP_ID).describe(move || match (weak.upgrade(), s.try_borrow()) {
        (Some(_), Ok(s)) => surface::describe(&s),
        _ => View::new("ySheets — the window is closed or busy"),
    });
    for (spec, handler) in surface::actions() {
        let name = spec.schema()["name"].as_str().unwrap_or_default().to_string();
        let (weak, s) = (ui.as_weak(), shared.clone());
        app = app.action(spec, move |args| {
            let ui = weak.upgrade().ok_or("the ySheets window is closed")?;
            let mut s = s.try_borrow_mut().map_err(|_| "the window is busy with another request; try again")?;
            let answer = surface::run(&mut s, &name, handler, args);
            files::redraw(&ui, &s);
            answer
        });
    }
    app.serve();
}

/// Give the running window the file this launch was asked to open.
fn hand_over(path: &std::path::Path) {
    let request = serde_json::json!({ "action": "open", "args": { "path": path } });
    let client = SyncRpcClient::for_service(&yantrik_app_runtime::control::service_id_for(APP_ID))
        .with_timeout(Duration::from_secs(3));
    for _ in 0..20 {
        if let Ok(reply) = client.call("app.act", request.clone()) {
            if reply["accepted"] != true {
                eprintln!("ySheets did not open {}: {reply}", path.display());
            }
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("ySheets is already starting; try opening {} again.", path.display());
}
