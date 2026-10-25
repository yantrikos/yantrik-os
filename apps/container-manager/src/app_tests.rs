//! Containers, headless, through `yantrik-testing`: a fake container runtime ([`FakeExec`]) and a
//! fake Mind, the real dispatch for every intent, and jobs that run only when a test says so.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::mind::Reach;
use yantrik_app::watch::Seen;
use yantrik_app::{Paths, Services};
use yantrik_testing::{Call, TestApp};

use crate::machine::fake::{self, ran, FakeExec};
use crate::machine::Listing;
use crate::model::{Command, Job, Landed};
use crate::runtime::Availability;
use crate::Containers;

const WEB: &str = "9f2c1b3d4e5f";
const DB: &str = "1a2b3c4d5e6f";

/// `web` running and `db` stopped, as `docker ps -a --format …` prints them.
fn two(web_state: &str) -> String {
    format!(
        "{WEB}\tweb\tnginx:latest\t{web_state}\tUp 2 hours\t0.0.0.0:8080->80/tcp\t2026-09-18 11:02:14 +0000 UTC\n\
         {DB}\tdb\tpostgres:16\texited\tExited (0) 3 days ago\t\t2026-09-15 09:41:00 +0000 UTC\n"
    )
}

fn only_db() -> String {
    format!("{DB}\tdb\tpostgres:16\texited\tExited (0) 3 days ago\t\t2026-09-15 09:41:00 +0000 UTC\n")
}

const IMAGES: &str = "sha256:aa\tdocker.io/library/nginx:latest\t187MB\t2026-09-01 10:00:00 +0000 UTC\n";
const VOLUMES: &str = "pgdata\tlocal\t/var/lib/docker/volumes/pgdata/_data\n";

/// A working docker with `web` (running) and `db`, one image and one volume.
pub(crate) fn machine() -> Arc<FakeExec> {
    let exec = FakeExec::new();
    exec.ps(&two("running"));
    exec.ok(&format!("images --format {}", crate::runtime::IMAGES_FORMAT), IMAGES);
    exec.ok(&format!("volume ls --format {}", crate::runtime::VOLUMES_FORMAT), VOLUMES);
    exec
}

fn start_with(exec: Arc<FakeExec>) -> (TestApp<Containers>, Arc<FakeMind>) {
    let paths = Paths::temporary(<Containers as yantrik_app::App>::ID);
    fake::install(&paths, exec);
    let mind = Arc::new(FakeMind::new());
    let app = TestApp::<Containers>::start_at_with(paths, Vec::new(), mind.clone(), Services::fakes());
    (app, mind)
}

/// Started, and the first listing landed.
pub(crate) fn started(exec: Arc<FakeExec>) -> (TestApp<Containers>, Arc<FakeMind>) {
    let (app, mind) = start_with(exec);
    app.run_jobs();
    (app, mind)
}

fn summary(app: &TestApp<Containers>) -> String {
    app.describe()["summary"].as_str().unwrap_or_default().to_string()
}

fn state(app: &TestApp<Containers>) -> Value {
    app.describe()["state"].clone()
}

fn asked(exec: &FakeExec, what: &str) -> usize {
    exec.asked().iter().filter(|a| a.as_str() == what).count()
}

// ── describe, in each case ─────────────────────────────────────────

#[test]
fn before_the_first_listing_lands_nothing_is_claimed_about_the_machine() {
    let (app, _) = start_with(machine());
    // The first read is a job; until it lands the app says what it said before it had asked.
    assert_eq!(app.jobs_waiting(), 1);
    assert_eq!(summary(&app), "Containers — the docker daemon is not reachable: not asked yet");
    let s = state(&app);
    assert_eq!(s["runtime"], json!("docker"));
    assert_eq!(s["runtime_available"], json!(false));
    assert_eq!(s["runtime_state"], json!("unreachable"));
    assert_eq!(s["containers"], json!([]));
}

#[test]
fn a_machine_with_no_runtime_is_described_as_one_and_not_as_an_empty_machine() {
    // The fake answers nothing it was not told: no docker at all.
    let (app, _) = started(FakeExec::new());
    assert_eq!(summary(&app), "Containers — docker is not installed on this machine");
    let s = state(&app);
    assert_eq!(s["runtime_available"], json!(false));
    assert_eq!(s["runtime_state"], json!("not_installed"));
    assert_eq!(s["notice"], json!("docker is not installed on this machine"));
    assert_eq!(app.ui(|ui| ui.get_notice()), "docker is not installed on this machine");
}

#[test]
fn a_daemon_that_will_not_answer_says_why_on_screen_and_to_the_mind() {
    let exec = FakeExec::new();
    exec.answer(
        &format!("ps -a --format {}", crate::runtime::PS_FORMAT),
        ran(1, "", "Cannot connect to the Docker daemon at unix:///var/run/docker.sock.\n"),
    );
    let (app, _) = started(exec.clone());
    let why = "Cannot connect to the Docker daemon at unix:///var/run/docker.sock.";
    assert_eq!(summary(&app), format!("Containers — the docker daemon is not reachable: {why}"));
    assert_eq!(state(&app)["notice"], json!(format!("the docker daemon is not reachable: {why}")));
    assert_eq!(state(&app)["runtime_state"], json!("unreachable"));
    // Nothing more is asked of a runtime that cannot answer.
    assert_eq!(exec.asked().len(), 1);
}

#[test]
fn an_empty_machine_with_a_working_runtime_is_said_to_be_empty() {
    let exec = FakeExec::new();
    exec.ready("");
    let (app, _) = started(exec);
    assert_eq!(summary(&app), "Containers — docker is running and has no containers");
    assert_eq!(state(&app)["notice"], json!(""));
    assert_eq!(state(&app)["runtime_available"], json!(true));
    assert_eq!(app.ui(|ui| ui.get_total_count()), 0);
}

#[test]
fn a_working_machine_is_described_as_the_screen_shows_it() {
    let (app, _) = started(machine());
    assert_eq!(summary(&app), "Containers — 1 running of 2 on docker, 1 images");
    let s = state(&app);
    assert_eq!(s["runtime"], json!("docker"));
    assert_eq!(s["runtime_available"], json!(true));
    assert_eq!(s["runtime_state"], json!("ready"));
    assert_eq!(s["notice"], json!(""));
    assert_eq!(s["tab"], json!("containers"));
    assert_eq!((s["running"].clone(), s["stopped"].clone(), s["total"].clone()), (json!(1), json!(1), json!(2)));
    assert_eq!(
        s["containers"][0],
        json!({
            "name": "web", "id": WEB, "image": "nginx:latest", "status": "running",
            "detail": "Up 2 hours", "ports": "0.0.0.0:8080->80/tcp",
        })
    );
    assert_eq!(s["containers"][1]["status"], json!("exited"));
    assert_eq!(s["images"], json!([{ "tag": "docker.io/library/nginx:latest", "size": "187MB" }]));
    assert_eq!(s["volumes"], json!([{ "name": "pgdata", "driver": "local" }]));
    assert!(s.get("open_logs").is_none(), "no log pane is open");
    // The window shows the same.
    assert_eq!(app.ui(|ui| (ui.get_running_count(), ui.get_stopped_count(), ui.get_total_count())), (1, 1, 2));
    assert_eq!(app.ui(|ui| ui.get_runtime_name()), "docker");
    use slint::Model as _;
    assert_eq!(app.ui(|ui| ui.get_images().row_data(0).unwrap().title.to_string()), "nginx, the latest version");
    assert_eq!(app.ui(|ui| ui.get_containers().row_count()), 2);
}

#[test]
fn podman_is_asked_when_it_is_there() {
    let exec = FakeExec::podman();
    exec.ready(&two("running"));
    let (app, _) = started(exec.clone());
    assert_eq!(summary(&app), "Containers — 1 running of 2 on podman, 0 images");
    assert!(exec.asked()[0].starts_with("podman ps -a"));
    assert_eq!(app.ui(|ui| ui.get_runtime_name()), "podman");
}

// ── The intents ────────────────────────────────────────────────────

#[test]
fn refresh_and_show_tab_answer_as_they_always_did() {
    let (app, _) = started(machine());
    let pending = app.act("refresh", json!({})).unwrap();
    assert!(pending.is_owed(), "answered once the listing has been re-read");
    app.run_jobs();
    assert_eq!(
        pending.settle().unwrap(),
        json!({ "running": 1, "total": 2, "runtime_available": true, "runtime_state": "ready" })
    );

    let reply = app.act("show_tab", json!({ "tab": " Images " })).unwrap().settle().unwrap();
    assert_eq!(reply, json!({ "tab": "images" }));
    assert_eq!(app.ui(|ui| ui.get_active_tab()), 1);
    assert_eq!(state(&app)["tab"], json!("images"));
    let e = app.act("show_tab", json!({ "tab": "networks" })).unwrap_err();
    assert!(e.contains("no `networks` tab; use containers, images or volumes"), "{e}");
    assert_eq!(state(&app)["tab"], json!("images"), "a refusal changes nothing");
    // The tab bar switches the list itself and tells the model.
    app.ui(|ui| ui.invoke_ct_tab(2));
    assert_eq!(state(&app)["tab"], json!("volumes"));
}

#[test]
fn a_container_that_is_not_here_is_refused_and_nothing_is_run() {
    let (app, _) = started(machine());
    let before = app.jobs_waiting();
    for action in ["start", "stop", "restart", "show_logs", "remove"] {
        let e = app.act(action, json!({ "container": "no-such-container" })).unwrap_err();
        assert!(e.contains("no container here is called \"no-such-container\""), "{action}: {e}");
    }
    assert_eq!(app.jobs_waiting(), before);
}

#[test]
fn start_and_restart_answer_with_the_state_the_runtime_shows_after() {
    let exec = machine();
    exec.ps(&two("exited"));
    let (app, _) = started(exec.clone());
    exec.ok(&format!("start {WEB}"), "web\n");
    let pending = app.act("start", json!({ "container": "web" })).unwrap();
    // `docker start` succeeded, and the container's process ended a moment later.
    exec.ps(&two("exited"));
    app.run_jobs();
    assert_eq!(pending.settle().unwrap(), json!({ "started": "web", "status": "exited" }));
    assert_eq!(asked(&exec, &format!("docker start {WEB}")), 1);

    exec.ok(&format!("restart {WEB}"), "");
    let pending = app.act("restart", json!({ "container": "WEB" })).unwrap();
    exec.ps(&two("running"));
    app.run_jobs();
    assert_eq!(pending.settle().unwrap(), json!({ "restarted": "web", "status": "running" }));
    // An id prefix names a container the way docker does.
    exec.ok(&format!("start {DB}"), "");
    let pending = app.act("start", json!({ "container": "1a2b" })).unwrap();
    app.run_jobs();
    assert_eq!(pending.settle().unwrap()["started"], json!("db"));
}

#[test]
fn a_stop_or_remove_the_runtime_did_not_do_is_a_failure_on_screen_and_to_the_mind() {
    let exec = machine();
    let (app, _) = started(exec.clone());
    exec.ok(&format!("stop {WEB}"), "");
    let pending = app.act("stop", json!({ "container": "web" })).unwrap();
    app.run_jobs();
    assert_eq!(pending.settle().unwrap_err(), "\u{201C}web\u{201D} is still running after stop");
    assert_eq!(app.ui(|ui| ui.get_notice()), "\u{201C}web\u{201D} is still running after stop");

    exec.ok(&format!("rm -f {WEB}"), "");
    let pending = app.act("remove", json!({ "container": "web" })).unwrap();
    app.run_jobs();
    assert_eq!(pending.settle().unwrap_err(), "\u{201C}web\u{201D} is still listed after rm -f");
    assert_eq!(state(&app)["notice"], json!("\u{201C}web\u{201D} is still listed after rm -f"));

    // And when it did: the answer says so, and the notice is cleared.
    let pending = app.act("stop", json!({ "container": "web" })).unwrap();
    exec.ps(&two("exited"));
    app.run_jobs();
    assert_eq!(pending.settle().unwrap(), json!({ "stopped": "web", "status": "exited" }));
    assert_eq!(state(&app)["notice"], json!(""));
    let pending = app.act("remove", json!({ "container": "web" })).unwrap();
    exec.ps(&only_db());
    app.run_jobs();
    assert_eq!(pending.settle().unwrap(), json!({ "removed": "web" }));
    assert_eq!(summary(&app), "Containers — 0 running of 1 on docker, 1 images");
}

#[test]
fn a_refused_command_carries_the_runtimes_own_words() {
    let exec = machine();
    let (app, _) = started(exec.clone());
    exec.answer(&format!("rm -f {WEB}"), ran(1, "", "Error response from daemon: No such container: web\n"));
    let pending = app.act("remove", json!({ "container": "web" })).unwrap();
    app.run_jobs();
    assert_eq!(pending.settle().unwrap_err(), "Error response from daemon: No such container: web");
    assert_eq!(state(&app)["notice"], json!("Error response from daemon: No such container: web"));
}

#[test]
fn show_logs_opens_the_pane_answers_the_tail_and_describe_carries_it_while_open() {
    let exec = machine();
    let (app, _) = started(exec.clone());
    // Both streams: a container's stderr is part of its log.
    exec.answer(&format!("logs --tail 200 {WEB}"), ran(0, "listening on :80\n", "warn: slow upstream\n"));
    let pending = app.act("show_logs", json!({ "container": "web" })).unwrap();
    app.run_jobs();
    assert_eq!(
        pending.settle().unwrap(),
        json!({ "container": "web", "tail": "listening on :80\nwarn: slow upstream" })
    );
    assert!(app.ui(|ui| ui.get_show_logs()));
    assert_eq!(app.ui(|ui| ui.get_log_container_name()), WEB);
    assert_eq!(
        state(&app)["open_logs"],
        json!({ "container": WEB, "tail": "listening on :80\nwarn: slow upstream\n" })
    );
    // The screen closes its own pane and says so; describe stops carrying the tail.
    app.ui(|ui| {
        ui.set_show_logs(false);
        ui.invoke_logs_closed();
    });
    assert!(state(&app).get("open_logs").is_none());
    // A log the runtime would not give is not shown, and the reason is.
    exec.answer(&format!("logs --tail 200 {DB}"), ran(1, "", "Error: no logs for db\n"));
    let pending = app.act("show_logs", json!({ "container": "db" })).unwrap();
    app.run_jobs();
    assert_eq!(pending.settle().unwrap_err(), "Error: no logs for db");
    assert!(!app.ui(|ui| ui.get_show_logs()));
}

#[test]
fn what_cannot_be_taken_back_needs_a_person_and_reading_does_not() {
    let exec = machine();
    let (app, _) = started(exec.clone());
    let agent = |name: &str| Call::new(name, json!({ "container": "web" })).as_agent("mind").in_mode("ask");
    for name in ["stop", "restart", "remove"] {
        assert!(app.act_as(agent(name)).is_err(), "{name} is above standard: an agent asks first");
    }
    assert!(exec.asked().iter().all(|a| !a.contains(" stop ") && !a.contains("rm -f")), "nothing ran");
    assert_eq!(app.jobs_waiting(), 0);
    // Starting is recoverable, and reading changes nothing on the machine.
    exec.ok(&format!("start {WEB}"), "");
    assert!(app.act_as(agent("start")).is_ok());
    assert!(app.act_as(Call::new("refresh", json!({})).as_agent("mind").in_mode("ask")).is_ok());
}

// ── The buttons ────────────────────────────────────────────────────

#[test]
fn every_button_runs_its_command_off_the_window_and_says_what_failed() {
    let exec = machine();
    let (app, _) = started(exec.clone());
    exec.ok(&format!("stop {WEB}"), "");
    app.ui(|ui| ui.invoke_ct_stop(WEB.into()));
    assert_eq!(asked(&exec, &format!("docker stop {WEB}")), 0, "nothing ran on the window's thread");
    app.run_jobs();
    assert_eq!(asked(&exec, &format!("docker stop {WEB}")), 1);
    // A button does not check what the action checks; its failures are the runtime's own.
    assert_eq!(state(&app)["notice"], json!(""));

    app.ui(|ui| {
        ui.invoke_ct_run("   ".into());
        ui.invoke_ct_run(" nginx ".into());
        ui.invoke_ct_pull("alpine".into());
        ui.invoke_ct_remove_image("sha256:aa".into());
        ui.invoke_ct_create_volume("cache".into());
        ui.invoke_ct_remove_volume("pgdata".into());
        ui.invoke_ct_start(DB.into());
        ui.invoke_ct_restart(DB.into());
    });
    app.run_jobs();
    for argv in [
        "docker run -d -- nginx".to_string(),
        "docker pull -- alpine".to_string(),
        "docker rmi sha256:aa".to_string(),
        "docker volume create cache".to_string(),
        "docker volume rm pgdata".to_string(),
        format!("docker start {DB}"),
        format!("docker restart {DB}"),
    ] {
        assert_eq!(asked(&exec, &argv), 1, "{argv}");
    }
    assert!(exec.asked().iter().all(|a| a != "docker run -d -- "), "spaces name no image");
    // Every one of those was answered "not installed" by the fake: the last one's words show.
    assert!(state(&app)["notice"].as_str().unwrap().contains("docker is not installed"));
    app.ui(|ui| ui.invoke_ct_refresh());
    app.run_jobs();
    assert_eq!(state(&app)["notice"], json!(""), "a good listing clears the machine's notice");
}

#[test]
fn a_second_press_before_the_first_lands_is_the_same_request() {
    let exec = machine();
    let (app, _) = started(exec.clone());
    exec.ok(&format!("rm -f {WEB}"), "");
    app.ui(|ui| {
        ui.invoke_ct_remove(WEB.into());
        ui.invoke_ct_remove(WEB.into());
    });
    exec.ps(&only_db());
    app.run_jobs();
    assert_eq!(asked(&exec, &format!("docker rm -f {WEB}")), 1);
    assert_eq!(state(&app)["notice"], json!(""));
    // Once it has landed, the row is gone; a later press would be a new request.
    assert_eq!(app.model(|m| m.in_flight.len()), 0);
}

// ── Stale results, a panicking worker, a shutdown ──────────────────

#[test]
fn a_listing_or_a_log_read_in_an_earlier_turn_is_not_drawn_over_a_later_one() {
    let exec = machine();
    let (app, _) = started(exec.clone());
    let shown = app.model(|m| m.listed_turn);
    let old = Listing {
        turn: shown - 1,
        runtime: "docker",
        availability: Availability::Missing,
        containers: Vec::new(),
        images: Vec::new(),
        volumes: Vec::new(),
    };
    app.run(Command::Landed(Box::new(Landed { job: Job::Refresh, exit: None, listing: old.clone() })));
    assert_eq!(summary(&app), "Containers — 1 running of 2 on docker, 1 images", "dropped");

    exec.answer(&format!("logs --tail 200 {DB}"), ran(0, "db log\n", ""));
    app.ui(|ui| ui.invoke_ct_logs(DB.into()));
    app.run_jobs();
    assert_eq!(app.model(|m| m.logs.text.clone()), "db log\n");
    let late = Landed {
        job: Job::Logs { id: WEB.into(), named: None },
        exit: Some(ran(0, "web log\n", "")),
        listing: Listing { turn: 1, ..old },
    };
    app.run(Command::Landed(Box::new(late)));
    assert_eq!(app.model(|m| m.logs.text.clone()), "db log\n", "the older read is not drawn");
    assert_eq!(app.ui(|ui| ui.get_log_container_name()), DB);
}

#[test]
fn a_worker_that_panics_is_answered_and_the_app_keeps_running() {
    let exec = machine();
    let (app, _) = started(exec.clone());
    exec.panic_on(&format!("restart {WEB}"));
    let pending = app.act("restart", json!({ "container": "web" })).unwrap();
    app.run_jobs();
    let e = pending.settle().unwrap_err();
    assert!(e.starts_with("the work failed"), "{e}");
    assert_eq!(state(&app)["notice"], json!(crate::update::LOST));
    assert_eq!(app.model(|m| m.in_flight.len()), 0, "the lost command is not left in flight");
    // Still running: the next refresh is read and drawn.
    app.ui(|ui| ui.invoke_ct_refresh());
    app.run_jobs();
    assert_eq!(state(&app)["notice"], json!(""));
}

#[test]
fn a_shutdown_while_a_command_is_in_flight_refuses_its_answer_and_does_not_hang() {
    let (app, _) = started(machine());
    let owed = app.act("start", json!({ "container": "db" })).unwrap();
    assert!(owed.is_owed());
    app.ui(|ui| ui.invoke_ct_refresh());
    let began = std::time::Instant::now();
    let _clean = app.shutdown();
    assert!(began.elapsed() < Duration::from_secs(2), "shutdown must not hang on the jobs");
    assert_eq!(owed.settle().unwrap_err(), "the app is closing");
}

// ── The rail and the AI button ─────────────────────────────────────

#[test]
fn the_rail_offers_its_one_question_only_with_a_log_to_read() {
    let exec = machine();
    let (app, mind) = started(exec.clone());
    app.advance(Duration::from_millis(300));
    let context = app.model(|m| m.rail.context().to_vec());
    assert_eq!(context.len(), 1);
    assert_eq!((context[0].id.as_str(), context[0].label.as_str()), ("counts", "1 running, 1 stopped"));
    assert_eq!((context[0].detail.as_str(), context[0].source.as_str()), ("docker", "file"));
    // No Mind: and no log, so nothing is said about it either.
    assert_eq!(app.ui(|ui| ui.get_agent_unavailable()), "");

    mind.set_reach(Reach::Ready);
    app.seen(Seen::Reach(Reach::Ready));
    assert_eq!(app.model(|m| m.rail.suggestions().len()), 0, "no log yet");
    exec.answer(&format!("logs --tail 200 {WEB}"), ran(0, "boot ok\n", ""));
    app.ui(|ui| ui.invoke_ct_logs(WEB.into()));
    app.run_jobs();
    app.advance(Duration::from_millis(300));
    let offered = app.model(|m| m.rail.suggestions().to_vec());
    assert_eq!(offered.len(), 1);
    assert_eq!((offered[0].id.as_str(), offered[0].label.as_str()), ("logs", "Explain these logs"));

    app.ui(|ui| ui.invoke_agent_suggestion_activated("logs".into()));
    app.run_jobs();
    assert!(mind.prompts()[0].starts_with(&format!("These are the last lines of the log for container {WEB}.")));
    assert!(mind.prompts()[0].ends_with("\n\nboot ok"));
    let card = app.model(|m| m.rail.proposal().cloned().unwrap());
    assert_eq!(card.verb, "Close");
    assert!(card.applies.is_none() && card.impact.is_empty(), "an answer to read, nothing to apply");
    // The fake had no answer left: the card says the companion did not answer.
    assert_eq!(card.title, "The companion did not answer");

    // Out of reach, with a log on screen: now the reason is said.
    mind.set_reach(Reach::NoShell);
    app.seen(Seen::Reach(Reach::NoShell));
    assert_eq!(app.ui(|ui| ui.get_agent_unavailable()), Reach::NoShell.hint().unwrap());
}

#[test]
fn the_ai_button_asks_the_rails_question_and_says_when_there_is_no_log() {
    let exec = machine();
    let paths = Paths::temporary(<Containers as yantrik_app::App>::ID);
    fake::install(&paths, exec.clone());
    let mind = Arc::new(FakeMind::new().answering("Nginx started and is serving."));
    let app = TestApp::<Containers>::start_at_with(paths, Vec::new(), mind.clone(), Services::fakes());
    app.run_jobs();

    app.ui(|ui| ui.invoke_ai_summarize_logs());
    assert_eq!(app.ui(|ui| ui.get_ai_response()), "There is no log open to read.");
    assert!(mind.prompts().is_empty());

    exec.answer(&format!("logs --tail 200 {WEB}"), ran(0, "boot ok\n", ""));
    app.ui(|ui| ui.invoke_ct_logs(WEB.into()));
    app.run_jobs();
    app.ui(|ui| ui.invoke_ai_summarize_logs());
    assert!(app.ui(|ui| ui.get_ai_is_working()));
    app.run_jobs();
    assert!(!app.ui(|ui| ui.get_ai_is_working()));
    assert_eq!(app.ui(|ui| ui.get_ai_response()), "Nginx started and is serving.");
    assert!(mind.prompts()[0].starts_with(&format!("These are the last lines of the log for container {WEB}.")));
    app.ui(|ui| ui.invoke_ai_dismiss());
    assert_eq!(app.ui(|ui| ui.get_ai_response()), "");
    // A failed ask is said in the companion's words.
    app.ui(|ui| ui.invoke_ai_summarize_logs());
    app.run_jobs();
    assert!(app.ui(|ui| ui.get_ai_response()).starts_with("The companion did not answer"));
}

#[test]
fn a_listing_that_lands_with_no_command_answers_nothing_owed() {
    // A refresh started by the button settles nothing: there is no caller to answer.
    let (app, _) = started(machine());
    app.ui(|ui| ui.invoke_ct_refresh());
    app.run_jobs();
    assert_eq!(app.settlements_open(), 0);
}
