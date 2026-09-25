// NOTE: Crate-level allow(unused) works around rustc 1.93.1 ICE in early_lint_checks.
// The ICE is triggered by the lint emission formatter (StyledBuffer::replace panic).
// Remove once rustc is updated past 1.93.1.
#![allow(unused)]

//! Yantrik OS — AI-native desktop shell.
//!
//! The desktop's primary interface. Embeds CompanionService in-process
//! on a worker thread, renders via Slint on the main thread.
//!
//! Layout: boot animation → desktop (particle field, orb, Intent Lens).
//! The Intent Lens is the primary interaction — search, ask, launch, control.
//!
//! Modules:
//! - app_context:    Shared state bundle (AppContext) + initialization
//! - wire:           Callback wiring registry (one sub-module per concern)
//! - streaming:      Shared token streaming helper
//! - lens:           Intent Lens query routing, NL→tool matching, action resolution
//! - cards:          Whisper Card lifecycle (add, dismiss, auto-expire, sync to UI)
//! - focus:          Focus mode countdown timer
//! - notifications:  Notification store + Slint sync helpers
//! - onboarding:     First-boot marker + guided results
//! - system_context: System snapshot formatting, event→memory, config loading
//! - bridge:         Crossbeam companion bridge (send messages, query memory)
//! - features:       Proactive features (ResourceGuardian, ProcessSentinel, etc.)
//! - voice:          Voice input via Whisper
//!
//! Usage:
//!   yantrik-ui [config.yaml]

use std::path::PathBuf;
use yantrik_companion::CompanionConfig;

mod activity_feed;
/// Every agent — one conversation with one mind — and its session, drawn by the Agents screen.
mod agents;
mod agents_overview;
mod ambient;
mod app_context;
mod approvals;
// NOTE: #[allow(dead_code)] required to avoid rustc 1.93.1 ICE in check_mod_deathness.
#[allow(dead_code)]
mod apps;
mod bridge;
/// Who is actually on the socket, as far as the kernel and `/proc` can say. See issue #43.
mod caller_identity;
mod companion_rpc;
mod control;
mod control_approvals;
mod control_installer;
mod control_update;
mod control_files;
/// Agents' commands on the shell's surface: agent_run / agent_job / agent_input / agent_kill.
mod control_agent_terminal;
/// A recipe on the shell's surface: answer_recipe / pause_recipe / resume_recipe / cancel_recipe.
mod control_recipes;
/// Agents on the shell's surface: new_agent / send_to_agent / stop_agent / read_agent / show_agent.
mod control_agents;
mod jobs;
mod cards;
mod clipboard;
// NOTE: #[allow(dead_code)] required to avoid rustc 1.93.1 ICE in check_mod_deathness.
// Remove once rustc is updated past the fix.
#[allow(dead_code)]
mod features;
mod filebrowser;
mod fileops;
// What minds this machine could have, before any of them is running.
mod harness_catalogue;
mod harness_install;
mod config_store;
mod models;
mod focus;
mod frecency;
mod i18n;
#[allow(dead_code)]
mod mime_dispatch;
mod lens;
/// The Lens's conversation put back from the saved session when it opens empty (#246).
mod lens_history;
mod lock;
mod markdown;
/// What the mind may do without being asked: plan / ask / auto / bypass. See its module doc.
mod mind_mode;
/// The right edge of every screen: the answering mind, what is at work, what it did.
mod mind_panel;
/// A desktop of the mind's own, inside one window: where the apps a mind opens are drawn (#239).
mod mind_view;
mod notifications;
mod onboarding;
mod perception;
mod icons;
mod render_backend;
/// Every recipe the companion holds, as the worker last published them: the Recipes screen,
/// `describe shell` and the mind panel read this and never wait on the worker.
mod recipes;
mod running;
mod streaming;
mod surfaces;
mod system_context;
mod telegram;
mod terminal;
mod trail;
// What protects the credential vault on this machine, and the honest answer when nothing does.
mod vault_unlock;
mod voice;
// NOTE: #[allow(dead_code)] required to avoid rustc 1.93.1 ICE in check_mod_deathness.
#[allow(dead_code)]
mod windows;
mod wire;

// Slint-generated types live in yantrik-ui-slint (separate crate so that
// Rust-only changes here don't trigger Slint recompilation).
pub use yantrik_ui_slint::*;

fn main() {
    // Before anything else, because `--version` has to work on a machine whose shell will not
    // start. The first argument is otherwise a config path, so this also stops `--version`
    // being read as the name of a config file that does not exist.
    yantrik_version::handle_version_flag("yantrik-ui");

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // The shell does not go through the runtime's `init_tracing`, so it installs the same
    // panic hook itself: a panic here becomes a problem record before it becomes a stack trace.
    yantrik_app_runtime::problems::install_panic_hook("yantrik-ui");

    // Load config
    let config_path = std::env::args().nth(1).map(PathBuf::from);
    let config = load_config(config_path.clone());

    // Propagate OAuth credentials from config to env vars (if not already set)
    if let Some(ref id) = config.connectors.google_client_id {
        if std::env::var("GOOGLE_CLIENT_ID").is_err() {
            std::env::set_var("GOOGLE_CLIENT_ID", id);
        }
    }
    if let Some(ref secret) = config.connectors.google_client_secret {
        if std::env::var("GOOGLE_CLIENT_SECRET").is_err() {
            std::env::set_var("GOOGLE_CLIENT_SECRET", secret);
        }
    }

    // Pick the renderer before Slint reads SLINT_BACKEND — it only looks once, at App::new().
    // Getting this wrong is not a small penalty: femtovg on a machine with no GPU falls through
    // to llvmpipe and burns ~6 cores, against ~1 for the software rasteriser. See render_backend.
    let renderer = render_backend::select();

    // Create Slint UI
    let ui = App::new().unwrap();

    // Ambient decoration is drawn at the renderer's budget: 60fps on a GPU where frames are
    // nearly free, ~10fps on the software rasteriser where each one is main-thread time. The
    // .slint files scale their phase increments by this, so the motion keeps its speed and only
    // loses smoothness. Measured: drawing this at 60fps on the CPU saturated a core by itself.
    ui.set_ambient_interval_ms(renderer.ambient_interval_ms());
    tracing::info!(
        renderer = ?renderer,
        ambient_interval_ms = renderer.ambient_interval_ms(),
        "Ambient animation budget set"
    );

    // Start background services.
    //
    // Before the companion rather than after it, which is new. `AppContext::init` below starts
    // the companion on its own worker thread, and one of the first things that thread does is
    // move the calendar events the companion's old tools kept to themselves into
    // calendar-service. That needs a way to start an on-demand service, and this process is the
    // only one that has one — so the manager and the hook come first. Nothing in `init` or in
    // `wire_all` depends on the services being down.
    let service_manager = start_services();

    // Anything in this process that needs an on-demand service can now start it directly.
    //
    // An app is a separate process and asks the shell over `app.act start_service`, which is
    // dispatched onto this thread. The companion is not: it runs on a worker thread here, and
    // that round trip would leave the process only to come back in and queue behind whatever the
    // compositor is doing. The manager is the same one the rail reads, so a service started this
    // way is never described as stopped.
    {
        let starter = service_manager.clone();
        // `start` reaps a child that has exited before it checks for Running (#58), so a
        // service that died is started again rather than reported as up.
        yantrik_ipc_transport::service::set_local_starter(move |id| starter.start(id));
    }

    // Initialize all shared state
    let ctx = app_context::AppContext::init(config, &ui, config_path);

    // Read perception-service into the companion's memory, through the reader's gate: the OS's
    // own processes, low salience and anything over the rate ceiling stay out. The service is
    // started on demand, so most of the time this parks on a retry and costs nothing (#58).
    perception::spawn(ctx.bridge.clone());

    // Wire all callbacks
    wire::wire_all(&ui, &ctx);

    // The machine rail lists the services; it needs the manager.
    wire::services::wire(&ui, service_manager.clone());

    // The boot screen's stages, read from the same manager the machine rail reads.
    wire::boot::wire(&ui, &ctx, service_manager.clone());

    // Publish the companion so the apps under apps/ can use it. Without this their AI actions
    // are stubs: the model, the memory and the bond all live in this process.
    companion_rpc::serve(ctx.bridge.handle());

    // Adopt the windows that were already open.
    //
    // The shell learns of a window by starting it, and that knowledge lives in a HashMap in this
    // process — so it is empty every time this process starts. Restart the shell while the
    // compositor keeps running (a crash, an update, `systemctl restart`) and the apps stay on
    // screen while the shell believes nothing is open: `describe shell` answered "0 windows
    // open" with four windows in front of the person reading it, and every dock tile was dark.
    //
    // The compositor is the one thing in the session that outlived us, so it is asked once here,
    // before anything publishes anything. After this the taskbar refresh keeps it current on its
    // own cadence; this call is only about the first answer being right rather than the fourth.
    let adopted = windows::refresh_compositor_windows();
    tracing::info!(adopted, "Windows already open when the shell started");

    // And publish the desktop itself, the same way every app does. Without it, "what is on my
    // desktop right now" was answerable only by photographing a status bar we wrote ourselves.
    control::publish(&ui, &ctx, service_manager.clone());

    // Debug: navigate to specific screen on startup via env var
    if let Ok(screen_str) = std::env::var("YANTRIK_START_SCREEN") {
        if let Ok(screen) = screen_str.parse::<i32>() {
            tracing::info!(screen, "Debug: navigating to startup screen");
            ui.set_current_screen(screen);
            ui.invoke_navigate(screen);
        }
    }

    // Give the shell's shortcut scope the keyboard.
    //
    // Slint delivers a key press to the focused element and walks up from there; with nothing
    // focused there is no chain and the event is dropped before any handler — capture included —
    // is consulted. A shell that has just started and is sitting on the desktop has nothing
    // focused, so the first Ctrl+K after boot would have gone nowhere. See `focus-global-keys`
    // in app.slint.
    ui.invoke_focus_global_keys();

    // The desktop is left by logging out, never by a request to close its window. To the
    // compositor this shell is one ordinary fullscreen window, and labwc's Alt+F4 closes whichever
    // window has focus: after a click on the desktop, the taskbar or the status bar, that is this
    // one. Answered with the default, the window hid, this loop ended and the session went with
    // it — Alt+F4 meant for an app logged the person out (#229). A logout takes the compositor
    // away instead, which ends the loop without asking.
    ui.window().on_close_requested(refuse_close);

    // Run
    tracing::info!("Starting Yantrik OS desktop shell");
    // A logout ends this loop by taking the compositor away, which winit returns as an error. That
    // is an ending, not a crash: the helper logs it and returns, so the shutdown below runs on
    // every ending. Unwrapped, every logout filed a crash record and orphaned agents' commands (#196).
    yantrik_app_runtime::run_until_closed(&ui, "yantrik-ui");

    // Clean shutdown
    tracing::info!("Yantrik OS shutting down");
    service_manager.stop_all();
    // Agents' commands belong to the shell and go with it: every process group, not one pid.
    control_agent_terminal::shutdown();
}

/// What the shell answers a request to close its window: no. Logged, so a close that was refused
/// can be told from one that never arrived.
fn refuse_close() -> slint::CloseRequestResponse {
    tracing::info!("Refused a request to close the desktop; it closes by logging out (#229)");
    slint::CloseRequestResponse::KeepWindowShown
}

/// Start background services via the ServiceManager.
/// Discovers services from manifests in the services directory, falling back
/// to hardcoded registrations for built-in services.
fn start_services() -> yantrik_shell_core::service_manager::ServiceManager {
    use yantrik_shell_core::service_manager::ServiceManager;

    // Services binary dir: same directory as the main yantrik-ui binary
    let bin_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let mgr = ServiceManager::new(bin_dir.clone());

    // Try manifest-based discovery first (installed services have yantrik.toml)
    let services_dir = bin_dir.join("services");
    if services_dir.is_dir() {
        mgr.scan_and_register(&services_dir);
        tracing::info!(path = %services_dir.display(), "Scanned service manifests");
    }

    // Register built-in services as fallback (for dev builds without manifest dirs)
    mgr.register("weather", "weather-service", true);
    mgr.register("system-monitor", "system-monitor-service", true);
    mgr.register("notes", "notes-service", false);
    mgr.register("notifications", "notifications-service", true);
    mgr.register("calendar", "calendar-service", false);
    mgr.register("network", "network-service", true);
    mgr.register("email", "email-service", false);
    // Reads windows we did not write. Autostarted: it costs nothing when there is no
    // accessibility bus, and connects lazily if one appears later.
    mgr.register("a11y", "a11y-service", true);
    // The kernel's periphery. Not autostarted, because it wants CAP_NET_ADMIN and CAP_SYS_ADMIN
    // to open its descriptors and a desktop session cannot grant either: started from here it
    // comes up on PSI alone, which is worth having on request and not worth running all session
    // for nobody. It is still startable through `start_service`, and that is what reaches it —
    // `yos perception`, and so os_perception, asks for it on the first request. Without that the
    // machine rail's "on demand" was a caption on a process nothing anywhere ever ran.
    mgr.register("perception", "perception-service", false);

    // Start autostart services (best-effort — binary may not exist in dev)
    mgr.start_autostart();

    mgr
}

fn load_config(path: Option<PathBuf>) -> CompanionConfig {
    match path {
        Some(p) => {
            tracing::info!(path = %p.display(), "Loading config");
            CompanionConfig::from_yaml(&p).expect("failed to load config")
        }
        None => {
            tracing::info!("Using default config");
            CompanionConfig::default()
        }
    }
}

#[cfg(test)]
mod close_tests {
    /// Alt+F4 with the shell focused ended the session (#229). The desktop refuses a close, and
    /// the refusal is in place before the loop a close request would end starts running.
    #[test]
    fn the_desktop_refuses_to_be_closed_from_before_it_runs() {
        assert!(matches!(super::refuse_close(), slint::CloseRequestResponse::KeepWindowShown));
        // Split, so this test's own text is not what the search finds.
        let installed = concat!("ui.window().on_close_", "requested(refuse_close);");
        let runs = concat!("yantrik_app_runtime::run_until_", "closed(&ui");
        let source = include_str!("main.rs");
        let installed = source.find(installed).expect("main installs the refusal");
        let runs = source.find(runs).expect("main runs the loop");
        assert!(installed < runs, "the refusal is installed before the loop starts");
    }
}
