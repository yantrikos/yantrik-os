//! Yantrik Download Manager — standalone app binary.
//!
//! Fetches files over HTTP, shows what each transfer is doing, verifies it against a checksum,
//! and publishes all of that on the control surface so an agent can fetch a file without a
//! terminal and read back where it landed and what its hash was.
//!
//! The window is a view onto [`engine::Engine`], which owns every transfer and runs each one on
//! its own thread. Nothing here blocks on the network: the callbacks below issue a command and
//! return, and a timer redraws the list when the engine says something changed.

mod engine;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use engine::{Download, Engine, Status};
use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};
use yantrik_app_runtime::prelude::*;

slint::include_modules!();

/// How many rows `describe` carries. The state is a glance, not a transcript — the true count
/// travels beside it, so a caller is never misled about how many there are.
const LISTING_CAP: usize = 20;

fn main() {
    init_tracing("yantrik-download-manager");

    // One window per app: a second launch defers to the running one (the shell focuses it).
    let Some(_instance) = instance::claim("download-manager") else { return };

    let app = DownloadManagerApp::new().unwrap();
    // The window's title bar is the app's own (#256).
    yantrik_app_runtime::window_chrome!(app);

    // Same dark/accent choice as the shell, read from the shell's settings file.
    let theme = theme::load();
    app.global::<ThemeMode>().set_dark(theme.dark);
    app.global::<AccentPreset>().set_index(theme.accent_index);

    // Reads the saved list before the window is shown, so the first `describe` and the first
    // paint both see what actually survived the last run.
    let engine = Engine::new();
    // Held for the life of the window: a dropped Slint timer stops.
    let _refresh_timer = wire(&app, engine.clone());
    publish_control(&app, engine.clone());

    run_until_closed(&app, "yantrik-download-manager");

    // The window has closed but the process is still here. Stop the writers and put the list down:
    // anything that was mid-transfer is recorded as such and reconciled on the way back in.
    engine.shutdown();
}

// ── One code path per command ───────────────────────────────────────

/// Show the outcome of one engine command in the window, and hand it back to whoever asked.
///
/// The button ignores the result and the control action returns it, but both go through here, so
/// an agent and a person cannot get different answers about whether something worked — the same
/// arrangement the terminal uses for its shared `exec_command`.
///
/// Deliberately *not* routed through `error_text`: the banner also carries the reason a download
/// failed earlier, so reading it back after a command would sometimes report an unrelated failure
/// as the command's own.
fn settle<T>(
    ui: &DownloadManagerApp,
    engine: &Engine,
    result: Result<T, String>,
) -> Result<T, String> {
    match &result {
        Ok(_) => ui.set_error_text("".into()),
        Err(reason) => ui.set_error_text(reason.as_str().into()),
    }
    // Whatever the state file had to say, the person has had it on screen and has now done
    // something else. Leaving it to be re-raised by the next timer tick would make a banner
    // about the last restart impossible to get past. `describe` keeps reporting it.
    engine.acknowledge_notice();
    refresh(ui, engine);
    result
}

/// Show the folder a download went to in the file manager.
fn open_folder(engine: &Engine, id: i32) -> Result<(), String> {
    let download = engine.get(id).ok_or_else(|| format!("no download {id}"))?;
    let dir = download.save_dir.clone();
    let opener = if cfg!(target_os = "windows") { "explorer" } else { "xdg-open" };
    std::process::Command::new(opener)
        .arg(&dir)
        .spawn()
        .map_err(|e| format!("cannot open {}: {e}", dir.display()))?;
    Ok(())
}

// ── The window ──────────────────────────────────────────────────────

fn wire(app: &DownloadManagerApp, engine: Engine) -> Timer {
    app.set_downloads(ModelRc::new(VecModel::<DownloadItem>::default()));
    app.set_add_save_dir_text(engine.default_dir().to_string_lossy().to_string().into());

    // ── Add ──
    {
        let weak = app.as_weak();
        let engine = engine.clone();
        app.on_dl_add(move |url, checksum, save_dir| {
            let Some(ui) = weak.upgrade() else { return };
            let started = settle(&ui, &engine, engine.add(&url, &checksum, Some(save_dir.as_str())));
            if let Ok(id) = started {
                tracing::info!(id, url = %url, "Download queued");
                // Only close the dialog on success; a rejected URL stays on screen to be fixed.
                ui.set_add_url_open(false);
                ui.set_add_url_text("".into());
                ui.set_add_checksum_text("".into());
            }
        });
    }

    // ── Per-download commands ──
    //
    // Non-capturing closures coerce to `fn`, so every one of these shares one wiring path and the
    // only thing that differs between them is the engine method named.
    app.on_dl_pause(id_command(app, &engine, |engine, id| engine.pause(id)));
    app.on_dl_resume(id_command(app, &engine, |engine, id| engine.resume(id)));
    app.on_dl_cancel(id_command(app, &engine, |engine, id| engine.cancel(id)));
    app.on_dl_retry(id_command(app, &engine, |engine, id| engine.retry(id)));
    app.on_dl_verify_checksum(id_command(app, &engine, |engine, id| engine.verify(id, None)));
    app.on_dl_open_folder(id_command(app, &engine, open_folder));

    // ── List-wide commands ──
    {
        let weak = app.as_weak();
        let engine = engine.clone();
        app.on_dl_clear_completed(move || {
            let Some(ui) = weak.upgrade() else { return };
            let cleared = engine.clear_completed();
            tracing::info!(cleared, "Cleared finished downloads");
            let _ = settle(&ui, &engine, Ok::<usize, String>(cleared));
        });
    }
    {
        let weak = app.as_weak();
        let engine = engine.clone();
        app.on_dl_pause_all(move || {
            let Some(ui) = weak.upgrade() else { return };
            engine.pause_all();
            let _ = settle(&ui, &engine, Ok::<(), String>(()));
        });
    }
    {
        let weak = app.as_weak();
        let engine = engine.clone();
        app.on_dl_resume_all(move || {
            let Some(ui) = weak.upgrade() else { return };
            engine.resume_all();
            let _ = settle(&ui, &engine, Ok::<(), String>(()));
        });
    }

    // ── View controls ──
    //
    // These are the two-way-bound ones: the tab strip and the sort menu write their own property
    // *and then* call the callback, so setting it again here is a no-op for a click and the whole
    // gesture for a caller that only has the callback.
    {
        let weak = app.as_weak();
        let engine = engine.clone();
        app.on_dl_filter(move |filter| {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_active_filter(filter);
            refresh(&ui, &engine);
        });
    }
    {
        let weak = app.as_weak();
        let engine = engine.clone();
        app.on_dl_sort(move |mode| {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_dl_sort_mode(mode);
            refresh(&ui, &engine);
        });
    }
    {
        let weak = app.as_weak();
        let engine = engine.clone();
        app.on_dl_search(move |query| {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_dl_search_text(query);
            refresh(&ui, &engine);
        });
    }

    // ── Selection ──
    {
        let weak = app.as_weak();
        let engine = engine.clone();
        app.on_dl_toggle_select(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            let now_selected = engine.get(id).map(|d| !d.selected).unwrap_or(false);
            engine.set_selected(id, now_selected);
            refresh(&ui, &engine);
        });
    }
    {
        let weak = app.as_weak();
        let engine = engine.clone();
        app.on_dl_select_all(move |selected| {
            let Some(ui) = weak.upgrade() else { return };
            engine.select_all(selected);
            ui.set_dl_all_selected(selected);
            refresh(&ui, &engine);
        });
    }

    // ── AI assist ──
    //
    // The companion lives in the shell, so this is an ordinary RPC and not a stub: the button
    // hands over the rows as rows and shows whatever comes back. Run on its own, with no shell,
    // it says that instead of spinning — a control that cannot work has to say why.
    {
        let weak = app.as_weak();
        let engine = engine.clone();
        app.on_ai_explain_pressed(move || {
            let Some(ui) = weak.upgrade() else { return };
            if let Some(hint) = companion::reach().hint() {
                ui.set_ai_is_working(false);
                ui.set_ai_response(hint.into());
                return;
            }
            // The list, handed over as the list. Describing it in prose first and asking the model
            // to re-derive the numbers is how an app starts reporting sizes nobody measured.
            let prompt = format!(
                "These are the downloads on my machine right now:\n{}\nIn at most three short \
                 lines say what is worth my attention and what to do about it. Use only these \
                 rows; do not guess at causes you cannot see.",
                ai_facts(&engine)
            );
            ui.set_ai_is_working(true);
            ui.set_ai_response("".into());
            let back = ui.as_weak();
            std::thread::spawn(move || {
                let outcome = companion::ask(&prompt);
                let _ = back.upgrade_in_event_loop(move |ui| {
                    ui.set_ai_is_working(false);
                    // A failure goes in the same panel the answer would have: the button was
                    // pressed, so something has to appear there.
                    ui.set_ai_response(
                        match outcome {
                            Ok(text) => text,
                            Err(e) => e.to_string(),
                        }
                        .into(),
                    );
                });
            });
        });
    }
    {
        let weak = app.as_weak();
        app.on_ai_dismiss(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_ai_panel_open(false);
            }
        });
    }

    // ── Redraw ──
    //
    // The workers change shared state, not the window. This ticks four times a second and does
    // nothing at all unless the engine's revision moved, so an idle window costs one atomic load
    // per tick and a busy one repaints at a rate a person can actually read.
    let timer = Timer::default();
    {
        let weak = app.as_weak();
        let engine = engine.clone();
        // Not 0: the first tick must draw the empty list's counters.
        let seen = Rc::new(RefCell::new(u64::MAX));
        timer.start(TimerMode::Repeated, Duration::from_millis(250), move || {
            let Some(ui) = weak.upgrade() else { return };
            let revision = engine.revision();
            if *seen.borrow() == revision {
                return;
            }
            *seen.borrow_mut() = revision;
            refresh(&ui, &engine);
        });
    }
    timer
}

/// Wire one `callback(int)` to one engine command.
fn id_command(
    app: &DownloadManagerApp,
    engine: &Engine,
    command: fn(&Engine, i32) -> Result<(), String>,
) -> impl FnMut(i32) + 'static {
    let weak = app.as_weak();
    let engine = engine.clone();
    move |id| {
        let Some(ui) = weak.upgrade() else { return };
        let _ = settle(&ui, &engine, command(&engine, id));
    }
}

/// Rebuild the list and the counters from the engine.
fn refresh(ui: &DownloadManagerApp, engine: &Engine) {
    let items = engine.snapshot();
    let query = ui.get_dl_search_text();
    let rows = engine::visible_rows(&items, ui.get_active_filter(), query.as_str(), ui.get_dl_sort_mode());

    ui.set_dl_search_match_count(rows.len() as i32);
    let model: Vec<DownloadItem> = rows.iter().map(to_row).collect();
    ui.set_downloads(ModelRc::new(VecModel::from(model)));

    let totals = engine.totals();
    ui.set_active_count(totals.active as i32);
    ui.set_total_speed(format!("{}/s", engine::format_bytes(totals.speed_bps as u64)).into());
    // The window has run for one session, so everything finished in it finished today.
    ui.set_completed_today(totals.completed as i32);

    // The detail panel's two single-valued properties follow whichever row is open.
    let expanded = ui.get_dl_expanded_id();
    let focus = items.iter().find(|d| d.id == expanded).or_else(|| items.last());
    ui.set_dl_resume_supported(focus.map(|d| d.resume_supported).unwrap_or(false));
    ui.set_dl_checksum_result(focus.map(|d| d.file_hash.clone()).unwrap_or_default().into());

    // A download that failed while nobody was looking still has to say why. It yields to a live
    // command's own message, which `settle` has already put there.
    if ui.get_error_text().is_empty() {
        // The state file leads. A list that could not be read or written, or one that came back
        // changed, outlives any one transfer, and it is the thing that explains why the window
        // looks different from the way the person left it.
        if let Some(notice) = engine.unseen_notice() {
            ui.set_error_text(notice.as_str().into());
        } else if let Some(failed) = items
            .iter()
            .rev()
            .find(|d| d.status == Status::Failed && !d.error.is_empty())
        {
            ui.set_error_text(format!("{} — {}", failed.filename, failed.error).into());
        }
    }
}

/// The list as lines a model can read, capped the same way `describe` is.
///
/// One line a download, with the words the surface already publishes. A row that is restored or
/// interrupted says so, because "this has been sitting paused since your last session" is exactly
/// the kind of thing worth being told and nothing else in the prompt would reveal it.
fn ai_facts(engine: &Engine) -> String {
    let items = engine.snapshot();
    if items.is_empty() {
        return "(nothing queued)".to_string();
    }
    let mut lines: Vec<String> = items
        .iter()
        .rev()
        .take(LISTING_CAP)
        .map(|d| {
            let mut line = format!("- {} — {}, {}", d.filename, d.status.as_str(), d.size_text());
            if d.interrupted {
                line.push_str(", interrupted by the last shutdown");
            } else if d.restored {
                line.push_str(", from a previous session");
            }
            if d.checksum_status == "fail" {
                line.push_str(", checksum does not match");
            }
            if !d.error.is_empty() {
                line.push_str(&format!(", error: {}", d.error));
            }
            line
        })
        .collect();
    if items.len() > LISTING_CAP {
        lines.push(format!("- (and {} more not listed)", items.len() - LISTING_CAP));
    }
    lines.join("\n")
}

fn to_row(download: &Download) -> DownloadItem {
    DownloadItem {
        id: download.id,
        filename: download.filename.clone().into(),
        url: download.url.clone().into(),
        progress: download.progress(),
        speed_text: download.speed_text().into(),
        size_text: download.size_text().into(),
        status: download.status.as_str().into(),
        eta_text: download.eta_text().into(),
        is_selected: download.selected,
        checksum_expected: download.checksum_expected.clone().into(),
        checksum_status: download.checksum_status.clone().into(),
        save_dir: download.save_dir.to_string_lossy().to_string().into(),
        start_time: download.started_at.clone().into(),
        end_time: download.ended_at.clone().into(),
        file_hash: download.file_hash.clone().into(),
        file_type: download.content_type.clone().into(),
    }
}

// ── The control surface ─────────────────────────────────────────────
//
// What this app can say that nothing else can: whether the file arrived, where it is, and whether
// its hash matches. An agent asked to fetch something had to shell out to `curl` and then work out
// for itself whether the result was whole; here it queues the URL, watches one number, and reads
// back a path and a SHA-256 it did not have to compute.
//
// `add`, `resume`, `retry` and `verify` all declare `defers`: each hands the work to a thread and
// returns long before the file exists. A caller that treated the call as the result would report a
// 4 GB image as downloaded the instant it started.

/// One line a person could read, with whatever is worth knowing first.
fn summary(items: &[Download], totals: engine::Totals) -> String {
    if items.is_empty() {
        return "Downloads — nothing queued".to_string();
    }
    // Trouble leads, the same way a failed service leads the shell's summary.
    if let Some(failed) = items.iter().rev().find(|d| d.status == Status::Failed) {
        let reason = if failed.error.is_empty() { "failed".into() } else { failed.error.clone() };
        return format!(
            "Downloads — {} failed ({}: {}), {} active",
            totals.failed, failed.filename, reason, totals.active
        );
    }
    // A file that was fetched and is now gone outranks everything below: it is the one state the
    // person cannot discover by looking at the window's progress bars.
    if totals.missing > 0 {
        let gone = items.iter().find(|d| d.status == Status::Missing);
        return format!(
            "Downloads — {} finished file(s) no longer on disk{}",
            totals.missing,
            gone.map(|d| format!(" ({})", d.filename)).unwrap_or_default()
        );
    }
    if let Some(mismatch) = items.iter().find(|d| d.checksum_status == "fail") {
        return format!("Downloads — {} downloaded but its checksum does not match", mismatch.filename);
    }
    if totals.active > 0 {
        let leader = items
            .iter()
            .find(|d| d.status == Status::Downloading)
            .map(|d| {
                if d.total.is_some() {
                    format!(", {} at {}%", d.filename, (d.progress() * 100.0).round() as i32)
                } else {
                    format!(", {} ({} so far)", d.filename, engine::format_bytes(d.downloaded))
                }
            })
            .unwrap_or_default();
        return format!(
            "Downloads — {} active at {}/s{leader}",
            totals.active,
            engine::format_bytes(totals.speed_bps as u64)
        );
    }
    if totals.paused > 0 {
        // "Paused" and "was running when the app died" are the same row to the engine and very
        // different news to whoever left it running.
        let interrupted = items.iter().filter(|d| d.interrupted).count();
        let held = if interrupted > 0 {
            format!("{} paused ({interrupted} interrupted by a restart)", totals.paused)
        } else {
            format!("{} paused", totals.paused)
        };
        return format!("Downloads — {held}, {} finished", totals.completed);
    }
    format!("Downloads — {} finished, nothing running", totals.completed)
}

fn row_json(download: &Download) -> serde_json::Value {
    let mut row = serde_json::json!({
        "id": download.id,
        "filename": download.filename,
        "status": download.status.as_str(),
        "size": download.size_text(),
        "path": download.path().to_string_lossy(),
        "url": download.url,
    });
    let map = row.as_object_mut().expect("row is an object");
    if download.total.is_some() {
        map.insert("percent".into(), ((download.progress() * 100.0).round() as i64).into());
    }
    if download.status == Status::Downloading {
        map.insert("speed".into(), format!("{}/s", engine::format_bytes(download.speed_bps as u64)).into());
        if let Some(eta) = download.eta_secs {
            map.insert("eta".into(), engine::format_duration(eta).into());
        }
    }
    // Only said when true, so a caller reading a row can take their presence as the claim. A mind
    // coming back to a machine it left needs to tell "paused because I paused it" from "paused
    // because the process died under it", and only the second one is news.
    if download.interrupted {
        map.insert("interrupted".into(), true.into());
    }
    if download.restored {
        map.insert("restored".into(), true.into());
    }
    if !download.file_hash.is_empty() {
        map.insert("sha256".into(), download.file_hash.clone().into());
    }
    if download.checksum_status != "none" {
        map.insert("checksum".into(), download.checksum_status.clone().into());
    }
    if !download.error.is_empty() {
        map.insert("error".into(), download.error.clone().into());
    }
    row
}

fn publish_control(app: &DownloadManagerApp, engine: Engine) {
    use yantrik_app_runtime::control::{Action, App, Param, View};

    let describe = {
        let engine = engine.clone();
        move || {
            let items = engine.snapshot();
            let totals = engine.totals();
            let shown: Vec<serde_json::Value> = items.iter().rev().take(LISTING_CAP).map(row_json).collect();
            View::new(summary(&items, totals))
                .with("downloads", serde_json::Value::Array(shown))
                .with("shown", items.len().min(LISTING_CAP) as i64)
                .with("total", items.len() as i64)
                .with("active", totals.active as i64)
                .with("paused", totals.paused as i64)
                .with("completed", totals.completed as i64)
                .with("failed", totals.failed as i64)
                .with("missing", totals.missing as i64)
                // How much of this list survived a restart, and where it survived in. A caller
                // that finds `restored` rows knows the window is not starting from nothing, and
                // `state_file` is where to look when it disagrees with what it expected.
                .with("restored", totals.restored as i64)
                .with("state_file", engine.state_path().to_string_lossy().to_string())
                .with("notice", engine.notice())
                .with("speed", format!("{}/s", engine::format_bytes(totals.speed_bps as u64)))
                .with("save_dir", engine.default_dir().to_string_lossy().to_string())
        }
    };

    /// Pull an `id` out of the arguments, saying what was wrong rather than defaulting to zero.
    fn id_arg(args: &serde_json::Value) -> Result<i32, String> {
        args["id"]
            .as_i64()
            .map(|id| id as i32)
            .ok_or_else(|| "`id` must be the number of a download, as `describe` reports it".to_string())
    }

    let ui = app.as_weak();
    let window = move || ui.upgrade().ok_or_else(|| "Download Manager window is gone".to_string());

    let add = {
        let engine = engine.clone();
        let window = window.clone();
        move |args: &serde_json::Value| {
            let ui = window()?;
            let url = args["url"].as_str().unwrap_or_default().trim().to_string();
            let save_dir = args["save_dir"].as_str().unwrap_or_default();
            let save_dir = if save_dir.trim().is_empty() {
                ui.get_add_save_dir_text().to_string()
            } else {
                save_dir.to_string()
            };
            let checksum = args["sha256"].as_str().unwrap_or_default();
            let id = settle(&ui, &engine, engine.add(&url, checksum, Some(save_dir.as_str())))?;
            let download = engine.get(id).ok_or("the download vanished as it was queued")?;
            Ok(serde_json::json!({
                "id": id,
                "filename": download.filename,
                "path": download.path().to_string_lossy(),
                "status": download.status.as_str(),
            }))
        }
    };

    /// Build a handler that runs one engine command and reports the download's new state.
    fn reporting(
        engine: Engine,
        window: impl Fn() -> Result<DownloadManagerApp, String> + Clone + 'static,
        command: fn(&Engine, i32) -> Result<(), String>,
    ) -> impl Fn(&serde_json::Value) -> Result<serde_json::Value, String> + 'static {
        move |args| {
            let ui = window()?;
            let id = id_arg(args)?;
            settle(&ui, &engine, command(&engine, id))?;
            let download = engine.get(id).ok_or_else(|| format!("no download {id}"))?;
            Ok(row_json(&download))
        }
    }

    App::new("download-manager")
        .describe(describe)
        .action(
            Action::new("add", "Download a URL to this machine")
                .defers()
                .arg(Param::text("url").describe("The http(s) URL to fetch"))
                .arg(
                    Param::text("save_dir")
                        .optional()
                        .describe("Directory to save into; defaults to ~/Downloads"),
                )
                .arg(
                    Param::text("sha256")
                        .optional()
                        .describe("Expected SHA-256; the file is checked against it when it finishes"),
                ),
            add,
        )
        .action(
            // Defers, and the live run is what proved it: pausing sets a flag the worker reads at
            // its next chunk boundary, so the row this returns can still say `downloading` — and
            // does. The stop is certain; its arrival is not instant, and a caller that treated the
            // reply as the settled state would misreport a transfer that is still writing.
            Action::new("pause", "Stop a running download, keeping what has arrived")
                .defers()
                .arg(Param::integer("id")),
            reporting(engine.clone(), window.clone(), |engine, id| engine.pause(id)),
        )
        .action(
            Action::new("resume", "Continue a paused or failed download")
                .defers()
                .arg(Param::integer("id")),
            reporting(engine.clone(), window.clone(), |engine, id| engine.resume(id)),
        )
        .action(
            // `sensitive`, not `standard`: it interrupts work in progress and deletes the partial
            // file. Not `dangerous` — nothing a person made is lost, only bytes that can be
            // fetched again. Defers for the same reason `pause` does: a running transfer stops at
            // its next chunk, and the file goes with it then.
            Action::new("cancel", "Stop a download and delete the partial file")
                .risk("sensitive")
                .defers()
                .arg(Param::integer("id")),
            reporting(engine.clone(), window.clone(), |engine, id| engine.cancel(id)),
        )
        .action(
            Action::new("retry", "Start a failed download again from the beginning")
                .defers()
                .arg(Param::integer("id")),
            reporting(engine.clone(), window.clone(), |engine, id| engine.retry(id)),
        )
        .action(
            Action::new("verify", "Hash a finished file again and compare it with a checksum")
                .defers()
                .arg(Param::integer("id"))
                .arg(
                    Param::text("sha256")
                        .optional()
                        .describe("Expected SHA-256; omit to re-check the one already recorded"),
                ),
            {
                let engine = engine.clone();
                let window = window.clone();
                move |args: &serde_json::Value| {
                    let ui = window()?;
                    let id = id_arg(args)?;
                    let expected = args["sha256"].as_str().filter(|s| !s.trim().is_empty());
                    settle(&ui, &engine, engine.verify(id, expected))?;
                    let download = engine.get(id).ok_or_else(|| format!("no download {id}"))?;
                    Ok(row_json(&download))
                }
            },
        )
        .action(Action::new("pause_all", "Stop every running download").defers(), {
            let engine = engine.clone();
            let window = window.clone();
            move |_: &serde_json::Value| {
                let ui = window()?;
                engine.pause_all();
                let _ = settle(&ui, &engine, Ok::<(), String>(()));
                Ok(serde_json::json!({ "paused": engine.totals().paused }))
            }
        })
        .action(Action::new("resume_all", "Continue every paused download").defers(), {
            let engine = engine.clone();
            let window = window.clone();
            move |_: &serde_json::Value| {
                let ui = window()?;
                engine.resume_all();
                let _ = settle(&ui, &engine, Ok::<(), String>(()));
                Ok(serde_json::json!({ "active": engine.totals().active }))
            }
        })
        .action(
            Action::new("clear_completed", "Drop finished downloads from the list, keeping the files"),
            {
                let engine = engine.clone();
                let window = window.clone();
                move |_: &serde_json::Value| {
                    let ui = window()?;
                    let cleared = engine.clear_completed();
                    let _ = settle(&ui, &engine, Ok::<(), String>(()));
                    Ok(serde_json::json!({ "cleared": cleared }))
                }
            },
        )
        .action(
            Action::new("open_folder", "Show a download's folder in the file manager")
                .arg(Param::integer("id")),
            reporting(engine, window, open_folder),
        )
        .serve();
}
