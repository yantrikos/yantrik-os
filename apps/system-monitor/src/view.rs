//! The only code that writes the window: the model, drawn, for the parts that changed.

use yantrik_app::app::Changed;
use yantrik_app::rail::{self, RailView as _};
use yantrik_app::rows;
use yantrik_ipc_contracts::system_monitor::ProcessInfo;

use crate::figures::Figures;
use crate::model::{Model, PART_AI, PART_NOTICE, PART_PROCESSES, PART_READINGS};
use crate::SystemMonitorApp;

pub fn render(ui: &SystemMonitorApp, m: &Model, changed: &Changed) {
    if changed.has(PART_READINGS) || changed.has(PART_PROCESSES) {
        let figures = Figures::of(m);
        if changed.has(PART_READINGS) {
            readings(ui, &figures);
        }
        if changed.has(PART_PROCESSES) {
            processes(ui, m, figures.processes);
        }
    }
    if changed.has(PART_NOTICE) {
        ui.set_notice(m.notice().into());
    }
    if changed.has(PART_AI) {
        // The panel's open flag is the header's own (it opens on the press and closes on the
        // dismiss); only what is in it is drawn from here.
        ui.set_ai_is_working(m.ai.working);
        ui.set_ai_response(m.ai.response.as_str().into());
    }
    if changed.has(rail::PART) {
        ui.render_rail(&m.rail);
    }
}

/// The sample, on screen, at the precision drawn (`crate::figures`). Every property is set to its
/// value, which Slint compares, and every list through `rows::set`, so a sample that moves
/// nothing visible draws nothing.
fn readings(ui: &SystemMonitorApp, f: &Figures) {
    ui.set_cpu_measured(f.cpu_measured);
    ui.set_cpu_usage(f.cpu_usage);
    ui.set_load_avg_1(f.load_avg[0].clone());
    ui.set_load_avg_5(f.load_avg[1].clone());
    ui.set_load_avg_15(f.load_avg[2].clone());
    rows::set(ui.get_cpu_cores(), f.cores.clone(), |list| ui.set_cpu_cores(list));

    ui.set_memory_usage(f.memory_usage);
    ui.set_memory_used_text(f.memory_used.clone());
    ui.set_memory_total_text(f.memory_total.clone());
    ui.set_memory_available_text(f.memory_available.clone());
    ui.set_memory_cached_text(f.memory_cached.clone());
    ui.set_memory_buffers_text(f.memory_buffers.clone());
    ui.set_swap_usage(f.swap_usage);
    ui.set_swap_used_text(f.swap_used.clone());
    ui.set_swap_total_text(f.swap_total.clone());

    rows::set(ui.get_disks(), f.disks.clone(), |list| ui.set_disks(list));
    rows::set(ui.get_network_interfaces(), f.interfaces.clone(), |list| {
        ui.set_network_interfaces(list)
    });

    ui.set_uptime_text(f.uptime.clone());

    // What it adds up to: the status row.
    ui.set_status_lead(f.status_lead.clone());
    ui.set_status_rest(f.status_rest.clone());
    ui.set_status_thresholds(f.status_thresholds.clone());
    ui.set_status_needs_you(f.status_needs_you);
}

fn processes(ui: &SystemMonitorApp, m: &Model, rows: Vec<crate::MonitorProcessData>) {
    if ui.get_sort_column() != m.sort_column {
        ui.set_sort_column(m.sort_column);
    }
    // The box already holds what was typed; an intent's filter is written into it.
    if ui.get_process_search().as_str() != m.search {
        ui.set_process_search(m.search.as_str().into());
    }
    let shown = m.shown();
    rows::set(ui.get_processes(), rows, |list| ui.set_processes(list));
    // The list was just rebuilt under the selection, and the row the selection points at may
    // not have survived it. Dropping a stale pid here is what hides End and Force Kill with the
    // row they act on (#220). The selection is the list's own (a row press sets it), so it is
    // read from the window and held to the list drawn.
    ui.set_selected_process_pid(selection_after_refresh(ui.get_selected_process_pid(), &shown));
}

/// What the selection must be after the process list is refreshed.
///
/// End and Force Kill act on `selected-process-pid`, and the list under them is rebuilt every
/// poll: the selected process can end — here, or anywhere else on the machine — or a filter can
/// stop showing its row. The pid used to stay selected through all of it, so the buttons
/// remained on screen aimed at a process that was gone (#220). The selection now lives exactly
/// as long as the shown list does.
pub fn selection_after_refresh(selected: i32, procs: &[ProcessInfo]) -> i32 {
    if selected >= 0 && procs.iter().any(|p| p.pid as i32 == selected) {
        selected
    } else {
        -1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::fake::process;

    fn running(pid: u32, name: &str) -> ProcessInfo {
        process(pid, name, 1.0, 1.0)
    }

    #[test]
    fn a_selection_the_refreshed_list_still_shows_survives_it() {
        let procs = vec![running(7, "pi"), running(9, "chromium")];
        assert_eq!(selection_after_refresh(7, &procs), 7);
    }

    #[test]
    fn a_selection_whose_process_is_gone_is_dropped() {
        // The person selected a row and the process ended — here or anywhere else on the
        // machine. The next poll rebuilds the list without it, and End and Force Kill must hide
        // with the row they act on instead of staying aimed at a dead pid (#220).
        let procs = vec![running(9, "chromium")];
        assert_eq!(selection_after_refresh(7, &procs), -1);
    }

    #[test]
    fn a_selection_a_filter_hides_is_dropped() {
        // The list is filtered before it is drawn, so a row the search stopped showing is gone
        // from the list the buttons belong to.
        let matching = vec![running(9, "chromium")];
        assert_eq!(selection_after_refresh(7, &matching), -1);
    }

    #[test]
    fn nothing_selected_stays_nothing_selected() {
        assert_eq!(selection_after_refresh(-1, &[]), -1);
        assert_eq!(selection_after_refresh(-1, &[running(7, "pi")]), -1);
    }
}
