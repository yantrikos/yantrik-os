//! The window: its callbacks bound to commands, and the model drawn for the parts that changed.
//! The only code that writes the window.

use slint::SharedString;
use yantrik_app::app::Changed;
use yantrik_app::cx::Cx;
use yantrik_app::rail::{self, RailView as _};
use yantrik_app::rows;

use crate::command::Command;
use crate::engine::{self, Download, Totals};
use crate::model::{Model, PART_AI, PART_BANNER, PART_FORM, PART_LIST};
use crate::{DownloadItem, DownloadManagerApp, Downloads};

pub fn wire(cx: &Cx<Downloads>) {
    let ui = cx.ui();
    let default_dir = cx.read(|m| m.engine.default_dir().to_string_lossy().to_string());
    ui.set_add_save_dir_text(default_dir.into());

    let adding = cx.clone();
    ui.on_dl_add(move |url, sha256, save_dir| {
        adding.run(Command::Add { url: url.to_string(), sha256: sha256.to_string(), save_dir: save_dir.to_string() });
    });
    ui.on_dl_pause(cx.cmd1(Command::Pause));
    ui.on_dl_resume(cx.cmd1(Command::Resume));
    ui.on_dl_cancel(cx.cmd1(Command::Cancel));
    ui.on_dl_retry(cx.cmd1(Command::Retry));
    ui.on_dl_verify_checksum(cx.cmd1(|id: i32| Command::Verify { id, sha256: None }));
    ui.on_dl_open_folder(cx.cmd1(Command::OpenFolder));
    ui.on_dl_clear_completed(cx.cmd(|| Command::ClearCompleted));
    ui.on_dl_pause_all(cx.cmd(|| Command::PauseAll));
    ui.on_dl_resume_all(cx.cmd(|| Command::ResumeAll));
    // The two-way-bound ones: the tab strip and the sort menu write their own property and then
    // call the callback, so `render` setting it again is a no-op for a click and the whole gesture
    // for a caller that only has the callback.
    ui.on_dl_filter(cx.cmd1(Command::Filter));
    ui.on_dl_sort(cx.cmd1(Command::Sort));
    ui.on_dl_search(cx.cmd1(|query: SharedString| Command::Search(query.to_string())));
    ui.on_dl_toggle_select(cx.cmd1(Command::ToggleSelect));
    ui.on_dl_select_all(cx.cmd1(Command::SelectAll));
    ui.on_ai_explain_pressed(cx.cmd(|| Command::AiExplain));
    ui.on_ai_dismiss(cx.cmd(|| Command::AiDismiss));
    drop(ui);
    rail::wire(cx);
}

pub fn render(ui: &DownloadManagerApp, m: &Model, changed: &Changed) {
    if changed.has(PART_LIST) {
        list(ui, m);
    }
    if changed.has(PART_BANNER) {
        ui.set_error_text(m.banner.as_str().into());
        ui.set_error_is_info(m.banner_info);
    }
    // The person's own add took: the dialog closes and its fields are cleared.
    if changed.has(PART_FORM) && m.form_done {
        ui.set_add_url_open(false);
        ui.set_add_url_text("".into());
        ui.set_add_checksum_text("".into());
    }
    if changed.has(PART_AI) {
        ui.set_ai_is_working(false);
        ui.set_ai_response(m.ai_hint.clone().unwrap_or_default().into());
    }
    if changed.has(rail::PART) {
        ui.render_rail(&m.rail);
    }
}

/// The rows, the counters, and the detail panel's two single-valued properties.
fn list(ui: &DownloadManagerApp, m: &Model) {
    let items = m.items();
    let visible = engine::visible_rows(&items, m.filter, &m.search, m.sort);
    ui.set_dl_search_match_count(visible.len() as i32);
    let home = yantrik_app::place::home();
    let rows: Vec<DownloadItem> = visible.iter().map(|d| to_row(d, &home)).collect();
    // Set only when a row differs: a list at rest is not drawn again.
    rows::set(ui.get_downloads(), rows, |model| ui.set_downloads(model));

    let totals = Totals::of(&items);
    ui.set_active_count(totals.active as i32);
    ui.set_total_speed(format!("{}/s", engine::format_bytes(totals.speed_bps as u64)).into());
    // The window has run for one session, so everything finished in it finished today.
    ui.set_completed_today(totals.completed as i32);

    // The detail panel follows whichever row is open.
    let expanded = ui.get_dl_expanded_id();
    let focus = items.iter().find(|d| d.id == expanded).or_else(|| items.last());
    ui.set_dl_resume_supported(focus.map(|d| d.resume_supported).unwrap_or(false));
    ui.set_dl_checksum_result(focus.map(|d| d.file_hash.clone()).unwrap_or_default().into());

    // The controls the person writes: set only where they differ, so a search box keeps its caret.
    if ui.get_active_filter() != m.filter {
        ui.set_active_filter(m.filter);
    }
    if ui.get_dl_sort_mode() != m.sort {
        ui.set_dl_sort_mode(m.sort);
    }
    if ui.get_dl_search_text().as_str() != m.search {
        ui.set_dl_search_text(m.search.as_str().into());
    }
    if ui.get_dl_all_selected() != m.all_selected {
        ui.set_dl_all_selected(m.all_selected);
    }
}

fn to_row(download: &Download, home: &std::path::Path) -> DownloadItem {
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
        // The row's own line says where in words; the path and URL are in its Details.
        place: yantrik_app::place::place_of(&download.save_dir, home).into(),
        site: yantrik_app::place::site_of(&download.url).into(),
        start_time: download.started_at.clone().into(),
        end_time: download.ended_at.clone().into(),
        file_hash: download.file_hash.clone().into(),
        file_type: download.content_type.clone().into(),
    }
}
