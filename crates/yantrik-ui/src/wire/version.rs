//! The About screen's updates panel: what is installed, which channel this machine follows,
//! what that channel has, and the two buttons that change either.
//!
//! ── What this file used to be ──
//!
//! A second update checker. It fetched `http://releases.yantrikos.com/manifest.json` — plain
//! http, hardcoded, no matter what the machine was configured with — with the channel hardcoded
//! to `"stable"`, and read `channels[ch]["components"][<crate>]["version"]`: a shape no manifest
//! this project has ever published. So the lookup always missed, `latest` always fell back to
//! the local version, and `has_update` was always false. Every error path — DNS failure, refused
//! connection, malformed JSON — did `return components.map(no_update)`, and the screen printed
//! "All components up to date".
//!
//! It could not answer anything else. A screen whose only possible answer is "everything is
//! fine" is not a check; it is a picture of one, and it was the picture shown on machines whose
//! configured channel was not even `stable`.
//!
//! Meanwhile the shell already ran the real updater for its control surface (`control_update`),
//! which reads `/opt/yantrik/update.conf`, talks to the configured host over the configured
//! scheme, and can tell "the server did not answer" from "that channel has nothing on it".
//!
//! Now there is one path: this screen calls `control_update`, which runs `yantrik-update`. The
//! script is the only thing that parses update.conf and the only thing that writes it.
//!
//! The COMPONENTS table stays, because it is fed: `build.rs` bakes each workspace crate's
//! version in at compile time (`COMPONENT_*_VERSION`). It lists what is installed and claims
//! nothing about what is available — per-component update availability is not something any
//! manifest this project publishes could answer, and pretending otherwise is what got us here.

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::app_context::AppContext;
use crate::control_update::{self, CheckOutcome};
use crate::{App, ComponentVersionData};

/// A component with its embedded version info.
#[derive(Clone, Debug)]
pub struct ComponentInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub git_hash: &'static str,
}

/// All component versions baked in at compile time by `build.rs`.
pub fn embedded_components() -> Vec<ComponentInfo> {
    vec![
        ComponentInfo {
            name: "yantrik-ml",
            version: option_env!("COMPONENT_YANTRIK_ML_VERSION").unwrap_or("0.1.0"),
            git_hash: option_env!("COMPONENT_YANTRIK_ML_GIT").unwrap_or("unknown"),
        },
        ComponentInfo {
            name: "yantrikdb",
            version: option_env!("COMPONENT_YANTRIKDB_CORE_VERSION").unwrap_or("0.1.0"),
            git_hash: option_env!("COMPONENT_YANTRIKDB_CORE_GIT").unwrap_or("unknown"),
        },
        ComponentInfo {
            name: "yantrik-companion",
            version: option_env!("COMPONENT_YANTRIK_COMPANION_VERSION").unwrap_or("0.1.0"),
            git_hash: option_env!("COMPONENT_YANTRIK_COMPANION_GIT").unwrap_or("unknown"),
        },
        ComponentInfo {
            name: "yantrik-os",
            version: option_env!("COMPONENT_YANTRIK_OS_VERSION").unwrap_or("0.1.0"),
            git_hash: option_env!("COMPONENT_YANTRIK_OS_GIT").unwrap_or("unknown"),
        },
        ComponentInfo {
            name: "yantrik-ui",
            version: option_env!("COMPONENT_YANTRIK_UI_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")),
            git_hash: option_env!("COMPONENT_YANTRIK_UI_GIT").unwrap_or("unknown"),
        },
    ]
}

/// Put the outcome of a check on the screen. One function, so "up to date" and "could not
/// check" are set from the same place and cannot drift into meaning the same thing.
fn show_outcome(ui: &App, outcome: &CheckOutcome) {
    ui.set_about_update_state(outcome.state().into());
    ui.set_about_update_status(outcome.headline().into());
    ui.set_about_update_busy(false);
    match outcome {
        CheckOutcome::UpToDate { installed, git } => {
            ui.set_about_update_latest(
                if git.is_empty() { installed.clone() } else { format!("{installed} ({git})") }
                    .into(),
            );
        }
        CheckOutcome::UpdateAvailable { to, .. } => {
            ui.set_about_update_latest(to.clone().into());
        }
        // The channel's build is still a fact worth showing, and it is the fact that explains
        // the sentence: this is what the machine is ahead OF.
        CheckOutcome::Ahead { channel_build, .. } => {
            ui.set_about_update_latest(channel_build.clone().into());
        }
        // The channel has nothing to report, so the field says nothing rather than keeping
        // whatever the last successful check left there.
        _ => ui.set_about_update_latest("".into()),
    }
}

/// Read this machine's update configuration onto the screen. Called after anything that can
/// change it, and always by re-asking the updater rather than remembering what we asked for.
fn show_status(ui: &App, status: &control_update::UpdateStatus) {
    ui.set_about_update_channel(status.channel.clone().into());
    ui.set_about_update_host(
        if status.host.is_empty() {
            String::new()
        } else {
            format!("{}://{}", status.scheme, status.host)
        }
        .into(),
    );
    ui.set_about_update_installed(status.installed_label().into());
    ui.set_about_update_can_set_channel(status.conf_writable);
}

/// Run a check on a worker thread and put the answer on the screen.
fn check_off_thread(weak: slint::Weak<App>) {
    if let Some(ui) = weak.upgrade() {
        ui.set_about_update_busy(true);
        ui.set_about_update_state("checking".into());
        ui.set_about_update_status("Checking…".into());
    }
    std::thread::spawn(move || {
        // Both reads happen out here, off the UI thread: `check` makes a network request with a
        // ten-second timeout, and a frozen desktop is the other way to make an update look
        // broken.
        let status = control_update::read_status().ok();
        let outcome = control_update::check_now(None);
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if let Some(s) = status {
                show_status(&ui, &s);
            }
            show_outcome(&ui, &outcome);
        });
    });
}

/// Wire the About screen's version and update panel.
pub fn wire(ui: &App, _ctx: &AppContext) {
    // The installed component table, immediately and from nothing but this binary.
    let items: Vec<ComponentVersionData> = embedded_components()
        .iter()
        .map(|c| ComponentVersionData {
            name: c.name.into(),
            current_version: format!("{} ({})", c.version, c.git_hash).into(),
        })
        .collect();
    ui.set_about_components(ModelRc::new(VecModel::from(items)));

    // Which channel this machine is on, read at startup so the screen states it before anyone
    // presses anything. No network: `status` only reads local files.
    {
        let weak = ui.as_weak();
        std::thread::spawn(move || match control_update::read_status() {
            Ok(status) => {
                let _ = weak.upgrade_in_event_loop(move |ui| show_status(&ui, &status));
            }
            Err(e) => {
                // Say so rather than showing an empty channel field that looks like a value.
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    ui.set_about_update_channel("unknown".into());
                    ui.set_about_update_state("failed".into());
                    ui.set_about_update_status(format!("Could not read the update settings — {e}").into());
                });
            }
        });
    }

    let weak = ui.as_weak();
    ui.on_about_check_updates(move || check_off_thread(weak.clone()));

    // ── Install ──
    //
    // The same call the control surface's `apply_update` makes: the updater, detached, because
    // the next thing it does is stop this shell. There is no progress to show here — the shell
    // this screen is drawn by goes away and comes back as the new build.
    //
    // Neither flag is ever set from this screen, and there is deliberately no control here that
    // sets them. The button only exists while a check has answered "the channel is newer", so a
    // machine ahead of its channel never sees one — and a downgrade, which is what installing
    // the channel's build on such a machine would be, stays something a person asks for by
    // name: `yantrik-update apply --allow-downgrade`, or `apply_update` with allow_downgrade.
    let weak = ui.as_weak();
    ui.on_about_install_update(move || {
        let Some(ui) = weak.upgrade() else { return };
        match control_update::spawn_apply(None, false, false) {
            Ok(()) => {
                ui.set_about_update_state("applying".into());
                ui.set_about_update_status(
                    "Installing — the desktop will restart. If the new build does not start, the \
                     previous one is restored automatically."
                        .into(),
                );
                ui.set_about_update_busy(true);
            }
            Err(e) => {
                ui.set_about_update_state("failed".into());
                ui.set_about_update_status(format!("Could not start the install — {e}").into());
            }
        }
    });

    // ── Channel picker ──
    //
    // Writes through the script, which is the single owner of update.conf, then re-reads the
    // answer and re-runs the check. "beta is not published yet" is a result, shown in the same
    // status line as any other — not an error dialog. It is the one failure a person fixes by
    // picking a different item in this very control.
    let weak = ui.as_weak();
    ui.on_about_set_channel(move |channel| {
        let channel = channel.to_string();
        let weak = weak.clone();
        if let Some(ui) = weak.upgrade() {
            ui.set_about_update_busy(true);
            ui.set_about_update_state("checking".into());
            ui.set_about_update_status(format!("Switching to {channel}…").into());
        }
        std::thread::spawn(move || match control_update::set_channel(&channel) {
            Ok(status) => {
                let outcome = control_update::check_now(None);
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    show_status(&ui, &status);
                    show_outcome(&ui, &outcome);
                });
            }
            Err(e) => {
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    ui.set_about_update_busy(false);
                    ui.set_about_update_state("failed".into());
                    ui.set_about_update_status(format!("Could not change the channel — {e}").into());
                });
            }
        });
    });
}
