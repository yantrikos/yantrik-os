//! Yantrik Container Manager — standalone app binary.
//!
//! Manages Docker/Podman containers by running the runtime and reading what it says back.
//! Everything about *what the runtime said* lives in [`runtime`], which has no window in it and
//! is tested without docker on the machine; this file is the window, the control surface, and
//! the single path that both of them take to change anything.

mod runtime;

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use yantrik_app_runtime::prelude::*;

slint::include_modules!();

/// What the app last learned about the runtime itself.
///
/// Kept beside the window because `describe` has to answer a question the container list cannot:
/// an empty list used to mean "docker is not installed", "the daemon is down" and "there is
/// nothing running here" all at once, and a mind reading it concluded the machine was empty.
type Health = Rc<RefCell<runtime::Availability>>;

/// Fill the agent rail from what the runtime reports.
///
/// Counts are facts the app already has. The suggestion only appears when there are logs on
/// screen to read -- offering to explain logs that are not there is the kind of empty promise
/// this rail exists not to make.
fn refresh_agent_rail(ui: &ContainerManagerApp) {
    let context = vec![AgentContextItem {
        id: "counts".into(),
        label: format!(
            "{} running, {} stopped",
            ui.get_running_count(),
            ui.get_stopped_count()
        )
        .into(),
        detail: ui.get_runtime_name(),
        source: "file".into(),
    }];
    ui.set_agent_context(ModelRc::new(VecModel::from(context)));

    let has_logs = !ui.get_log_text().to_string().trim().is_empty();
    let reach = companion::reach();
    let mut next: Vec<AgentSuggestion> = Vec::new();
    if reach == companion::Reach::Ready && has_logs {
        next.push(AgentSuggestion {
            id: "logs".into(),
            label: "Explain these logs".into(),
            detail: "reads the last of the output".into(),
            icon: "spark".into(),
            running: ui.get_proposal_working(),
            proposes: false,
        });
    }
    ui.set_agent_suggestions(ModelRc::new(VecModel::from(next)));
    ui.set_agent_unavailable(match reach.hint() {
        Some(hint) if has_logs => hint.into(),
        _ => SharedString::new(),
    });
}

/// What to ask the companion about the log that is on screen, or `None` when there is none.
///
/// One place, because two controls ask it: the rail's suggestion and the header's AI button.
fn log_question(ui: &ContainerManagerApp) -> Option<String> {
    let all = ui.get_log_text().to_string();
    let tail = all
        .lines()
        .rev()
        .take(40)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    if tail.trim().is_empty() {
        return None;
    }
    let name = ui.get_log_container_name().to_string();
    Some(format!(
        "These are the last lines of the log for container {name}. In at most four short lines \
         say what is happening and whether anything is wrong. Use only what is shown.\n\n{tail}"
    ))
}

fn main() {
    init_tracing("yantrik-container-manager");

    // One window per app: a second launch defers to the running one (the shell focuses it).
    let Some(_instance) = instance::claim("container-manager") else { return };

    let app = ContainerManagerApp::new().unwrap();
    // The window's title bar is the app's own (#256).
    yantrik_app_runtime::window_chrome!(app);

    // Same dark/accent choice as the shell, read from the shell's settings file.
    let theme = theme::load();
    app.global::<ThemeMode>().set_dark(theme.dark);
    app.global::<AccentPreset>().set_index(theme.accent_index);

    // Nothing has been asked yet, so nothing is known; the first refresh below decides.
    let health: Health = Rc::new(RefCell::new(runtime::Availability::Unreachable(
        "not asked yet".into(),
    )));

    wire(&app, &health);
    // ── The agent layer ──
    {
        let weak = app.as_weak();
        app.on_agent_suggestion_activated(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            if id != "logs" {
                return;
            }
            let Some(prompt) = log_question(&ui) else { return };
            ui.set_proposal_working(true);
            ui.set_proposal(AgentProposal {
                title: "What these logs say".into(),
                source: "from the container log".into(),
                ..Default::default()
            });
            let back = ui.as_weak();
            std::thread::spawn(move || {
                let outcome = companion::ask(&prompt);
                let _ = back.upgrade_in_event_loop(move |ui| {
                    ui.set_proposal_working(false);
                    match outcome {
                        Ok(text) => ui.set_proposal(AgentProposal {
                            title: "What these logs say".into(),
                            body: text.into(),
                            source: "from the container log".into(),
                            verb: "Close".into(),
                            ..Default::default()
                        }),
                        Err(e) => ui.set_proposal(AgentProposal {
                            title: "The companion did not answer".into(),
                            body: format!("{e}").into(),
                            verb: "Close".into(),
                            ..Default::default()
                        }),
                    }
                });
            });
        });
    }
    {
        let weak = app.as_weak();
        app.on_proposal_dismissed(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_proposal(AgentProposal::default());
            }
        });
    }
    app.on_proposal_applied(|| {});
    app.on_agent_context_activated(|_| {});
    // The rail follows the app's state on a timer.
    //
    // Calling it once at startup was not enough: at that moment Weather has no reading yet and
    // Image Viewer has no file, so both rails computed "nothing to say", collapsed, and stayed
    // collapsed for the life of the process. Every app loads its content on some path of its own
    // and hooking each one is how a refresh gets missed; asking every few seconds is cheap and
    // cannot be forgotten.
    let rail_timer = slint::Timer::default();
    {
        let weak = app.as_weak();
        rail_timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(4),
            move || {
                if let Some(ui) = weak.upgrade() {
                    refresh_agent_rail(&ui);
                }
            },
        );
    }
    refresh_agent_rail(&app);

    run_until_closed(&app, "yantrik-container-manager");
}

// ── Reading the runtime into the window ──────────────────────────────

fn refresh(app: &ContainerManagerApp, health: &Health) {
    let rt = runtime::detect();
    let ps = runtime::run(rt, &["ps", "-a", "--format", runtime::PS_FORMAT]);
    let availability = runtime::availability(&ps);

    let containers: Vec<ContainerData> = match &ps {
        runtime::Exit::Ran { code: Some(0), stdout, .. } => runtime::parse_containers(stdout)
            .into_iter()
            .map(|c| ContainerData {
                id: c.id.into(),
                name: c.name.into(),
                image: c.image.into(),
                status: c.state.into(),
                status_text: c.status_text.into(),
                ports: c.ports.into(),
                created: c.created.into(),
                is_selected: false,
            })
            .collect(),
        _ => Vec::new(),
    };

    let running = containers.iter().filter(|c| c.status.as_str() == "running").count() as i32;
    let total = containers.len() as i32;
    app.set_containers(ModelRc::new(VecModel::from(containers)));
    app.set_running_count(running);
    app.set_stopped_count(total - running);
    app.set_total_count(total);

    // Nothing to ask when the runtime cannot answer: two more processes that would fail the same
    // way, and two more empty lists that would look like emptiness rather than absence.
    if availability.is_ready() {
        app.set_images(ModelRc::new(VecModel::from(read_images(rt))));
        app.set_volumes(ModelRc::new(VecModel::from(read_volumes(rt))));
    } else {
        app.set_images(ModelRc::new(VecModel::<ImageData>::default()));
        app.set_volumes(ModelRc::new(VecModel::<VolumeData>::default()));
    }

    // The state of the machine, said on screen. A command's own failure is more specific than
    // this and overwrites it in `settle`, which runs immediately after.
    app.set_notice(match availability.trouble(rt) {
        Some(line) => line.into(),
        None => SharedString::new(),
    });
    *health.borrow_mut() = availability;
}

fn read_images(rt: &str) -> Vec<ImageData> {
    let exit = runtime::run(rt, &["images", "--format", runtime::IMAGES_FORMAT]);
    let runtime::Exit::Ran { code: Some(0), stdout, .. } = &exit else { return Vec::new() };
    runtime::parse_images(stdout)
        .into_iter()
        .map(|i| ImageData {
            id: i.id.into(),
            repo_tag: i.repo_tag.into(),
            size_text: i.size_text.into(),
            created: i.created.into(),
        })
        .collect()
}

fn read_volumes(rt: &str) -> Vec<VolumeData> {
    let exit = runtime::run(rt, &["volume", "ls", "--format", runtime::VOLUMES_FORMAT]);
    let runtime::Exit::Ran { code: Some(0), stdout, .. } = &exit else { return Vec::new() };
    runtime::parse_volumes(stdout)
        .into_iter()
        .map(|v| VolumeData {
            name: v.name.into(),
            driver: v.driver.into(),
            mount_point: v.mount_point.into(),
            size_text: "".into(),
        })
        .collect()
}

// ── One code path per command ────────────────────────────────────────

/// Run one runtime command, show what it did, and hand the result back to whoever asked.
///
/// Every mutation in this app goes through here, the button and the control action alike, so a
/// person and a mind cannot be told different things about the same command. What this replaces
/// was nine copies of `let _ = Command::new(runtime_cmd())...output();` — the result of every
/// `start`, `stop`, `restart`, `rm -f`, `run -d`, `pull`, `rmi`, `volume create` and `volume rm`
/// was dropped on the floor, so the surface answered `{"removed": name}` on an action graded
/// `dangerous` whether or not the container was still there.
fn command(
    ui: &ContainerManagerApp,
    health: &Health,
    argv: &[&str],
) -> Result<String, String> {
    let rt = runtime::detect();
    let outcome = runtime::outcome(rt, argv, runtime::run(rt, argv));
    settle(ui, health, outcome)
}

/// Re-read the machine, then put the command's own verdict on screen.
///
/// The re-read comes first on purpose: `refresh` writes the notice about the machine, and the
/// command's message is the more specific of the two, so it has to be written last.
fn settle(
    ui: &ContainerManagerApp,
    health: &Health,
    result: Result<String, String>,
) -> Result<String, String> {
    refresh(ui, health);
    match &result {
        Ok(_) => ui.set_notice(SharedString::new()),
        Err(reason) => ui.set_notice(reason.as_str().into()),
    }
    result
}

/// A command that exited zero but did not do what it was asked is a failure too.
///
/// Said on screen as well as returned, because a row that is still in the list after a delete
/// otherwise reads as "the delete has not drawn yet".
fn disagree(ui: &ContainerManagerApp, reason: String) -> String {
    ui.set_notice(reason.as_str().into());
    reason
}

/// Read a container's log into the pane.
fn read_logs(ui: &ContainerManagerApp, health: &Health, id: &str) -> Result<String, String> {
    let rt = runtime::detect();
    let argv = ["logs", "--tail", "200", id];
    let exit = runtime::run(rt, &argv);
    // Both streams: a container's own stderr is part of its log and `docker logs` passes it
    // through, so taking stdout alone showed an empty pane for every program that logs the
    // ordinary way — which reads as "this container said nothing".
    let text = match &exit {
        runtime::Exit::Ran { code: Some(0), stdout, stderr } => format!("{stdout}{stderr}"),
        _ => String::new(),
    };
    settle(ui, health, runtime::outcome(rt, &argv, exit))?;
    ui.set_log_text(text.as_str().into());
    ui.set_log_container_name(id.into());
    ui.set_show_logs(true);
    Ok(text)
}

// ── The control surface ──────────────────────────────────────────────
//
// The companion has docker tools of its own, and they shell out exactly as this app does. What
// this adds is the *user's* view: which containers they are looking at, which one is selected,
// what the log pane is showing. See `yantrik_app_runtime::control`.
//
// Stopping and removing are declared `dangerous`. Starting a container is recoverable; removing
// one, or the volume under it, is not.

/// Every container the window is showing, as `(id, name)`.
fn listed(ui: &ContainerManagerApp) -> Vec<(String, String)> {
    let model = ui.get_containers();
    (0..model.row_count())
        .filter_map(|i| model.row_data(i))
        .map(|c| (c.id.to_string(), c.name.to_string()))
        .collect()
}

/// Container whose name or id matches `needle`, as shown in the list.
fn container_named(ui: &ContainerManagerApp, needle: &str) -> Option<(String, String)> {
    let rows = listed(ui);
    runtime::resolve(&rows, needle).map(|i| rows[i].clone())
}

/// How the list describes this container now, or `None` when it is no longer in it.
///
/// Read after a command, from a list the command itself caused to be re-read, so an action
/// answers what the runtime shows rather than what a zero exit status implied.
fn observed_state(ui: &ContainerManagerApp, id: &str) -> Option<String> {
    let model = ui.get_containers();
    (0..model.row_count())
        .filter_map(|i| model.row_data(i))
        .find(|c| c.id.as_str() == id)
        .map(|c| c.status.to_string())
}

fn publish_control(app: &ContainerManagerApp, health: &Health) {
    use yantrik_app_runtime::control::{Action, App, Param, View};

    let describe = {
        let weak = app.as_weak();
        let health = health.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return View::new("Containers — closing");
            };

            let model = ui.get_containers();
            let containers: Vec<serde_json::Value> = (0..model.row_count())
                .filter_map(|i| model.row_data(i))
                .map(|c| {
                    serde_json::json!({
                        "name": c.name.to_string(),
                        "id": c.id.to_string(),
                        "image": c.image.to_string(),
                        "status": c.status.to_string(),
                        "detail": c.status_text.to_string(),
                        "ports": c.ports.to_string(),
                    })
                })
                .collect();

            let image_model = ui.get_images();
            let images: Vec<serde_json::Value> = (0..image_model.row_count().min(30))
                .filter_map(|i| image_model.row_data(i))
                .map(|i| {
                    serde_json::json!({
                        "tag": i.repo_tag.to_string(),
                        "size": i.size_text.to_string(),
                    })
                })
                .collect();

            let volume_model = ui.get_volumes();
            let volumes: Vec<serde_json::Value> = (0..volume_model.row_count().min(30))
                .filter_map(|i| volume_model.row_data(i))
                .map(|v| {
                    serde_json::json!({
                        "name": v.name.to_string(),
                        "driver": v.driver.to_string(),
                    })
                })
                .collect();

            let runtime_name = ui.get_runtime_name().to_string();
            // Three summaries where there used to be two, and the missing one was the important
            // one: "no docker containers" was what this app said on a machine with no docker on
            // it, which is a statement about the machine that happened not to be true.
            let state = health.borrow();
            let summary = match &*state {
                runtime::Availability::Missing => {
                    format!("Containers — {runtime_name} is not installed on this machine")
                }
                runtime::Availability::Unreachable(reason) => {
                    format!("Containers — the {runtime_name} daemon is not reachable: {reason}")
                }
                runtime::Availability::Ready if containers.is_empty() => {
                    format!("Containers — {runtime_name} is running and has no containers")
                }
                runtime::Availability::Ready => format!(
                    "Containers — {} running of {} on {runtime_name}, {} images",
                    ui.get_running_count(),
                    ui.get_total_count(),
                    images.len()
                ),
            };

            let mut view = View::new(summary)
                .with("runtime", runtime_name)
                .with("runtime_available", state.is_ready())
                .with("runtime_state", state.state_name())
                // Said twice: the strip under the header is the person's half of this.
                .with("notice", ui.get_notice().to_string())
                .with("tab", match ui.get_active_tab() {
                    1 => "images",
                    2 => "volumes",
                    _ => "containers",
                })
                .with("running", ui.get_running_count())
                .with("stopped", ui.get_stopped_count())
                .with("total", ui.get_total_count())
                .with("containers", serde_json::Value::Array(containers))
                .with("images", serde_json::Value::Array(images))
                .with("volumes", serde_json::Value::Array(volumes));

            // Only when the pane is actually open. A log tail is the largest thing this surface
            // can carry, and sending one nobody asked for would make every describe expensive.
            if ui.get_show_logs() {
                let log = ui.get_log_text().to_string();
                let tail: String = log.chars().rev().take(4000).collect::<Vec<_>>().into_iter().rev().collect();
                view = view.with(
                    "open_logs",
                    serde_json::json!({
                        "container": ui.get_log_container_name().to_string(),
                        "tail": tail,
                    }),
                );
            }
            view
        }
    };

    let weak = app.as_weak();
    let ui_for = move || weak.upgrade().ok_or_else(|| "Containers window is gone".to_string());

    let refresh_ui = ui_for.clone();
    let start_ui = ui_for.clone();
    let stop_ui = ui_for.clone();
    let restart_ui = ui_for.clone();
    let logs_ui = ui_for.clone();
    let remove_ui = ui_for;

    let refresh_health = health.clone();
    let start_health = health.clone();
    let stop_health = health.clone();
    let restart_health = health.clone();
    let logs_health = health.clone();
    let remove_health = health.clone();

    App::new("containers")
        .describe(describe)
        .action(Action::new("refresh", "Re-read containers, images and volumes"), move |_| {
            let ui = refresh_ui()?;
            ui.invoke_ct_refresh();
            let state = refresh_health.borrow();
            Ok(serde_json::json!({
                "running": ui.get_running_count(),
                "total": ui.get_total_count(),
                "runtime_available": state.is_ready(),
                "runtime_state": state.state_name(),
            }))
        })
        .action(
            Action::new("start", "Start a stopped container").arg(Param::text("container")),
            move |args| {
                let ui = start_ui()?;
                let want = args["container"].as_str().unwrap_or_default();
                let (id, name) = container_named(&ui, want)
                    .ok_or_else(|| format!("no container here is called \"{want}\""))?;
                command(&ui, &start_health, &["start", id.as_str()])?;
                // What the re-read list shows, not what the exit status implied: `docker start`
                // succeeds for a container whose process ends a moment later, and answering
                // "started" without the state would hide that it is not running.
                let status = observed_state(&ui, &id).unwrap_or_else(|| "gone".to_string());
                Ok(serde_json::json!({ "started": name, "status": status }))
            },
        )
        .action(
            // Recoverable, but it interrupts whatever the container was serving.
            Action::new("stop", "Stop a running container")
                .arg(Param::text("container"))
                .risk("sensitive"),
            move |args| {
                let ui = stop_ui()?;
                let want = args["container"].as_str().unwrap_or_default();
                let (id, name) = container_named(&ui, want)
                    .ok_or_else(|| format!("no container here is called \"{want}\""))?;
                command(&ui, &stop_health, &["stop", id.as_str()])?;
                match observed_state(&ui, &id) {
                    Some(status) if status == "running" => Err(disagree(
                        &ui,
                        format!("“{name}” is still running after stop"),
                    )),
                    Some(status) => Ok(serde_json::json!({ "stopped": name, "status": status })),
                    None => Ok(serde_json::json!({ "stopped": name, "status": "gone" })),
                }
            },
        )
        .action(
            Action::new("restart", "Restart a container")
                .arg(Param::text("container"))
                .risk("sensitive"),
            move |args| {
                let ui = restart_ui()?;
                let want = args["container"].as_str().unwrap_or_default();
                let (id, name) = container_named(&ui, want)
                    .ok_or_else(|| format!("no container here is called \"{want}\""))?;
                command(&ui, &restart_health, &["restart", id.as_str()])?;
                let status = observed_state(&ui, &id).unwrap_or_else(|| "gone".to_string());
                Ok(serde_json::json!({ "restarted": name, "status": status }))
            },
        )
        .action(
            Action::new("show_logs", "Open the log pane for a container, and read the tail")
                .arg(Param::text("container")),
            move |args| {
                let ui = logs_ui()?;
                let want = args["container"].as_str().unwrap_or_default();
                let (id, name) = container_named(&ui, want)
                    .ok_or_else(|| format!("no container here is called \"{want}\""))?;
                read_logs(&ui, &logs_health, &id)?;
                let log = ui.get_log_text().to_string();
                let tail: String = log.lines().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
                Ok(serde_json::json!({ "container": name, "tail": tail }))
            },
        )
        .action(
            // No undo, and the container's writable layer goes with it.
            Action::new("remove", "Delete a container")
                .arg(Param::text("container"))
                .risk("dangerous"),
            move |args| {
                let ui = remove_ui()?;
                let want = args["container"].as_str().unwrap_or_default();
                let (id, name) = container_named(&ui, want)
                    .ok_or_else(|| format!("no container here is called \"{want}\""))?;
                command(&ui, &remove_health, &["rm", "-f", id.as_str()])?;
                // The one that mattered most: this action is graded `dangerous`, and it used to
                // answer `{"removed": name}` without ever looking at whether the container was
                // gone. A grade on an action that fabricates its outcome approves nothing.
                if observed_state(&ui, &id).is_some() {
                    return Err(disagree(
                        &ui,
                        format!("“{name}” is still listed after rm -f"),
                    ));
                }
                Ok(serde_json::json!({ "removed": name }))
            },
        )
        .serve();
}

// ── Wire all callbacks ───────────────────────────────────────────────

fn wire(app: &ContainerManagerApp, health: &Health) {
    app.set_runtime_name(runtime::detect().into());

    // Initial load
    refresh(app, health);

    // Published after the first read, so a describe reports real containers.
    publish_control(app, health);

    // Tab switch
    app.on_ct_tab(|_tab| {
        tracing::debug!("Switched to tab {}", _tab);
    });

    // Refresh
    {
        let weak = app.as_weak();
        let health = health.clone();
        app.on_ct_refresh(move || {
            if let Some(ui) = weak.upgrade() {
                refresh(&ui, &health);
            }
        });
    }

    // ── The mutations ──
    //
    // Each button body is one call into `command`, which is the same call the matching control
    // action makes, so there is nothing on the person's path that the mind's path skips. The
    // button drops the result because it has nowhere to return it to; what it cannot drop is the
    // notice, which `command` has already put on screen.
    {
        let weak = app.as_weak();
        let health = health.clone();
        app.on_ct_start(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            let _ = command(&ui, &health, &["start", id.as_str()]);
        });
    }
    {
        let weak = app.as_weak();
        let health = health.clone();
        app.on_ct_stop(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            let _ = command(&ui, &health, &["stop", id.as_str()]);
        });
    }
    {
        let weak = app.as_weak();
        let health = health.clone();
        app.on_ct_restart(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            let _ = command(&ui, &health, &["restart", id.as_str()]);
        });
    }
    {
        let weak = app.as_weak();
        let health = health.clone();
        app.on_ct_remove(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            let _ = command(&ui, &health, &["rm", "-f", id.as_str()]);
        });
    }
    {
        let weak = app.as_weak();
        let health = health.clone();
        app.on_ct_run(move |image| {
            let Some(ui) = weak.upgrade() else { return };
            let _ = command(&ui, &health, &["run", "-d", image.as_str()]);
        });
    }
    {
        let weak = app.as_weak();
        let health = health.clone();
        app.on_ct_pull(move |image| {
            let Some(ui) = weak.upgrade() else { return };
            let _ = command(&ui, &health, &["pull", image.as_str()]);
        });
    }
    {
        let weak = app.as_weak();
        let health = health.clone();
        app.on_ct_remove_image(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            let _ = command(&ui, &health, &["rmi", id.as_str()]);
        });
    }
    {
        let weak = app.as_weak();
        let health = health.clone();
        app.on_ct_create_volume(move |name| {
            let Some(ui) = weak.upgrade() else { return };
            let _ = command(&ui, &health, &["volume", "create", name.as_str()]);
        });
    }
    {
        let weak = app.as_weak();
        let health = health.clone();
        app.on_ct_remove_volume(move |name| {
            let Some(ui) = weak.upgrade() else { return };
            let _ = command(&ui, &health, &["volume", "rm", name.as_str()]);
        });
    }

    // View logs
    {
        let weak = app.as_weak();
        let health = health.clone();
        app.on_ct_logs(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            let _ = read_logs(&ui, &health, &id.to_string());
        });
    }

    // ── The header's AI button ──
    //
    // It used to be wired to a handler whose whole body was a `tracing::info!`, while the rail
    // beside it had the real `companion::ask` path: the panel opened, stayed empty, and nothing
    // had been asked. Both controls now ask the same question of the same companion.
    {
        let weak = app.as_weak();
        app.on_ai_summarize_logs(move || {
            let Some(ui) = weak.upgrade() else { return };
            let Some(prompt) = log_question(&ui) else {
                ui.set_ai_response("There is no log open to read.".into());
                return;
            };
            ui.set_ai_is_working(true);
            ui.set_ai_response(SharedString::new());
            let back = ui.as_weak();
            std::thread::spawn(move || {
                let outcome = companion::ask(&prompt);
                let _ = back.upgrade_in_event_loop(move |ui| {
                    ui.set_ai_is_working(false);
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
                ui.set_ai_response(SharedString::new());
            }
        });
    }
}
