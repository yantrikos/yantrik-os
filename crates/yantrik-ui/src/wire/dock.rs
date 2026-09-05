//! Dock wiring — on_launch_app callback.

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::App;

/// Wire on_launch_app callback.
pub fn wire(ui: &App, ctx: &AppContext) {
    let apps = ctx.installed_apps.clone();
    let ui_weak = ui.as_weak();

    ui.on_launch_app(move |app_id| {
        let app = app_id.to_string();
        tracing::info!(app = %app, "Launching app");

        // Check installed .desktop apps first (skip built-in Yantrik apps)
        for entry in apps.iter() {
            if entry.app_id == app || entry.name.to_lowercase() == app {
                if entry.exec == "__builtin__" {
                    break; // Fall through to built-in screen routing below
                }
                // A pin or the Lens can name an app ("notes") that ALSO has a .desktop entry
                // (Name=Notes); this branch matches first, so it must launch exactly like the
                // arms below do — same resolution, same environment scrubbing.
                let parts: Vec<&str> = entry.exec.split_whitespace().collect();
                if let Some((bin, args)) = parts.split_first() {
                    spawn_app_with_args(bin, args);
                }
                return;
            }
        }

        // Fallback: hardcoded commands
        let cmd = match app.as_str() {
            "terminal" => {
                spawn_app("yantrik-terminal");
                return;
            }
            "browser" => {
                // Launch visible Chromium with Wayland + separate user-data-dir
                // (headless instance may be holding the default profile lock)
                match std::process::Command::new("chromium")
                    .args([
                        "--ozone-platform=wayland",
                        "--no-first-run",
                        "--no-default-browser-check",
                        "--disable-gpu",
                        "--user-data-dir=/tmp/chromium-visible",
                    ])
                    .env("WAYLAND_DISPLAY", "wayland-0")
                    .env("XDG_RUNTIME_DIR", "/run/user/1000")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                {
                    Ok(_) => tracing::info!("Browser launched (visible mode)"),
                    Err(e) => tracing::error!(error = %e, "Failed to launch browser"),
                }
                return;
            }
            "files" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(8);
                    ui.invoke_navigate(8);
                }
                return;
            }
            "settings" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(7);
                    ui.invoke_navigate(7);
                }
                return;
            }
            "notes" => {
                spawn_app("yantrik-notes");
                return;
            }
            "editor" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_editor_file_name("untitled".into());
                    ui.set_editor_file_content("".into());
                    ui.set_editor_is_modified(false);
                    ui.set_editor_is_readonly(false);
                    ui.set_current_screen(12);
                    ui.invoke_navigate(12);
                }
                return;
            }
            "bond" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(4);
                    ui.invoke_navigate(4);
                }
                return;
            }
            "personality" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(5);
                    ui.invoke_navigate(5);
                }
                return;
            }
            "memory" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(6);
                    ui.invoke_navigate(6);
                }
                return;
            }
            "notifications" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(9);
                    ui.invoke_navigate(9);
                }
                return;
            }
            "system" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(10);
                    ui.invoke_navigate(10);
                }
                return;
            }
            "media" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(13);
                    ui.invoke_navigate(13);
                }
                return;
            }
            "email" => {
                spawn_app("yantrik-email");
                return;
            }
            "calendar" => {
                spawn_app("yantrik-calendar");
                return;
            }
            "packages" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(21);
                    ui.invoke_navigate(21);
                }
                return;
            }
            "network" => {
                spawn_app("yantrik-network-manager");
                return;
            }
            "sysmonitor" | "system_monitor" => {
                spawn_app("yantrik-system-monitor");
                return;
            }
            "weather" => {
                spawn_app("yantrik-weather");
                return;
            }
            "music" | "music_player" => {
                spawn_app("yantrik-music-player");
                return;
            }
            "downloads" | "download_manager" => {
                spawn_app("yantrik-download-manager");
                return;
            }
            "snippets" | "snippet_manager" => {
                spawn_app("yantrik-snippet-manager");
                return;
            }
            "containers" | "container_manager" => {
                spawn_app("yantrik-container-manager");
                return;
            }
            "devices" | "device_dashboard" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(27);
                    ui.invoke_navigate(27);
                }
                return;
            }
            "permissions" | "permission_dashboard" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(28);
                    ui.invoke_navigate(28);
                }
                return;
            }
            "spreadsheet" => {
                spawn_app("yantrik-spreadsheet");
                return;
            }
            "documents" | "document_editor" => {
                spawn_app("yantrik-document-editor");
                return;
            }
            "presentation" | "slides" => {
                spawn_app("yantrik-presentation");
                return;
            }
            "launchpad" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(1);
                    ui.invoke_navigate(1);
                    ui.set_app_grid_open(true);
                }
                return;
            }
            _ => {
                tracing::warn!(app = %app, "Unknown app");
                return;
            }
        };
    });
}

/// Where an app binary lives when `bin` is a bare name.
///
/// The shell is started from `/opt/yantrik/bin` (or a cargo target dir in development), and the
/// apps are deployed beside it — but nothing puts that directory on PATH, so a bare
/// `Command::new("yantrik-notes")` fails with ENOENT on a clean install and the launcher logs
/// "Failed to launch" for every app. Prefer the shell's own directory, then the deploy path, and
/// only then whatever PATH says.
pub fn resolve_app_binary(bin: &str) -> std::path::PathBuf {
    use std::path::{Path, PathBuf};
    if bin.contains('/') {
        return PathBuf::from(bin);
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(bin));
        }
    }
    candidates.push(Path::new("/opt/yantrik/bin").join(bin));
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(bin))
}

/// Launch a standalone app binary. The app's own single-instance guard handles repeats.
pub fn spawn_app(bin: &str) {
    spawn_app_with_args(bin, &[]);
}

/// The one place the shell starts an app process, whatever path asked for it.
pub fn spawn_app_with_args(bin: &str, args: &[&str]) {
    let path = resolve_app_binary(bin);
    match std::process::Command::new(&path)
        .args(args)
        // The shell is often started with SLINT_FULLSCREEN=1 (dev runs, kiosk sessions). A child
        // inherits the environment, and an app that inherits that variable opens fullscreen too.
        // The renderer choice (SLINT_BACKEND, GALLIUM_DRIVER) is deliberately left inherited so
        // apps draw with the same backend the shell settled on.
        .env_remove("SLINT_FULLSCREEN")
        .stdin(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            tracing::info!(app = bin, path = %path.display(), "App launched");
            // Reap it when it exits. Without a wait, every app the shell ever launched lingers
            // as a zombie until the shell itself quits — and a zombie still has a /proc entry,
            // which is enough to confuse anything that checks "is that pid alive".
            let name = bin.to_string();
            std::thread::spawn(move || match child.wait() {
                Ok(status) => tracing::info!(app = %name, %status, "App exited"),
                Err(e) => tracing::warn!(app = %name, error = %e, "Could not wait for app"),
            });
        }
        Err(e) => tracing::error!(app = bin, path = %path.display(), error = %e, "Failed to launch app"),
    }
}
