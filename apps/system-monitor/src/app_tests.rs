//! System Monitor run headless through `yantrik-testing`: a fake machine, a fake Mind, the real
//! dispatch, and jobs and timers that move only when a test says so.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::mind::Reach;
use yantrik_app::watch::Seen;
use yantrik_app::{Paths, Services};
use yantrik_testing::{Call, TestApp};

use crate::machine::fake::{self, process, FakeMachine};
use crate::outcome::{Provenance, Signal};
use crate::report;
use crate::sample::Reading;
use crate::{Command, SystemMonitor, POLL_EVERY};

fn readings_of(procs: Vec<yantrik_ipc_contracts::system_monitor::ProcessInfo>, from: Provenance) -> Reading {
    fake::reading(fake::snapshot(Some(2000)), procs, from)
}

/// The machine as most tests see it: the service answered, three processes, chromium busiest.
pub(crate) fn normal() -> Reading {
    readings_of(
        vec![
            process(4242, "chromium", 30.04, 12.36),
            process(4300, "chrome_crashpad", 2.0, 0.5),
            process(77, "pi", 1.0, 0.2),
        ],
        Provenance::service(),
    )
}

fn start_with(machine: FakeMachine, mind: FakeMind) -> (TestApp<SystemMonitor>, Arc<FakeMachine>, Arc<FakeMind>) {
    let paths = Paths::temporary(<SystemMonitor as yantrik_app::App>::ID);
    let machine = Arc::new(machine);
    fake::install(&paths, machine.clone());
    let mind = Arc::new(mind);
    let app = TestApp::<SystemMonitor>::start_at_with(paths, Vec::new(), mind.clone(), Services::fakes());
    (app, machine, mind)
}

pub(crate) fn start(machine: FakeMachine) -> (TestApp<SystemMonitor>, Arc<FakeMachine>) {
    let (app, machine, _) = start_with(machine, FakeMind::new());
    (app, machine)
}

// ── describe, in each case ──────────────────────────────────────────

#[test]
fn in_private_mode_describe_keeps_the_machines_load_and_names_no_process() {
    use yantrik_app::control::private_mode::{testing::pin, PRIVATE_SUMMARY};
    let (app, _) = start(FakeMachine::new().reads(normal()));
    app.run_jobs();
    pin(&app.paths(), true);
    let d = app.describe();
    assert_eq!(d["summary"], json!(PRIVATE_SUMMARY));
    let mut keys: Vec<&str> = d["state"].as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["cpu_percent", "cpu_window_ms", "load_average", "memory_percent", "open", "private", "swap_percent", "windows_open"]
    );
    assert!(!d.to_string().contains("chromium"), "{d}");
    pin(&app.paths(), false);
    assert_eq!(app.describe()["state"]["top_processes"][0]["name"], json!("chromium"));
}

#[test]
fn before_the_first_reading_describe_says_there_is_none() {
    let (app, _) = start(FakeMachine::new().reads(normal()));
    // The first reading is asked for at start and has not landed.
    assert_eq!(app.jobs_waiting(), 1);
    let d = app.describe();
    assert_eq!(d["summary"], json!("System — no reading yet; memory  of ; up "));
    let s = &d["state"];
    assert_eq!(s["source"], json!("service"));
    assert_eq!(s["degraded"], json!(false));
    assert_eq!(s["degraded_reason"], json!(null));
    assert_eq!(s["notice"], json!(""));
    assert_eq!(s["cpu_percent"], json!(null));
    assert_eq!(s["cpu_window_ms"], json!(null));
    assert_eq!(s["cpu_model"], json!(null));
    assert_eq!(s["load_average"], json!(["0.00", "0.00", "0.00"]));
    assert_eq!(s["memory_percent"], json!(0.0));
    assert_eq!(s["memory_used"], json!(""));
    assert_eq!(s["swap_percent"], json!(0.0));
    assert_eq!(s["uptime"], json!(""));
    assert_eq!(s["process_filter"], json!(""));
    assert_eq!(s["top_processes"], json!([]));
    assert_eq!(s["disks"], json!([]));
    assert_eq!(s["interfaces"], json!([]));
    assert_eq!(s["health"], json!("No limits reached"));
    assert_eq!(s["health_summary"], json!(""));
    assert_eq!(s["status"], json!(""));
    assert_eq!(s["limits_reached"], json!([]));
    assert_eq!(s["thresholds"], json!(""));
}

#[test]
fn a_reading_from_the_service_is_drawn_and_described_value_for_value() {
    let (app, machine) = start(FakeMachine::new().reads(normal()));
    app.run_jobs();
    assert_eq!(machine.asked(), vec![("cpu", 100)]);
    let snap = fake::snapshot(Some(2000));
    let line = report::assess(&snap).line();
    let d = app.describe();
    assert_eq!(d["summary"], json!(format!("System — {line}; memory 8.0 GB of 16.0 GB; up 3h 25m")));
    let s = &d["state"];
    assert_eq!(s["source"], json!("service"));
    assert_eq!(s["degraded"], json!(false));
    assert_eq!(s["notice"], json!(""));
    assert_eq!(s["cpu_percent"], json!(12.3));
    assert_eq!(s["cpu_window_ms"], json!(2000));
    assert_eq!(s["cpu_model"], json!(null));
    assert_eq!(s["load_average"], json!(["1.15", "1.00", "0.90"]));
    assert_eq!(s["memory_percent"], json!(50.0));
    assert_eq!(s["memory_used"], json!("8.0 GB"));
    assert_eq!(s["memory_total"], json!("16.0 GB"));
    assert_eq!(s["memory_available"], json!("8.0 GB"));
    assert_eq!(s["swap_percent"], json!(25.0));
    assert_eq!(s["uptime"], json!("3h 25m"));
    assert_eq!(s["status"], json!(line));
    assert_eq!(s["health_summary"], json!(line));
    // The window's own `f32`s, rounded as they always were.
    assert_eq!(
        s["top_processes"][0],
        json!({ "pid": 4242, "name": "chromium", "cpu_percent": 30.0f32, "mem_percent": 12.4f32, "status": "S" })
    );
    assert_eq!(s["top_processes"].as_array().unwrap().len(), 3);
    assert_eq!(s["disks"], json!([{ "mount": "/", "used": "10.0 GB", "total": "31.3 GB", "percent": 31.9f32 }]));
    assert_eq!(s["interfaces"], json!([{ "name": "eth0", "ip": null }]));
    // And the window says the same.
    app.ui(|ui| {
        assert!(ui.get_cpu_measured());
        // Held to what is drawn, "12%" (crate::figures); describe keeps the tenth above.
        assert_eq!(ui.get_cpu_usage(), 12.0f32);
        let top = slint::Model::row_data(&ui.get_processes(), 0).unwrap();
        assert_eq!((top.cpu_percent, top.mem_percent), (30.0f32, 12.4f32), "as describe says");
        assert_eq!(ui.get_memory_used_text(), "8.0 GB");
        assert_eq!(ui.get_uptime_text(), "3h 25m");
        assert_eq!(ui.get_load_avg_5(), "1.00");
        assert_eq!(ui.get_status_rest().as_str(), report::assess(&fake::snapshot(Some(2000))).rest);
        assert_eq!(slint::Model::row_count(&ui.get_processes()), 3);
        assert_eq!(slint::Model::row_count(&ui.get_cpu_cores()), 4);
        assert_eq!(slint::Model::row_data(&ui.get_network_interfaces(), 0).unwrap().rx_speed, "2.0 KB/s");
    });
}

#[test]
fn a_first_reading_has_no_cpu_figure_and_draws_no_cores() {
    let first = fake::reading(fake::snapshot(None), Vec::new(), Provenance::service());
    let (app, _) = start(FakeMachine::new().reads(first));
    app.run_jobs();
    let s = app.describe()["state"].clone();
    assert_eq!(s["cpu_percent"], json!(null));
    assert_eq!(s["cpu_window_ms"], json!(null));
    app.ui(|ui| {
        assert!(!ui.get_cpu_measured());
        assert_eq!(slint::Model::row_count(&ui.get_cpu_cores()), 0);
    });
}

#[test]
fn a_reading_the_service_did_not_answer_says_so_on_screen_and_to_the_mind() {
    let local = readings_of(vec![process(9, "sh", 0.0, 0.1)], Provenance::local("connection refused"));
    let (app, _) = start(FakeMachine::new().reads(local));
    app.run_jobs();
    let s = app.describe()["state"].clone();
    let said = "Readings are local: the system-monitor service did not answer (connection refused)";
    assert_eq!(s["source"], json!("local"));
    assert_eq!(s["degraded"], json!(true));
    assert_eq!(s["degraded_reason"], json!("connection refused"));
    assert_eq!(s["notice"], json!(said));
    assert_eq!(app.ui(|ui| ui.get_notice()), said);
}

// ── the readings' job ───────────────────────────────────────────────

#[test]
fn one_reading_at_a_time_every_two_seconds_in_the_order_on_screen() {
    let (app, machine) = start(FakeMachine::new().reads(normal()));
    assert_eq!(app.jobs_waiting(), 1);
    // A service slower than the poll: the next tick finds a reading in flight and asks nothing.
    app.advance(POLL_EVERY);
    assert_eq!(app.jobs_waiting(), 1);
    app.run_jobs();
    // The memory header pressed: the next reading is asked for in that order.
    app.ui(|ui| ui.invoke_sort_by_column(1));
    assert_eq!(app.ui(|ui| ui.get_sort_column()), 1);
    app.advance(POLL_EVERY);
    assert_eq!(app.jobs_waiting(), 1);
    app.run_jobs();
    assert_eq!(machine.asked(), vec![("cpu", 100), ("mem", 100)]);
}

#[test]
fn a_stale_or_repeated_reading_is_dropped() {
    let (app, machine) = start(FakeMachine::new().reads(normal()));
    app.run_jobs();
    let before = app.describe();
    let other = readings_of(vec![process(5, "other", 99.0, 99.0)], Provenance::local("gone"));
    // The reading that already landed, delivered again.
    app.run(Command::Sampled { seq: 1, reading: other.clone() });
    assert_eq!(app.describe()["state"], before["state"]);
    // One for a reading nobody is waiting on, while another is in flight.
    machine.will_read(normal());
    app.advance(POLL_EVERY);
    app.run(Command::Sampled { seq: 99, reading: other });
    assert_eq!(app.describe()["state"]["top_processes"][0]["name"], json!("chromium"));
    assert_eq!(app.describe()["state"]["source"], json!("service"));
    app.run_jobs();
    assert_eq!(app.model(|m| m.sampling), None);
}

#[test]
fn a_reading_whose_worker_panics_is_asked_for_again_and_the_app_runs_on() {
    let (app, _) = start(FakeMachine::new().panicking());
    app.run_jobs();
    assert_eq!(app.model(|m| m.sampling), None, "a panicked reading is not left in flight");
    assert_eq!(app.describe()["summary"], json!("System — no reading yet; memory  of ; up "));
    app.advance(POLL_EVERY);
    assert_eq!(app.jobs_waiting(), 1, "the next tick asks again");
}

// ── the search box ──────────────────────────────────────────────────

#[test]
fn a_key_in_the_search_box_narrows_the_list_at_once_and_drops_a_hidden_selection() {
    let (app, _) = start(FakeMachine::new().reads(normal()));
    app.run_jobs();
    app.ui(|ui| ui.set_selected_process_pid(77));
    // As the markup does it: the in-out text, then the callback.
    app.ui(|ui| {
        ui.set_process_search("CHROM".into());
        ui.invoke_process_search_changed("CHROM".into());
    });
    assert_eq!(app.jobs_waiting(), 0, "no poll was needed");
    assert_eq!(app.ui(|ui| slint::Model::row_count(&ui.get_processes())), 2);
    assert_eq!(app.ui(|ui| ui.get_selected_process_pid()), -1, "End and Force Kill hide with the row");
    let s = app.describe()["state"].clone();
    assert_eq!(s["process_filter"], json!("CHROM"));
    assert_eq!(s["top_processes"].as_array().unwrap().len(), 2);
    // A selection the list still shows survives a reading.
    app.ui(|ui| ui.set_selected_process_pid(4242));
    app.advance(POLL_EVERY);
    app.run_jobs();
    assert_eq!(app.ui(|ui| ui.get_selected_process_pid()), 4242);
}

// ── the intents ─────────────────────────────────────────────────────

#[test]
fn sort_and_filter_answer_as_they_did_and_refuse_what_they_cannot_do() {
    let (app, machine) = start(FakeMachine::new().reads(normal()));
    app.run_jobs();
    let reply = app.act("sort_processes", json!({ "by": "Memory" })).unwrap().settle().unwrap();
    assert_eq!(reply, json!({ "sorted_by": "Memory" }));
    assert_eq!(app.ui(|ui| ui.get_sort_column()), 1);
    let e = app.act("sort_processes", json!({ "by": "disk" })).unwrap_err();
    assert!(e.contains("sort by cpu or memory, not `disk`"), "{e}");
    assert_eq!(app.ui(|ui| ui.get_sort_column()), 1, "a refusal changes nothing");
    assert!(app.act("sort_processes", json!({})).is_err(), "`by` is required");
    app.advance(POLL_EVERY);
    app.run_jobs();
    assert_eq!(machine.asked().last(), Some(&("mem", 100)));

    let reply = app.act("filter_processes", json!({ "query": "chrom" })).unwrap().settle().unwrap();
    assert_eq!(reply, json!({ "showing": 2 }));
    assert_eq!(app.ui(|ui| ui.get_process_search()), "chrom", "the box shows the filter");
    let reply = app.act("filter_processes", json!({ "query": "" })).unwrap().settle().unwrap();
    assert_eq!(reply, json!({ "showing": 3 }), "an empty query clears the filter");
    // Views, not settings: nothing of this app is written anywhere (#48).
    assert!(!app.paths().config_file().exists());
}

#[test]
fn kill_process_answers_once_the_table_says_the_process_is_gone() {
    let machine = FakeMachine::new().reads(normal()).running([4242, 4300]);
    let (app, machine) = start(machine);
    app.run_jobs();
    let pending = app.act("kill_process", json!({ "pid": 4242 })).unwrap();
    assert!(pending.is_owed(), "the kill runs in a job and is answered when it ends");
    app.run_jobs();
    let reply = pending.settle().unwrap();
    assert_eq!(reply["pid"], json!(4242));
    assert_eq!(reply["name"], json!("chromium"));
    assert_eq!(reply["signal"], json!("SIGTERM"));
    assert_eq!(reply["via"], json!("service"));
    assert_eq!(reply["exited"], json!(true));
    assert!(reply["confirmed_gone_after_ms"].is_number());
    // Forced: SIGKILL, which the service has no call for, so by this process.
    let pending = app.act("kill_process", json!({ "pid": 4300, "force": true })).unwrap();
    app.run_jobs();
    let reply = pending.settle().unwrap();
    assert_eq!(reply["signal"], json!("SIGKILL"));
    assert_eq!(reply["via"], json!("local"));
    assert_eq!(machine.signalled(), vec![(4242, Signal::Term), (4300, Signal::Kill)]);
    assert_eq!(app.ui(|ui| ui.get_notice()), "");
}

#[test]
fn a_kill_that_fails_says_why_to_the_mind_and_on_the_strip() {
    let machine = FakeMachine::new()
        .reads(readings_of(vec![process(4242, "chromium", 1.0, 1.0)], Provenance::local("no socket")))
        .undeliverable(4242, "not permitted to end 4242; it belongs to another user");
    let (app, _) = start(machine);
    app.run_jobs();
    // Init is never a process to end, and the refusal is the strip's too.
    let pending = app.act("kill_process", json!({ "pid": 1 })).unwrap();
    let e = pending.settle().unwrap_err();
    assert_eq!(e, "1 is not a process this app should end");
    // A degraded reading does not wipe it: both are on the strip.
    let notice = app.ui(|ui| ui.get_notice()).to_string();
    assert!(notice.starts_with("1 is not a process this app should end — Readings are local"), "{notice}");
    assert_eq!(app.describe()["state"]["notice"], json!(notice));
    // A pid that is not running.
    let pending = app.act("kill_process", json!({ "pid": 31337 })).unwrap();
    app.run_jobs();
    assert_eq!(pending.settle().unwrap_err(), "no process 31337 is running");
    // One that cannot be signalled.
    let pending = app.act("kill_process", json!({ "pid": 4242 })).unwrap();
    app.run_jobs();
    assert!(pending.settle().unwrap_err().contains("not permitted"));
    // Arguments the declaration does not take never reach the app.
    assert!(app.act("kill_process", json!({ "pid": "chromium" })).is_err());
    assert!(app.act("kill_process", json!({})).is_err());
}

#[test]
fn a_process_that_declines_to_exit_is_not_reported_ended() {
    let (app, _) = start(FakeMachine::new().reads(normal()).stubborn(4242).running([77]));
    app.run_jobs();
    // The End button: the answer is the strip's.
    app.ui(|ui| ui.invoke_kill_process(4242));
    app.run_jobs();
    let notice = app.ui(|ui| ui.get_notice()).to_string();
    assert!(notice.contains("was sent SIGTERM and was still running"), "{notice}");
    assert!(notice.contains("force the kill"), "{notice}");
    // Force Kill, the other button, through the same path; a kill that worked clears the strip.
    app.ui(|ui| ui.invoke_force_kill_process(77));
    app.run_jobs();
    assert_eq!(app.ui(|ui| ui.get_notice()), "");
    assert_eq!(app.model(|m| m.ended.as_ref().map(|o| (o.pid, o.signal))), Some((77, Signal::Kill)));
}

#[test]
fn an_agent_in_ask_mode_cannot_end_a_process_but_may_change_the_view() {
    let (app, machine) = start(FakeMachine::new().reads(normal()).running([4242]));
    app.run_jobs();
    let refused = app.act_as(Call::new("kill_process", json!({ "pid": 4242 })).as_agent("mind").in_mode("ask"));
    // Refused by the gate, for its grade: not for any other reason a call can fail.
    let why = refused.err().expect("a dangerous intent needs the person's word in ask mode");
    assert!(why.starts_with("GRANT:") && why.contains("kill_process"), "{why}");
    app.run_jobs();
    assert!(machine.signalled().is_empty(), "nothing was signalled");
    assert!(app
        .act_as(Call::new("sort_processes", json!({ "by": "cpu" })).as_agent("mind").in_mode("ask"))
        .is_ok());
}

#[test]
fn a_shutdown_while_a_reading_and_a_kill_are_in_flight_ends_in_time() {
    let (app, _) = start(FakeMachine::new().reads(normal()).running([4242]));
    // The first reading has not been run; a kill is owed its answer.
    let owed = app.act("kill_process", json!({ "pid": 4242 })).unwrap();
    assert!(owed.is_owed());
    assert_eq!(app.jobs_waiting(), 2);
    let began = std::time::Instant::now();
    let _clean = app.shutdown();
    assert!(began.elapsed() < Duration::from_secs(2), "shutdown must not hang on the jobs");
    assert_eq!(owed.settle().unwrap_err(), "the app is closing");
}

// ── the rail and AI Explain ─────────────────────────────────────────

#[test]
fn the_rail_and_ai_explain_ask_the_same_question_from_the_readings() {
    let mind = FakeMind::new().answers([Ok("Nothing needs attention.".into()), Ok("Still fine.".into())]);
    let (app, _, mind) = start_with(FakeMachine::new().reads(normal()), mind);
    // Before the first reading, and before the Mind is known to answer.
    app.advance(Duration::from_millis(300));
    assert_eq!(app.model(|m| m.rail.context()[0].label.clone()), "CPU —");
    assert!(app.model(|m| m.rail.suggestions().is_empty()));
    app.ui(|ui| ui.invoke_ai_explain_pressed());
    assert_eq!(app.ui(|ui| ui.get_ai_response()), Reach::NoShell.hint().unwrap());
    assert!(mind.prompts().is_empty(), "nothing is asked of a Mind that cannot answer");

    app.run_jobs();
    mind.set_reach(Reach::Ready);
    app.seen(Seen::Reach(Reach::Ready));
    app.advance(Duration::from_millis(300));
    let context = app.model(|m| m.rail.context().to_vec());
    assert_eq!(context[0].label, "CPU 12%");
    assert_eq!(context[1].label, "Memory 8.0 GB of 16.0 GB");
    assert_eq!(context[1].detail, "50% in use");
    assert_eq!(app.model(|m| m.rail.suggestions()[0].id.clone()), "explain");

    app.ui(|ui| ui.invoke_agent_suggestion_activated("explain".into()));
    app.run_jobs();
    let card = app.model(|m| m.rail.proposal().cloned().unwrap());
    assert_eq!(card.title, "The machine right now");
    assert_eq!(card.body, "Nothing needs attention.");
    assert_eq!(card.verb, "Close");
    assert!(card.impact.is_empty() && card.applies.is_none(), "an answer to read, nothing to apply");
    assert!(mind.prompts()[0].starts_with("Here are my machine's readings: "));

    // The header's button: the same question, drawn in its own panel.
    app.ui(|ui| {
        ui.invoke_ai_explain_pressed();
        ui.set_ai_panel_open(true);
    });
    assert!(app.ui(|ui| ui.get_ai_is_working()));
    app.run_jobs();
    assert_eq!(mind.prompts()[1], mind.prompts()[0]);
    assert_eq!(app.ui(|ui| ui.get_ai_response()), "Still fine.");
    assert!(!app.ui(|ui| ui.get_ai_is_working()));
    // Dismissed: the answer goes with the panel, so a later open shows no stale reading.
    app.ui(|ui| ui.invoke_ai_dismiss());
    assert_eq!(app.ui(|ui| ui.get_ai_response()), "");
    app.ui(|ui| ui.invoke_proposal_dismissed());
    assert!(app.model(|m| m.rail.proposal().is_none()));
}
