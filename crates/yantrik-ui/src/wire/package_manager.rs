//! Package Manager wire module — apk list, search, info, install, remove, upgrade.
//!
//! All heavy operations (apk commands) run in background threads.
//! UI is updated via Slint Timers polling oneshot channels.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::{App, PackageData};

/// Parsed package entry from apk output.
#[derive(Clone, Debug)]
struct PkgEntry {
    name: String,
    version: String,
    description: String,
    installed: bool,
    upgradable: bool,
    size_text: String,
    repo: String,
}

/// Parsed package detail from `apk info -a`.
#[derive(Clone, Debug, Default)]
struct PkgDetail {
    name: String,
    version: String,
    description: String,
    maintainer: String,
    dependencies: String,
    size: String,
    repo: String,
    installed: bool,
    upgradable: bool,
}

/// Wire package manager callbacks.
pub fn wire(ui: &App, _ctx: &AppContext) {
    let pkg_cache: Rc<RefCell<Vec<PkgEntry>>> = Rc::new(RefCell::new(Vec::new()));
    let poll_timer: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));

    // ── Search callback ──
    {
        let ui_weak = ui.as_weak();
        let cache = pkg_cache.clone();
        ui.on_pkg_search(move |query| {
            let query_str = query.to_string();
            let cache_ref = cache.borrow();

            if query_str.is_empty() {
                // Show all cached packages
                if let Some(ui) = ui_weak.upgrade() {
                    let items = cache_to_model(&cache_ref);
                    ui.set_pkg_packages(ModelRc::new(VecModel::from(items)));
                    ui.set_pkg_status_text(
                        format!("{} packages", cache_ref.len()).into(),
                    );
                }
                return;
            }

            // Filter from cache
            let lower = query_str.to_lowercase();
            let filtered: Vec<&PkgEntry> = cache_ref
                .iter()
                .filter(|p| {
                    p.name.to_lowercase().contains(&lower)
                        || p.description.to_lowercase().contains(&lower)
                })
                .collect();

            if let Some(ui) = ui_weak.upgrade() {
                let items: Vec<PackageData> = filtered
                    .iter()
                    .map(|p| pkg_to_model(p))
                    .collect();
                let count = items.len();
                ui.set_pkg_packages(ModelRc::new(VecModel::from(items)));
                ui.set_pkg_status_text(
                    format!("{} packages matching '{}'", count, query_str).into(),
                );
                ui.set_pkg_selected_index(-1);
                clear_detail(&ui);
            }
        });
    }

    // ── Filter changed callback ──
    {
        let ui_weak = ui.as_weak();
        let cache = pkg_cache.clone();
        ui.on_pkg_filter_changed(move |filter_idx| {
            let cache_ref = cache.borrow();
            let filtered: Vec<&PkgEntry> = match filter_idx {
                1 => cache_ref.iter().filter(|p| p.installed).collect(),
                2 => cache_ref.iter().filter(|p| p.upgradable).collect(),
                _ => cache_ref.iter().collect(),
            };

            if let Some(ui) = ui_weak.upgrade() {
                let items: Vec<PackageData> = filtered
                    .iter()
                    .map(|p| pkg_to_model(p))
                    .collect();
                let label = match filter_idx {
                    1 => "installed",
                    2 => "upgradable",
                    _ => "total",
                };
                ui.set_pkg_status_text(
                    format!("{} {} packages", items.len(), label).into(),
                );
                ui.set_pkg_packages(ModelRc::new(VecModel::from(items)));
                ui.set_pkg_selected_index(-1);
                clear_detail(&ui);
            }
        });
    }

    // ── Refresh callback — loads installed + upgradable in background ──
    {
        let ui_weak = ui.as_weak();
        let timer_ref = poll_timer.clone();
        let cache = pkg_cache.clone();
        ui.on_pkg_refresh(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_pkg_is_loading(true);
                ui.set_pkg_status_text("Updating package database...".into());
                ui.set_pkg_error_text("".into());
            }

            let (tx, rx) = mpsc::channel::<Result<Vec<PkgEntry>, String>>();

            // Background thread: apk update + list installed + list upgradable
            std::thread::spawn(move || {
                // Run apk update first
                let _ = std::process::Command::new("sudo")
                    .args(["apk", "update"])
                    .output();

                let mut packages: Vec<PkgEntry> = Vec::new();

                // List installed packages
                match std::process::Command::new("apk")
                    .args(["list", "--installed"])
                    .output()
                {
                    Ok(output) if output.status.success() => {
                        let text = String::from_utf8_lossy(&output.stdout);
                        for line in text.lines() {
                            if let Some(pkg) = parse_apk_list_line(line, true) {
                                packages.push(pkg);
                            }
                        }
                    }
                    Ok(output) => {
                        let err = String::from_utf8_lossy(&output.stderr);
                        let _ = tx.send(Err(format!("apk list failed: {}", err)));
                        return;
                    }
                    Err(e) => {
                        let _ = tx.send(Err(format!("Failed to run apk: {}", e)));
                        return;
                    }
                }

                // List upgradable packages
                match std::process::Command::new("apk")
                    .args(["list", "--upgradable"])
                    .output()
                {
                    Ok(output) if output.status.success() => {
                        let text = String::from_utf8_lossy(&output.stdout);
                        for line in text.lines() {
                            let line = line.trim();
                            if line.is_empty() {
                                continue;
                            }
                            // Extract package name from upgradable line
                            if let Some(name) = extract_pkg_name(line) {
                                // Mark matching package as upgradable
                                for pkg in &mut packages {
                                    if pkg.name == name {
                                        pkg.upgradable = true;
                                    }
                                }
                            }
                        }
                    }
                    _ => {} // Non-fatal
                }

                packages.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                let _ = tx.send(Ok(packages));
            });

            // Poll for result
            let weak = ui_weak.clone();
            let handle = timer_ref.clone();
            let cache_inner = cache.clone();
            let timer = Timer::default();
            timer.start(TimerMode::Repeated, Duration::from_millis(50), move || {
                if let Ok(result) = rx.try_recv() {
                    if let Some(ui) = weak.upgrade() {
                        match result {
                            Ok(packages) => {
                                let count = packages.len();
                                let upgradable = packages.iter().filter(|p| p.upgradable).count();
                                let items = cache_to_model(&packages);
                                *cache_inner.borrow_mut() = packages;
                                ui.set_pkg_packages(ModelRc::new(VecModel::from(items)));
                                ui.set_pkg_upgradable_count(upgradable as i32);
                                ui.set_pkg_status_text(
                                    format!("{} installed packages, {} upgradable", count, upgradable).into(),
                                );
                                ui.set_pkg_is_loading(false);
                            }
                            Err(err) => {
                                ui.set_pkg_error_text(err.into());
                                ui.set_pkg_is_loading(false);
                                ui.set_pkg_status_text("Error loading packages".into());
                            }
                        }
                    }
                    *handle.borrow_mut() = None;
                }
            });
            *timer_ref.borrow_mut() = Some(timer);
        });
    }

    // ── Select package — fetch details in background ──
    {
        let ui_weak = ui.as_weak();
        let timer_ref = poll_timer.clone();
        let cache = pkg_cache.clone();
        ui.on_pkg_select_package(move |idx| {
            let cache_ref = cache.borrow();
            let active_filter = if let Some(ui) = ui_weak.upgrade() {
                ui.get_pkg_active_filter()
            } else {
                return;
            };

            // Resolve the actual package from filtered view
            let filtered: Vec<&PkgEntry> = match active_filter {
                1 => cache_ref.iter().filter(|p| p.installed).collect(),
                2 => cache_ref.iter().filter(|p| p.upgradable).collect(),
                _ => cache_ref.iter().collect(),
            };

            let pkg = match filtered.get(idx as usize) {
                Some(p) => (*p).clone(),
                None => return,
            };

            let pkg_name = pkg.name.clone();

            // Set basic detail immediately from cache
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_pkg_detail_name(pkg.name.clone().into());
                ui.set_pkg_detail_version(pkg.version.clone().into());
                ui.set_pkg_detail_description(pkg.description.clone().into());
                ui.set_pkg_detail_installed(pkg.installed);
                ui.set_pkg_detail_upgradable(pkg.upgradable);
                ui.set_pkg_detail_size(pkg.size_text.clone().into());
                ui.set_pkg_detail_repo(pkg.repo.clone().into());
                ui.set_pkg_detail_maintainer("".into());
                ui.set_pkg_detail_dependencies("".into());
            }

            // Fetch full detail in background
            let (tx, rx) = mpsc::channel::<PkgDetail>();

            std::thread::spawn(move || {
                let detail = fetch_pkg_detail(&pkg_name, pkg.installed, pkg.upgradable);
                let _ = tx.send(detail);
            });

            let weak = ui_weak.clone();
            let handle = timer_ref.clone();
            let timer = Timer::default();
            timer.start(TimerMode::Repeated, Duration::from_millis(50), move || {
                if let Ok(detail) = rx.try_recv() {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_pkg_detail_description(detail.description.into());
                        ui.set_pkg_detail_maintainer(detail.maintainer.into());
                        ui.set_pkg_detail_dependencies(detail.dependencies.into());
                        if !detail.size.is_empty() {
                            ui.set_pkg_detail_size(detail.size.into());
                        }
                        if !detail.repo.is_empty() {
                            ui.set_pkg_detail_repo(detail.repo.into());
                        }
                    }
                    *handle.borrow_mut() = None;
                }
            });
            *timer_ref.borrow_mut() = Some(timer);
        });
    }

    // ── Install package ──
    {
        let ui_weak = ui.as_weak();
        let timer_ref = poll_timer.clone();
        ui.on_pkg_install_package(move |name| {
            let pkg_name = name.to_string();
            run_pkg_action(
                &ui_weak,
                &timer_ref,
                pkg_name.clone(),
                "install",
                &["sudo", "apk", "add", "--no-cache", &pkg_name],
            );
        });
    }

    // ── Remove package ──
    {
        let ui_weak = ui.as_weak();
        let timer_ref = poll_timer.clone();
        ui.on_pkg_remove_package(move |name| {
            let pkg_name = name.to_string();
            run_pkg_action(
                &ui_weak,
                &timer_ref,
                pkg_name.clone(),
                "remove",
                &["sudo", "apk", "del", &pkg_name],
            );
        });
    }

    // ── Upgrade single package ──
    {
        let ui_weak = ui.as_weak();
        let timer_ref = poll_timer.clone();
        ui.on_pkg_upgrade_package(move |name| {
            let pkg_name = name.to_string();
            run_pkg_action(
                &ui_weak,
                &timer_ref,
                pkg_name.clone(),
                "upgrade",
                &["sudo", "apk", "add", "--upgrade", &pkg_name],
            );
        });
    }

    // ── Upgrade all ──
    {
        let ui_weak = ui.as_weak();
        let timer_ref = poll_timer.clone();
        ui.on_pkg_upgrade_all(move || {
            run_pkg_action(
                &ui_weak,
                &timer_ref,
                "all packages".to_string(),
                "upgrade",
                &["sudo", "apk", "upgrade"],
            );
        });
    }

    // ── Apply changes (currently not batched — direct actions) ──
    {
        ui.on_pkg_apply_changes(move || {
            // Actions are applied immediately in this implementation
        });
    }

    // ── Cancel changes ──
    {
        ui.on_pkg_cancel_changes(move || {
            // No-op in direct-action mode
        });
    }
}

/// Run a package action (install/remove/upgrade) in a background thread.
fn run_pkg_action(
    ui_weak: &slint::Weak<App>,
    timer_ref: &Rc<RefCell<Option<Timer>>>,
    pkg_name: String,
    action: &str,
    args: &[&str],
) {
    if let Some(ui) = ui_weak.upgrade() {
        ui.set_pkg_is_applying(true);
        ui.set_pkg_status_text(
            format!("{}ing {}...", capitalize(action), pkg_name).into(),
        );
        ui.set_pkg_error_text("".into());
    }

    let action_str = action.to_string();
    let args_owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let (tx, rx) = mpsc::channel::<Result<String, String>>();

    std::thread::spawn(move || {
        if args_owned.is_empty() {
            let _ = tx.send(Err("No command".to_string()));
            return;
        }
        let cmd = &args_owned[0];
        let cmd_args = &args_owned[1..];

        match std::process::Command::new(cmd).args(cmd_args).output() {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let _ = tx.send(Ok(stdout));
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let msg = if stderr.contains("Permission denied") || stderr.contains("not permitted") {
                    "Requires root privileges. Configure sudo/doas for the yantrik user.".to_string()
                } else {
                    format!("{} {}", stdout.trim(), stderr.trim())
                };
                let _ = tx.send(Err(msg));
            }
            Err(e) => {
                let _ = tx.send(Err(format!("Failed to execute: {}", e)));
            }
        }
    });

    let weak = ui_weak.clone();
    let handle = timer_ref.clone();
    let action_label = action_str.clone();
    let pkg = pkg_name.clone();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(50), move || {
        if let Ok(result) = rx.try_recv() {
            if let Some(ui) = weak.upgrade() {
                ui.set_pkg_is_applying(false);
                match result {
                    Ok(_) => {
                        ui.set_pkg_status_text(
                            format!("Successfully {}ed {}", action_label, pkg).into(),
                        );
                        // Trigger a refresh to update the list
                        ui.invoke_pkg_refresh();
                    }
                    Err(err) => {
                        ui.set_pkg_error_text(err.into());
                        ui.set_pkg_status_text("Operation failed".into());
                    }
                }
            }
            *handle.borrow_mut() = None;
        }
    });
    *timer_ref.borrow_mut() = Some(timer);
}

/// Parse a line from `apk list --installed` or `apk list --upgradable`.
///
/// Format: `name-version arch {origin} (license) [installed]`
/// Example: `busybox-1.36.1-r2 x86_64 {busybox} (GPL-2.0-only) [installed]`
fn parse_apk_list_line(line: &str, mark_installed: bool) -> Option<PkgEntry> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // Split on first space to get name-version
    let mut parts = line.splitn(2, ' ');
    let name_version = parts.next()?;
    let rest = parts.next().unwrap_or("");

    // Split name-version: last hyphen before a digit starts the version
    let (name, version) = split_name_version(name_version);
    if name.is_empty() {
        return None;
    }

    // Extract description: we don't have it in list output, use name as placeholder
    // The description will be fetched when selected via `apk info`
    let description = extract_description_from_rest(rest);

    // Extract repo from origin: {origin}
    let repo = if let Some(start) = rest.find('{') {
        if let Some(end) = rest.find('}') {
            rest[start + 1..end].to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    Some(PkgEntry {
        name,
        version,
        description,
        installed: mark_installed,
        upgradable: false,
        size_text: String::new(),
        repo,
    })
}

/// Split "package-1.2.3-r0" into ("package", "1.2.3-r0").
fn split_name_version(s: &str) -> (String, String) {
    // Walk backwards from the end, find the last '-' followed by a digit
    let bytes = s.as_bytes();
    for i in (1..bytes.len()).rev() {
        if bytes[i - 1] == b'-' && bytes[i].is_ascii_digit() {
            return (s[..i - 1].to_string(), s[i..].to_string());
        }
    }
    (s.to_string(), String::new())
}

/// Extract package name from an apk list line.
fn extract_pkg_name(line: &str) -> Option<String> {
    let name_ver = line.split_whitespace().next()?;
    let (name, _) = split_name_version(name_ver);
    if name.is_empty() { None } else { Some(name) }
}

/// Extract a brief description from the rest of the apk list line.
fn extract_description_from_rest(rest: &str) -> String {
    // The rest after name-version arch is like: `{origin} (license) [status] - description`
    // Or it might just be architecture info. Use empty for now.
    if let Some(idx) = rest.find(" - ") {
        rest[idx + 3..].trim().to_string()
    } else {
        String::new()
    }
}

/// Fetch detailed info for a package via `apk info -a`.
fn fetch_pkg_detail(name: &str, installed: bool, upgradable: bool) -> PkgDetail {
    let mut detail = PkgDetail {
        name: name.to_string(),
        installed,
        upgradable,
        ..Default::default()
    };

    let output = match std::process::Command::new("apk")
        .args(["info", "-a", name])
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return detail,
    };

    let mut section = "";
    let mut desc_lines: Vec<String> = Vec::new();
    let mut dep_lines: Vec<String> = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();

        // Section headers like "nano-7.2-r0 description:" or "nano-7.2-r0 depends:"
        if trimmed.ends_with("description:") {
            section = "description";
            continue;
        }
        if trimmed.ends_with("depends:") {
            section = "depends";
            continue;
        }
        if trimmed.ends_with("provides:") || trimmed.ends_with("required by:")
            || trimmed.ends_with("contains:") || trimmed.ends_with("triggers:")
        {
            section = "other";
            continue;
        }
        if trimmed.ends_with("installed size:") {
            section = "size";
            continue;
        }
        if trimmed.ends_with("webpage:") {
            section = "webpage";
            continue;
        }

        // Parse maintainer from "name-ver license:" or a specific line
        if trimmed.contains("maintainer:") {
            if let Some(idx) = trimmed.find("maintainer:") {
                detail.maintainer = trimmed[idx + 11..].trim().to_string();
            }
            section = "";
            continue;
        }

        // Parse specific sections
        match section {
            "description" => {
                if !trimmed.is_empty() {
                    desc_lines.push(trimmed.to_string());
                }
            }
            "depends" => {
                if !trimmed.is_empty() {
                    dep_lines.push(trimmed.to_string());
                }
            }
            "size" => {
                if !trimmed.is_empty() {
                    detail.size = trimmed.to_string();
                    section = "";
                }
            }
            _ => {}
        }

        // Try to extract version from "name-version description:" header
        if detail.version.is_empty() && trimmed.contains(' ') {
            let first = trimmed.split_whitespace().next().unwrap_or("");
            let (n, v) = split_name_version(first);
            if n == name && !v.is_empty() {
                detail.version = v;
            }
        }
    }

    if !desc_lines.is_empty() {
        detail.description = desc_lines.join(" ");
    }
    if !dep_lines.is_empty() {
        detail.dependencies = dep_lines.join(", ");
    }

    detail
}

/// Convert cached packages to Slint model items.
fn cache_to_model(cache: &[PkgEntry]) -> Vec<PackageData> {
    cache.iter().map(|p| pkg_to_model(p)).collect()
}

/// Convert a single PkgEntry to a Slint PackageData.
fn pkg_to_model(p: &PkgEntry) -> PackageData {
    PackageData {
        name: p.name.clone().into(),
        version: p.version.clone().into(),
        description: p.description.clone().into(),
        installed: p.installed,
        upgradable: p.upgradable,
        size_text: p.size_text.clone().into(),
        repo: p.repo.clone().into(),
    }
}

/// Clear the detail panel.
fn clear_detail(ui: &App) {
    ui.set_pkg_detail_name("".into());
    ui.set_pkg_detail_version("".into());
    ui.set_pkg_detail_description("".into());
    ui.set_pkg_detail_maintainer("".into());
    ui.set_pkg_detail_dependencies("".into());
    ui.set_pkg_detail_size("".into());
    ui.set_pkg_detail_repo("".into());
    ui.set_pkg_detail_installed(false);
    ui.set_pkg_detail_upgradable(false);
}

/// Capitalize first letter.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}
