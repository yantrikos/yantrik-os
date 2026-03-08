//! Component version registry and update checker.
//!
//! Embeds per-component versions at compile time and checks
//! releases.yantrikos.com/manifest.json for available updates.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::{App, ComponentVersionData};

/// A component with its embedded version info.
#[derive(Clone, Debug)]
pub struct ComponentInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub git_hash: &'static str,
}

/// Result of an update check for one component.
#[derive(Clone, Debug)]
struct UpdateInfo {
    name: String,
    current: String,
    latest: String,
    has_update: bool,
}

/// All component versions baked in at compile time.
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

/// Check for updates from the release server.
/// Returns a list of components with update availability.
fn check_updates(components: &[ComponentInfo], channel: &str) -> Vec<UpdateInfo> {
    let url = format!("http://releases.yantrikos.com/manifest.json");

    let manifest: serde_json::Value = match ureq::get(&url).call() {
        Ok(resp) => match resp.into_json() {
            Ok(v) => v,
            Err(_) => return components.iter().map(|c| no_update(c)).collect(),
        },
        Err(_) => return components.iter().map(|c| no_update(c)).collect(),
    };

    let channel_data = &manifest["channels"][channel];
    let remote_components = &channel_data["components"];

    components
        .iter()
        .map(|c| {
            let latest = remote_components[c.name]["version"]
                .as_str()
                .unwrap_or(c.version);
            UpdateInfo {
                name: c.name.to_string(),
                current: c.version.to_string(),
                latest: latest.to_string(),
                has_update: version_newer(latest, c.version),
            }
        })
        .collect()
}

fn no_update(c: &ComponentInfo) -> UpdateInfo {
    UpdateInfo {
        name: c.name.to_string(),
        current: c.version.to_string(),
        latest: c.version.to_string(),
        has_update: false,
    }
}

/// Simple semver comparison: is `remote` newer than `local`?
fn version_newer(remote: &str, local: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> {
        v.split('-')
            .next()
            .unwrap_or(v)
            .split('.')
            .filter_map(|s| s.parse().ok())
            .collect()
    };
    let r = parse(remote);
    let l = parse(local);
    r > l
}

/// Wire version info into the About screen.
pub fn wire(ui: &App, _ctx: &AppContext) {
    // Populate component versions immediately
    let components = embedded_components();
    let items: Vec<ComponentVersionData> = components
        .iter()
        .map(|c| ComponentVersionData {
            name: c.name.into(),
            current_version: format!("{} ({})", c.version, c.git_hash).into(),
            latest_version: "".into(),
            has_update: false,
            is_checking: false,
        })
        .collect();
    ui.set_about_components(ModelRc::new(VecModel::from(items)));

    // Check for updates callback
    let ui_weak = ui.as_weak();
    ui.on_about_check_updates(move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        // Mark all as checking
        let components = embedded_components();
        let items: Vec<ComponentVersionData> = components
            .iter()
            .map(|c| ComponentVersionData {
                name: c.name.into(),
                current_version: format!("{} ({})", c.version, c.git_hash).into(),
                latest_version: "checking...".into(),
                has_update: false,
                is_checking: true,
            })
            .collect();
        ui.set_about_components(ModelRc::new(VecModel::from(items)));
        ui.set_about_update_status("Checking for updates...".into());

        let (tx, rx) = mpsc::channel::<Vec<UpdateInfo>>();
        let comps = embedded_components();

        std::thread::spawn(move || {
            let results = check_updates(&comps, "stable");
            let _ = tx.send(results);
        });

        let weak = ui_weak.clone();
        let timer = Timer::default();
        let timer_holder: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
        let holder = timer_holder.clone();

        timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
            if let Ok(results) = rx.try_recv() {
                if let Some(ui) = weak.upgrade() {
                    let update_count = results.iter().filter(|r| r.has_update).count();
                    let items: Vec<ComponentVersionData> = results
                        .iter()
                        .map(|r| ComponentVersionData {
                            name: r.name.clone().into(),
                            current_version: r.current.clone().into(),
                            latest_version: r.latest.clone().into(),
                            has_update: r.has_update,
                            is_checking: false,
                        })
                        .collect();
                    ui.set_about_components(ModelRc::new(VecModel::from(items)));
                    ui.set_about_update_count(update_count as i32);

                    let status = if update_count > 0 {
                        format!("{} update(s) available", update_count)
                    } else {
                        "All components up to date".to_string()
                    };
                    ui.set_about_update_status(status.into());
                }
                *holder.borrow_mut() = None;
            }
        });
        *timer_holder.borrow_mut() = Some(timer);
    });
}
