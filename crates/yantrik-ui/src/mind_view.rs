//! Mind View (#239): a desktop of the mind's own, inside one window on the person's.
//!
//! A mind opening Files, the Editor or the Browser used to put that window on the person's
//! desktop, over whatever they were doing — and over the Lens and its approval card (#205, #218).
//! With several agents at work the desktop stopped being the person's at all (#230).
//!
//! Mind View is a second labwc, run nested: on the wlroots Wayland backend it is an ordinary
//! window on the person's desktop, with a display of its own inside it. An app a mind opens is
//! started with `WAYLAND_DISPLAY` (and `DISPLAY`, for the nested labwc's own Xwayland) pointing
//! there, so it draws inside that window. Nothing about the app changes: its control surface is
//! a socket in `$XDG_RUNTIME_DIR/yantrik`, not the screen, and a mind drives it the same way
//! wherever it is drawn. Approval cards are the shell's own, so they stay on the person's desktop
//! in front of everything, including Mind View.
//!
//! # Who counts as a mind
//!
//! Decided in [`requester_now`], while the call that asked for the launch is still on the UI
//! thread: `invoke_launch_app` is synchronous, so the kernel's word on the caller
//! (`SO_PEERCRED`) and any agent token are still in scope when the dock's spawn runs. A click
//! has no caller at all. Everything else is sorted by [`classify`].
//!
//! # When Mind View cannot start
//!
//! If the nested compositor cannot be started — no labwc, no config, or it does not say which
//! display it is serving within [`START_BUDGET`] — the reason is logged and published in
//! `describe shell`, and Mind View is tried again after `RETRY_AFTER`. The mind's launch
//! then fails with that reason. It used to open the app on the person's desktop instead, which
//! put a mind's work over theirs in exactly the case nobody was watching (2 Oct 2026); a refused
//! launch that says why is better than a window in the wrong place.

use std::collections::HashSet;

use yantrik_companion::tools::browser::MindDisplay;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The environment that titles Mind View's window: the title library preloaded into its labwc,
/// when it ships beside the shell. Without it the window keeps labwc's own title; nothing else
/// changes.
fn title_preload() -> Vec<(&'static str, String)> {
    let lib = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(TITLE_LIBRARY)))
        .filter(|p| p.is_file());
    match lib {
        Some(lib) => vec![
            ("LD_PRELOAD", lib.display().to_string()),
            ("YANTRIK_MIND_VIEW_TITLE", TITLE.to_string()),
        ],
        None => Vec::new(),
    }
}

/// The title library's file name, beside the shell in bin/.
const TITLE_LIBRARY: &str = "libyantrik_mind_view_title.so";

/// How long the nested compositor gets to say which display it is serving.
///
/// It is paid off the UI thread (the launch waits on a worker, see `wire::dock::spawn_launch`),
/// and only the first time: labwc on the software renderer answers in well under a second.
const START_BUDGET: Duration = Duration::from_secs(5);

/// How long a failed start is believed before Mind View is started again.
const RETRY_AFTER: Duration = Duration::from_secs(30);

/// The startup script the nested labwc runs, in the session's socket directory. Only a Mind View
/// runs it, which is how one an earlier shell left is found.
const SEAT_SCRIPT: &str = "mind-view-seat.sh";

/// Where the nested labwc's configuration is installed, and where it is in a checkout.
const INSTALLED_CONFIG: &str = "/opt/yantrik/share/labwc-mind";
const CHECKOUT_CONFIG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/labwc-mind");

/// Whether a window on the person's desktop is the one a nested compositor draws into, which is
/// Mind View. The window list then reads it back as Mind View.
///
/// The names are not ours to choose. wlroots gives the window the app_id `wlroots` and the title
/// `wlroots - WL-1`, which is what the spike saw on labwc 0.7.1. labwc 0.8 retitles it
/// `labwc - WL-1`, and on the VM that miss left the taskbar showing a black window by that name.
/// So either program's name is accepted, as the app_id or as the start of such a title.
///
/// The title counts only when the window declared no app_id. Any page or terminal can set its
/// own title to `labwc - WL-1`, and a window on the person's desktop must not pass as contained.
pub fn is_nested_window(declared_id: &str, title: &str) -> bool {
    const COMPOSITORS: [&str; 2] = ["wlroots", "labwc"];
    if !declared_id.is_empty() {
        return COMPOSITORS.iter().any(|c| declared_id.eq_ignore_ascii_case(c));
    }
    title
        .split_once(" - ")
        .is_some_and(|(name, output)| COMPOSITORS.contains(&name) && output.starts_with("WL-"))
}

/// The title Mind View's window carries on the person's desktop. The one place the words are
/// spelled for code that finds the window by title: the title library is handed it, and
/// rc.xml's Super+Shift+V binding is tested against it.
pub const TITLE: &str = "Mind View";

/// The shell's own id for the Mind View window, as the window list and `show_app` spell it.
pub const APP_ID: &str = "mind-view";

/// The display Mind View serves, as the nested labwc itself reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seat {
    /// `wayland-N`, relative to `$XDG_RUNTIME_DIR`.
    pub wayland: String,
    /// The nested labwc's Xwayland, `:N`, or `None` when it has none.
    pub x11: Option<String>,
}

impl Seat {
    /// The environment a process needs to draw on this display rather than the person's.
    ///
    /// Also pins the toolkits' backend choice to what Mind View offers: without an Xwayland of its
    /// own, GTK and Qt are told Wayland only, so a dead Wayland socket makes an app fail instead
    /// of falling back to the person's X display (PR #582 review, B1).
    pub fn env(&self) -> Vec<(&'static str, String)> {
        let mut env = vec![("WAYLAND_DISPLAY", self.wayland.clone())];
        match &self.x11 {
            Some(x11) => {
                env.push(("DISPLAY", x11.clone()));
                env.push(("GDK_BACKEND", "wayland,x11".to_string()));
                env.push(("QT_QPA_PLATFORM", "wayland;xcb".to_string()));
            }
            None => {
                env.push(("GDK_BACKEND", "wayland".to_string()));
                env.push(("QT_QPA_PLATFORM", "wayland".to_string()));
            }
        }
        env
    }

    /// What every launch for a mind does to its environment: everything that names the person's
    /// session cleared first, then this display set. The one place that decides it; the launcher,
    /// the agent terminal's viewer and the companion's browser all apply it.
    pub fn display(&self) -> MindDisplay {
        MindDisplay::mind_view(self.env())
    }
}

/// Who a launch is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Requester {
    /// A pointer on the desktop, or a program the person runs themselves.
    Person,
    /// A mind, named as far as the shell could tell.
    Mind(String),
}

/// What the kernel and the agent host said about the caller, reduced to what [`classify`] needs.
#[derive(Clone, Debug, Default)]
pub struct CallerFacts {
    /// `SO_PEERCRED`'s pid, or `None` when there was no socket call at all (a click).
    pub pid: Option<u32>,
    /// `Some(true)` for an agent token the shell believed, `Some(false)` for one it did not,
    /// `None` when no token came.
    pub agent: Option<bool>,
    /// The attached mind this caller's ancestry belongs to, if any.
    pub attached_mind: Option<String>,
    /// Whether the kernel says the caller is the mind account (#411): a caller that came in
    /// through the mind door. A mind by what it is, before anything it says or any /proc walk.
    pub mind_account: bool,
}

/// Sort a caller into the person or a mind.
///
/// - No caller: a click, a key or a Files double-click — the person.
/// - A token, believed or not: an agent. One that presented a token and was not believed is
///   still not the person (`control_agent_terminal::calling_agent` holds the same line).
/// - This process: the built-in companion, which calls the shell's own socket from inside it.
/// - A process descended from an attached mind (Hermes, pi through `yos-mcp`).
/// - Anything else — `yos` typed in the Terminal, labwc's Ctrl+Alt+T — is the person.
pub fn classify(facts: &CallerFacts, own_pid: u32) -> Requester {
    // First: the kernel's word on the account outranks everything, a missing pid included.
    if facts.mind_account {
        return Requester::Mind(facts.attached_mind.clone().unwrap_or_else(|| "a mind".to_string()));
    }
    let Some(pid) = facts.pid else { return Requester::Person };
    match facts.agent {
        Some(true) => return Requester::Mind("an agent".to_string()),
        Some(false) => return Requester::Mind("an agent whose token was not believed".to_string()),
        None => {}
    }
    if pid == own_pid {
        return Requester::Mind("the companion".to_string());
    }
    match &facts.attached_mind {
        Some(name) => Requester::Mind(name.clone()),
        None => Requester::Person,
    }
}

/// Who the call being dispatched on this thread is for. Read on the UI thread, inside the
/// handler that asked for the launch — nowhere else has the caller in scope.
pub fn requester_now() -> Requester {
    let caller = yantrik_app_runtime::control::caller();
    let pid = caller.as_ref().and_then(|c| u32::try_from(c.pid).ok()).filter(|p| *p > 0);
    let agent = crate::control_agent_terminal::calling_agent().map(|r| r.is_ok());
    // The /proc walk is only worth doing when nothing cheaper has decided it.
    let attached_mind = match (pid, agent) {
        (Some(pid), None) if pid != std::process::id() => {
            crate::caller_identity::resolve(pid as i32).attached_mind
        }
        _ => None,
    };
    let mind_account = caller.as_ref().is_some_and(|c| yantrik_ipc_transport::mind_door::is_mind(c.uid));
    classify(&CallerFacts { pid, agent, attached_mind, mind_account }, std::process::id())
}

/// Where one launch goes, decided while the call that asked for it is still on this thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    /// `Some(who)` to open it in Mind View for that mind; `None` for the person's desktop.
    pub mind_view: Option<String>,
    /// Whether a second copy handing over to a window already open may bring that window to the
    /// front. Never for a mind working in its own view: raising the person's window because a mind
    /// asked for the app is the interruption Mind View exists to end.
    pub raise_on_handover: bool,
    /// Whether to start a process at all. `false` for a mind that asks for an app already open on
    /// the person's desktop: a second process there would be a second window on their desk, so
    /// nothing is started and the answer says so (PR #582 review, B2).
    pub spawn: bool,
}

/// Route the launch the current call asked for.
pub fn route_now(app_id: &str) -> Route {
    let desktop = Route { mind_view: None, raise_on_handover: true, spawn: true };
    if !crate::wire::settings::minds_open_in_mind_view() {
        return desktop;
    }
    let who = match requester_now() {
        Requester::Person => return desktop,
        Requester::Mind(who) => who,
    };
    route(
        who,
        crate::running::is_running(app_id) && !is_mind_view_app(app_id),
    )
}

/// [`route_now`] once the facts are in: a mind asked, and whether the app is already open on the
/// person's desktop. A Mind View that failed this session no longer sends the app to the desktop:
/// a mind's app never opens on the person's desk, so the launch fails and says why.
fn route(who: String, open_on_desktop: bool) -> Route {
    if open_on_desktop {
        // Not started again, anywhere on the person's desktop. Only the shell's own apps are
        // single-instance; a catalogue app, `foot` or a browser would open a second window on
        // their desk at a mind's request. The mind drives the window that is open through its
        // surface, and is told it was neither launched nor raised.
        return Route { mind_view: None, raise_on_handover: false, spawn: false };
    }
    // Also when Mind View is down: the launch then fails where it started and says why
    // (`wire::dock::spawn_launch`), rather than opening the app over the person's work.
    Route { mind_view: Some(who), raise_on_handover: false, spawn: true }
}

// ── The nested compositor ────────────────────────────────────────────

struct Nested {
    child: Child,
    seat: Seat,
}

#[derive(Default)]
struct State {
    nested: Option<Nested>,
    /// Why Mind View could not be started, and when. Tried again after [`RETRY_AFTER`], so one slow
    /// start on a loaded machine does not refuse every mind's app until the shell restarts.
    unavailable: Option<(String, Instant)>,
    /// The apps launched into Mind View, by pid, with the id they were launched as.
    apps: Vec<(u32, String)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut guard = STATE.lock().unwrap_or_else(|p| p.into_inner());
    f(guard.get_or_insert_with(State::default))
}

/// Why Mind View is not available this session, if it is not.
pub fn unavailable() -> Option<String> {
    with_state(|s| s.unavailable.as_ref().map(|(why, _)| why.clone()))
}

/// Held for the whole of a start, so two launches in the same instant start one compositor.
/// Separate from [`STATE`], which the UI thread reads for `describe`: a start that takes seconds
/// must never hold the lock the UI thread waits on.
static STARTING: Mutex<()> = Mutex::new(());

/// The display Mind View is serving, starting it first if it is not running.
///
/// Blocks for up to [`START_BUDGET`] the first time. Never call it on the UI thread.
pub fn ensure() -> Result<Seat, String> {
    let _starting = STARTING.lock().unwrap_or_else(|p| p.into_inner());
    let current = with_state(|state| {
        if let Some((why, since)) = &state.unavailable {
            if since.elapsed() < RETRY_AFTER {
                return Some(Err(why.clone()));
            }
            state.unavailable = None;
        }
        let nested = state.nested.as_mut()?;
        let alive = matches!(nested.child.try_wait(), Ok(None));
        if alive && socket_path(&nested.seat.wayland).exists() {
            return Some(Ok(nested.seat.clone()));
        }
        // The person closed Mind View, or it died. A fresh one is started below; the apps that
        // were in it went with its display.
        tracing::info!("Mind View is gone; starting it again for the next app");
        let _ = nested.child.kill();
        let _ = nested.child.wait();
        state.nested = None;
        state.apps.clear();
        None
    });
    if let Some(answer) = current {
        return answer;
    }
    match start() {
        Ok(nested) => {
            let seat = nested.seat.clone();
            tracing::info!(wayland = %seat.wayland, x11 = ?seat.x11, "Mind View started");
            with_state(|state| state.nested = Some(nested));
            Ok(seat)
        }
        Err(why) => {
            tracing::warn!(reason = %why, "Mind View could not start; minds' apps are refused until it can");
            with_state(|state| state.unavailable = Some((why.clone(), Instant::now())));
            Err(why)
        }
    }
}

/// Stops every Mind View an earlier shell left running. A shell that stops, whether the
/// supervisor restarts it or an update does, never stops the nested labwc it started, so each
/// restart used to leave one on the person's desktop: a `labwc - WL-1` window that no shell
/// tracks or can reach. VM 520 had two. Called once as the shell starts, and again before a Mind
/// View is started.
pub fn stop_left_behind() {
    let script = yantrik_ipc_transport::server::socket_dir().join(SEAT_SCRIPT);
    for pid in left_behind(Path::new("/proc"), &script, std::process::id()) {
        tracing::info!(pid, "stopping a Mind View an earlier shell left running");
        #[cfg(unix)]
        // SAFETY: kill(2) on a pid read from /proc; a pid that has gone since is ESRCH, no harm.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }
}

/// The labwcs running this session's Mind View startup script, which only a Mind View runs, from
/// a `/proc`-shaped directory, leaving out `own` (this shell). Only asked while this shell holds
/// no Mind View of its own: as it starts, and before it starts one.
fn left_behind(proc_dir: &Path, script: &Path, own: u32) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir(proc_dir) else { return Vec::new() };
    let script = script.as_os_str().as_encoded_bytes();
    let mut found: Vec<u32> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            let cmdline = std::fs::read(entry.path().join("cmdline")).ok()?;
            let args: Vec<&[u8]> = cmdline.split(|b| *b == 0).collect();
            let labwc = args.first().is_some_and(|a| a.ends_with(b"labwc"));
            let runs_script = args.windows(2).any(|w| w[0] == b"-s" && w[1] == script);
            (labwc && runs_script && pid != own).then_some(pid)
        })
        .collect();
    found.sort_unstable();
    found
}

fn socket_path(wayland: &str) -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_default();
    runtime.join(wayland)
}

fn config_dir() -> Option<PathBuf> {
    [INSTALLED_CONFIG, CHECKOUT_CONFIG]
        .iter()
        .map(PathBuf::from)
        .find(|dir| dir.join("rc.xml").is_file())
}

/// The file the nested labwc's startup command writes its display into.
fn seat_file(dir: &Path) -> PathBuf {
    dir.join("mind-view.seat")
}

/// The startup command: labwc runs it once it is serving, with its own `WAYLAND_DISPLAY` and
/// `DISPLAY` in the environment — the only place those names can be read from, since labwc picks
/// the first free `wayland-N` itself. Written to a file rather than passed as `sh -c '…'` because
/// labwc splits `-s` into words itself, and quoting through that is not something to get wrong.
///
/// It then does two things for how Mind View looks, both skipped where the tool is missing:
/// - sizes it to [`SIZE_PERCENT`] of the person's screen, by setting the nested output's mode,
///   which is what the window on their desktop follows. At wlroots' default 1280×720 it filled a
///   small screen, and read as the desktop having gone black rather than as a window on it;
/// - puts [`EMPTY_HINT`] behind the windows. A nested labwc draws nothing of its own, so an empty
///   Mind View was a black rectangle that said nothing about what it was.
///
/// `outer` is the person's display, which the script cannot otherwise see: its own
/// `WAYLAND_DISPLAY` is the nested one.
fn seat_script(seat_file: &Path, config: &Path, outer: &str) -> String {
    const SCRIPT: &str = r#"#!/bin/sh
# Written by yantrik-ui for Mind View (#239): say which display this labwc is serving.
printf '%s\n%s\n' "$WAYLAND_DISPLAY" "${DISPLAY:-}" > '@SEAT@.tmp' && mv '@SEAT@.tmp' '@SEAT@'
# A window on the person's desktop, not all of it: a share of their screen, in logical pixels.
if command -v wlr-randr >/dev/null; then
    size=$(WAYLAND_DISPLAY='@OUTER@' wlr-randr | awk -v pct=@PCT@ '
        /\(.*current/ && !mode { split($1, m, "x"); mode = 1 }
        /Scale:/ && !scale { scale = $2 }
        END { if (mode) { if (scale <= 0) scale = 1; printf "%dx%d", m[1] * pct / 100 / scale, m[2] * pct / 100 / scale } }')
    output=$(wlr-randr | awk 'NR == 1 { print $1 }')
    [ -n "$size" ] && [ -n "$output" ] && wlr-randr --output "$output" --custom-mode "$size"
fi >/dev/null 2>&1 &
# And say what it is while nothing is drawn in it.
command -v swaybg >/dev/null && swaybg -m center -c '@BG@' -i '@HINT@' >/dev/null 2>&1 &
"#;
    SCRIPT
        .replace("@SEAT@", &seat_file.display().to_string())
        .replace("@OUTER@", outer)
        .replace("@PCT@", &SIZE_PERCENT.to_string())
        .replace("@BG@", EMPTY_BACKGROUND)
        .replace("@HINT@", &config.join(EMPTY_HINT).display().to_string())
}

/// How much of the person's screen Mind View takes when it opens, in each direction. Enough for
/// an app to be usable in, small enough to read as one window among theirs; the title bar's
/// maximise button gives it the whole screen.
const SIZE_PERCENT: u32 = 70;

/// The picture behind an empty Mind View, in its configuration directory: its name and one line
/// on what it is for.
const EMPTY_HINT: &str = "empty.png";

/// What the rest of the window is filled with around it: the desktop's inactive title bar colour
/// (`config/labwc/themerc`), which is also the picture's own background.
const EMPTY_BACKGROUND: &str = "#0c0c14";

/// Read what the startup command wrote: the Wayland display, then the X display or nothing.
fn parse_seat(text: &str) -> Option<Seat> {
    let mut lines = text.lines().map(str::trim);
    let wayland = lines.next().filter(|w| !w.is_empty() && !w.contains('/'))?.to_string();
    let x11 = lines.next().filter(|x| !x.is_empty()).map(str::to_string);
    Some(Seat { wayland, x11 })
}

/// Mind View's window, as the person's compositor names it: labwc's app id, and the title the
/// title library gives it (crates/yantrik-mind-view-title).
const WINDOW_MATCH: [&str; 2] = ["app_id:labwc", "title:Mind View"];

/// Minimise Mind View's window the moment it first appears (#427).
///
/// A window that maps is focused and raised by the person's compositor like any other, so the
/// first app a mind opened put Mind View over whatever the person was doing and took their
/// keyboard: over the launcher they had just opened, in the case that found it. That is the
/// interruption Mind View exists to end. It starts out of the way; the person opens it from its
/// taskbar entry when they want to watch, and focus returns to what they had in front. Off the UI
/// thread: it waits for the window, and wlrctl is a process.
fn step_aside_when_shown() {
    std::thread::spawn(|| {
        let deadline = Instant::now() + START_BUDGET;
        while Instant::now() < deadline {
            let shown = Command::new("wlrctl")
                .args(["toplevel", "find"])
                .args(WINDOW_MATCH)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if shown {
                let minimised = Command::new("wlrctl")
                    .args(["toplevel", "minimize"])
                    .args(WINDOW_MATCH)
                    .status()
                    .is_ok_and(|s| s.success());
                if !minimised {
                    tracing::warn!("Mind View appeared but could not be set aside; it may be over the person's work");
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        tracing::debug!("Mind View's window did not appear in time to be set aside");
    });
}

fn start() -> Result<Nested, String> {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Err("the shell is not running under a Wayland compositor, so there is nothing \
                    to put a Mind View window on"
            .to_string());
    }
    let labwc = crate::wire::dock::find_program("labwc")
        .ok_or_else(|| "labwc is not installed, so Mind View cannot run".to_string())?;
    let config = config_dir().ok_or_else(|| {
        format!("Mind View's labwc configuration is missing (looked in {INSTALLED_CONFIG})")
    })?;

    let dir = yantrik_ipc_transport::server::socket_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let seat_file = seat_file(&dir);
    let _ = std::fs::remove_file(&seat_file);
    // One this shell does not hold is one an earlier shell left: its apps are out of reach, and
    // it would stay on the desktop beside the new one.
    stop_left_behind();
    let script = dir.join(SEAT_SCRIPT);
    let outer = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
    std::fs::write(&script, seat_script(&seat_file, &config, &outer))
        .map_err(|e| format!("{}: {e}", script.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("{}: {e}", script.display()))?;
    }

    let mut child = Command::new(&labwc)
        .arg("-C")
        .arg(&config)
        .arg("-s")
        .arg(&script)
        // A window on the person's compositor, not a session of its own on the hardware.
        .env("WLR_BACKENDS", "wayland")
        // Titled "Mind View" on the person's desktop, not labwc's own "labwc - WL-1": the
        // title library answers labwc's wlr_wl_output_set_title (crates/yantrik-mind-view-title).
        .envs(title_preload())
        // Its Xwayland sets DISPLAY for what it starts; the person's must not leak in.
        .env_remove("DISPLAY")
        .env_remove("SLINT_FULLSCREEN")
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", labwc.display()))?;

    let deadline = Instant::now() + START_BUDGET;
    loop {
        if let Some(seat) = std::fs::read_to_string(&seat_file).ok().as_deref().and_then(parse_seat)
        {
            if socket_path(&seat.wayland).exists() {
                step_aside_when_shown();
                return Ok(Nested { child, seat });
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("Mind View's labwc exited as it started ({status})"));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "Mind View's labwc did not say which display it was serving within {}s",
                START_BUDGET.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Where a window a mind opens outside the launcher is drawn: Mind View's display, with every
/// variable naming the person's session cleared (started if need be), or an error. Mind View
/// being down is an error, not a fall back to the person's display; only the person's own
/// setting (`minds open apps in Mind View` off) sends a mind's window to the desktop. Handed to
/// the companion's browser tools at startup.
///
/// Can wait for Mind View to start, so never on the UI thread; the companion's tools run on its
/// own worker.
pub fn display_for_mind() -> Result<MindDisplay, String> {
    if !crate::wire::settings::minds_open_in_mind_view() {
        let person = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".to_string());
        return Ok(MindDisplay::person(person));
    }
    ensure()
        .map(|seat| seat.display())
        .map_err(|why| format!("Mind View is not available ({why}); a mind's window is not opened on the person's desktop"))
}

// ── What is in it ────────────────────────────────────────────────────

/// Record that `pid`, launched as `app_id`, is drawing in Mind View.
pub fn mark_launched(app_id: &str, pid: u32) {
    with_state(|s| {
        s.apps.retain(|(p, _)| *p != pid);
        s.apps.push((pid, app_id.to_string()));
    });
}

/// The one app drawing in Mind View that `want` names, by its id or the name it goes by. None when
/// none does, or more than one does.
///
/// `close_window` reaches for this when nothing on the person's desktop answers: an app a mind
/// opened is drawn here, not there, so "Close the Notes app" was refused with "no open window
/// matches `Notes`" while Notes was open, and the mind told the person it was not (yantrik-mind,
/// VM 520, 2026-09-27).
pub fn app_named(want: &str) -> Option<String> {
    let want = want.trim().to_lowercase();
    if want.is_empty() {
        return None;
    }
    let apps: Vec<String> = with_state(|s| s.apps.iter().map(|(_, id)| id.clone()).collect());
    let mut found: Vec<String> = apps
        .into_iter()
        .filter(|id| {
            let name = crate::windows::app_display_name(id).to_lowercase();
            id.to_lowercase() == want || name == want || name.contains(&want)
        })
        .collect();
    found.sort();
    found.dedup();
    (found.len() == 1).then(|| found.remove(0))
}

/// Ask an app drawing in Mind View to close, as its × does: on Mind View's own display, so an app
/// with unsaved work can still put up its dialog. Returns the name it was asked by.
pub fn close_app(app_id: &str) -> Result<String, String> {
    let display = with_state(|s| s.nested.as_ref().map(|n| n.seat.wayland.clone()))
        .ok_or_else(|| "Mind View is not running, so nothing is drawn there to close".to_string())?;
    let title = crate::windows::app_display_name(app_id);
    let args = ["toplevel".to_string(), "close".to_string(), format!("title:{title}")];
    let (answer, wait) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("yos-wlrctl-mind-view".to_string())
        .spawn(move || {
            let _ = answer.send(Command::new("wlrctl").args(&args).env("WAYLAND_DISPLAY", &display).status());
        })
        .map_err(|_| "could not start a thread to talk to Mind View's compositor".to_string())?;
    match wait.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(status)) if status.success() => Ok(title),
        Ok(Ok(_)) => Err(format!("Mind View's compositor has no window called `{title}` to close")),
        Ok(Err(e)) => Err(format!("could not run wlrctl: {e}")),
        Err(_) => Err("Mind View's compositor did not answer within 2 s".to_string()),
    }
}

/// Forget an app that has exited.
pub fn mark_exited(pid: u32) {
    with_state(|s| s.apps.retain(|(p, _)| *p != pid));
}

/// The pids of the apps drawing in Mind View — not windows on the person's desktop, so not
/// theirs to be listed in the taskbar or switched to.
pub fn app_pids() -> HashSet<u32> {
    with_state(|s| s.apps.iter().map(|(p, _)| *p).collect())
}

/// What is drawing in Mind View, as a person reads the apps' names, in the order they were opened.
pub fn app_names() -> Vec<String> {
    let ids: Vec<String> = with_state(|s| s.apps.iter().map(|(_, id)| id.clone()).collect());
    let mut names: Vec<String> = Vec::new();
    for id in ids {
        let name = crate::windows::app_display_name(&id);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

pub(crate) fn is_mind_view_app(app_id: &str) -> bool {
    with_state(|s| s.apps.iter().any(|(_, id)| id == app_id))
}

/// The windows Mind View's own compositor lists, one `wlrctl toplevel list` line each, or `None`
/// when Mind View is not running or its compositor did not answer in 2 s. A process, so never on
/// the UI thread.
pub fn nested_window_lines() -> Option<Vec<String>> {
    let display = display_now()?;
    let (answer, wait) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("yos-wlrctl-mind-view-list".to_string())
        .spawn(move || {
            let _ = answer.send(
                Command::new("wlrctl")
                    .args(["toplevel", "list"])
                    .env("WAYLAND_DISPLAY", &display)
                    .stderr(Stdio::null())
                    .output(),
            );
        })
        .ok()?;
    match wait.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(out)) if out.status.success() => {
            Some(String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect())
        }
        _ => None,
    }
}

/// Whether one of Mind View's window lines (`app_id: title`, as `wlrctl toplevel list` prints
/// them) is the app `id`. The line is read the way the desktop's own window list reads it
/// (`windows::toplevel_entry`): the declared app id when the window has one, else our own
/// windows' exact title. A third-party window also answers to `aliases` (the program it ran),
/// compared with its declared app id only. Never a word found inside a title: a title can say
/// anything, and the agent terminal's viewer is titled `<agent> terminal`, which made `terminal`
/// look open after any `agent_run` (PR #582 review, S1).
pub fn lists_app(lines: &[String], id: &str, aliases: &[&str]) -> bool {
    let id = id.to_lowercase();
    lines.iter().any(|line| {
        let window = crate::windows::toplevel_entry(line);
        if is_agent_viewer(&window.wayland_app_id.to_lowercase(), &window.title) {
            return false;
        }
        if window.app_id == id {
            return true;
        }
        let declared = window.wayland_app_id.to_lowercase();
        !declared.is_empty()
            && aliases.iter().any(|a| {
                let a = a.to_lowercase();
                !a.is_empty() && (declared == a || declared == a.rsplit('/').next().unwrap_or(&a))
            })
    })
}

/// Whether a window is one the agent terminal's viewer opened (`foot`, titled `<agent> terminal`),
/// which is not an app a mind asked for.
fn is_agent_viewer(app_id: &str, title: &str) -> bool {
    app_id == "foot" && title.trim_end().ends_with(" terminal")
}

/// The Wayland display Mind View draws on, while it is running.
pub fn display_now() -> Option<String> {
    with_state(|s| {
        let n = s.nested.as_mut()?;
        matches!(n.child.try_wait(), Ok(None)).then(|| n.seat.wayland.clone())
    })
}

/// What `describe shell` says about Mind View.
pub fn for_describe() -> serde_json::Value {
    let on = crate::wire::settings::minds_open_in_mind_view();
    with_state(|s| {
        let running = s
            .nested
            .as_mut()
            .map(|n| matches!(n.child.try_wait(), Ok(None)))
            .unwrap_or(false);
        let mut apps: Vec<&str> = s.apps.iter().map(|(_, id)| id.as_str()).collect();
        apps.sort_unstable();
        apps.dedup();
        serde_json::json!({
            "minds_open_apps_here": on,
            // Where a mind's windows are, in words a mind can repeat to the person.
            "your_windows_are": if on {
                "inside Mind View (one window on the person's screen), not as windows of their own"
            } else {
                "on the person's desktop, because `minds open apps in Mind View` is off"
            },
            "agent_terminal": if on {
                "commands from agent_run are shown live in a terminal in Mind View"
            } else {
                "commands from agent_run are not drawn anywhere; their output is in the answer and on the Agents screen"
            },
            "running": running,
            "display": s.nested.as_ref().filter(|_| running).map(|n| n.seat.wayland.clone()),
            "apps": apps,
            "unavailable": s.unavailable.as_ref().map(|(why, _)| why),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHELL: u32 = 4242;

    fn facts(pid: Option<u32>, agent: Option<bool>, mind: Option<&str>) -> CallerFacts {
        CallerFacts { pid, agent, attached_mind: mind.map(str::to_string), mind_account: false }
    }

    #[test]
    fn a_caller_the_kernel_says_is_the_mind_account_is_a_mind_whatever_else_is_true() {
        let own = 4242;
        let door = CallerFacts { pid: Some(777), agent: None, attached_mind: None, mind_account: true };
        assert_eq!(classify(&door, own), Requester::Mind("a mind".to_string()), "no token, no ancestry: still a mind");
        let named = CallerFacts { attached_mind: Some("Pi".to_string()), ..door };
        assert_eq!(classify(&named, own), Requester::Mind("Pi".to_string()));
    }

    #[test]
    fn a_mind_view_an_earlier_shell_left_is_found_by_its_script_and_nothing_else_is() {
        let proc_dir = std::env::temp_dir().join(format!("mv-left-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&proc_dir);
        let script = Path::new("/run/user/1000/yantrik/mind-view-seat.sh");
        let process = |pid: u32, args: &[&str]| {
            let dir = proc_dir.join(pid.to_string());
            std::fs::create_dir_all(&dir).unwrap();
            let mut cmdline = args.join("\0");
            cmdline.push('\0');
            std::fs::write(dir.join("cmdline"), cmdline).unwrap();
        };
        let left = ["/usr/bin/labwc", "-C", "/opt/yantrik/share/labwc-mind", "-s", "/run/user/1000/yantrik/mind-view-seat.sh"];
        process(256173, &left);
        process(1021934, &left);
        // The person's own compositor runs the shell, not the script.
        process(713, &["labwc", "-s", "/opt/yantrik/bin/yantrik-ui /opt/yantrik/config.yaml"]);
        // Another user's Mind View runs a script in their own runtime dir.
        process(900, &["/usr/bin/labwc", "-C", "x", "-s", "/run/user/1001/yantrik/mind-view-seat.sh"]);
        // Something that only names the script is not a labwc.
        process(901, &["cat", "-s", "/run/user/1000/yantrik/mind-view-seat.sh"]);
        std::fs::create_dir_all(proc_dir.join("self")).unwrap();

        assert_eq!(left_behind(&proc_dir, script, SHELL), vec![256173, 1021934]);
        assert_eq!(left_behind(&proc_dir, script, 256173), vec![1021934], "never this shell itself");
        assert!(left_behind(&proc_dir.join("absent"), script, SHELL).is_empty(), "no /proc, nothing to stop");
        let _ = std::fs::remove_dir_all(&proc_dir);
    }

    #[test]
    fn an_app_a_mind_opened_is_found_by_its_name_and_only_when_one_answers() {
        mark_launched("notes", 990_001);
        assert_eq!(app_named("Notes").as_deref(), Some("notes"), "by the name it goes by");
        assert_eq!(app_named("notes").as_deref(), Some("notes"), "by its id");
        assert_eq!(app_named("  NOTES "), Some("notes".to_string()));
        assert_eq!(app_named("Calendar"), None, "not drawn here");
        assert_eq!(app_named(""), None);
        mark_launched("notes", 990_002);
        assert_eq!(app_named("Notes").as_deref(), Some("notes"), "two of one app are still one app");
        mark_exited(990_001);
        mark_exited(990_002);
        assert_eq!(app_named("Notes"), None, "gone once it exits");
    }

    #[test]
    fn a_click_is_the_person() {
        assert_eq!(classify(&facts(None, None, None), SHELL), Requester::Person);
    }

    #[test]
    fn a_person_typing_yos_is_the_person() {
        // `yos act shell open_app` from the Terminal, or labwc's Ctrl+Alt+T binding.
        assert_eq!(classify(&facts(Some(900), None, None), SHELL), Requester::Person);
    }

    #[test]
    fn the_companion_calling_its_own_shell_is_a_mind() {
        assert_eq!(
            classify(&facts(Some(SHELL), None, None), SHELL),
            Requester::Mind("the companion".into())
        );
    }

    #[test]
    fn an_attached_mind_is_named() {
        assert_eq!(
            classify(&facts(Some(900), None, Some("Hermes")), SHELL),
            Requester::Mind("Hermes".into())
        );
    }

    #[test]
    fn a_token_makes_an_agent_believed_or_not() {
        assert!(matches!(classify(&facts(Some(900), Some(true), None), SHELL), Requester::Mind(_)));
        // Presenting a token that is not believed never falls back to "the person".
        assert!(matches!(
            classify(&facts(Some(900), Some(false), None), SHELL),
            Requester::Mind(_)
        ));
    }

    #[test]
    fn a_minds_app_goes_to_mind_view_and_never_falls_back_to_the_desktop() {
        let who = || "Hermes".to_string();
        assert_eq!(route(who(), false), Route { mind_view: Some(who()), raise_on_handover: false, spawn: true });
        // Already open on the person's desktop: used where it is, and not raised over their work.
        assert_eq!(route(who(), true), Route { mind_view: None, raise_on_handover: false, spawn: false });
    }

    #[test]
    fn the_seat_is_read_as_labwc_wrote_it() {
        assert_eq!(
            parse_seat("wayland-1\n:2\n"),
            Some(Seat { wayland: "wayland-1".into(), x11: Some(":2".into()) })
        );
        // No Xwayland in the nested labwc: X apps are then not redirected at all.
        assert_eq!(parse_seat("wayland-1\n\n"), Some(Seat { wayland: "wayland-1".into(), x11: None }));
        assert_eq!(parse_seat(""), None);
        assert_eq!(parse_seat("\n:2\n"), None);
        // A path is not a display name; nothing outside the runtime directory is a seat.
        assert_eq!(parse_seat("../wayland-0\n"), None);
    }

    #[test]
    fn an_app_in_mind_view_gets_both_displays() {
        let seat = Seat { wayland: "wayland-1".into(), x11: Some(":2".into()) };
        let env = seat.env();
        assert!(env.contains(&("WAYLAND_DISPLAY", "wayland-1".to_string())));
        assert!(env.contains(&("DISPLAY", ":2".to_string())));
    }

    fn env_of(command: &Command, key: &str) -> Option<Option<String>> {
        command.get_envs().find(|(k, _)| *k == key).map(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
    }

    /// B1: with no Xwayland of its own, Mind View must not leave the person's `DISPLAY` (or their
    /// socket, or their X authority) in the environment of what a mind starts, and the toolkits
    /// are told Wayland only so a dead socket fails instead of falling back to their X display.
    #[test]
    fn a_minds_app_never_inherits_the_persons_display() {
        let seat = Seat { wayland: "wayland-1".into(), x11: None };
        let mut command = Command::new("true");
        // What the shell itself holds, and `session_env` puts back.
        command.env("DISPLAY", ":0").env("WAYLAND_DISPLAY", "wayland-0").env("WAYLAND_SOCKET", "3").env("XAUTHORITY", "/home/p/.Xauthority");
        command.env("GDK_BACKEND", "x11").env("QT_QPA_PLATFORM", "xcb");
        seat.display().apply(&mut command);
        assert_eq!(env_of(&command, "DISPLAY"), Some(None), "DISPLAY must be cleared, not inherited");
        assert_eq!(env_of(&command, "WAYLAND_SOCKET"), Some(None));
        assert_eq!(env_of(&command, "XAUTHORITY"), Some(None));
        assert_eq!(env_of(&command, "WAYLAND_DISPLAY"), Some(Some("wayland-1".to_string())));
        assert_eq!(env_of(&command, "GDK_BACKEND"), Some(Some("wayland".to_string())));
        assert_eq!(env_of(&command, "QT_QPA_PLATFORM"), Some(Some("wayland".to_string())));
    }

    #[test]
    fn with_an_xwayland_only_mind_views_own_display_is_set() {
        let seat = Seat { wayland: "wayland-1".into(), x11: Some(":7".into()) };
        let mut command = Command::new("true");
        command.env("DISPLAY", ":0");
        seat.display().apply(&mut command);
        assert_eq!(env_of(&command, "DISPLAY"), Some(Some(":7".to_string())));
        assert_eq!(env_of(&command, "WAYLAND_DISPLAY"), Some(Some("wayland-1".to_string())));
        // Whatever it is set to, it is never one of the person's displays.
        assert!(!seat.env().iter().any(|(_, v)| v == ":0" || v == "wayland-0"));
    }

    /// Every variable the companion's browser tools clear is cleared by the shell's launcher too:
    /// one list.
    #[test]
    fn the_shells_launches_clear_what_the_browser_tools_clear() {
        let seat = Seat { wayland: "wayland-1".into(), x11: None };
        assert_eq!(seat.display().clear, yantrik_companion::tools::browser::PERSON_SESSION_VARS.to_vec());
    }

    /// B3: Mind View down is an error for a mind's window, not the person's display.
    #[test]
    fn a_window_for_a_mind_is_refused_not_sent_to_the_person_when_mind_view_is_down() {
        let src = include_str!("mind_view.rs");
        let f = &src[src.find("pub fn display_for_mind()").unwrap()..];
        let f = &f[..f.find("\n}\n").unwrap()];
        assert!(f.contains("map_err"), "an unavailable Mind View must be an Err: {f}");
        assert!(!f.contains("unwrap_or_else(|_| \"wayland-0\"") || f.contains("minds_open_in_mind_view"),
            "the person's display is chosen only by the person's setting");
    }

    // ── Which window is the app (S1) ──

    fn lines(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    /// The agent terminal's viewer is a `foot` titled `<agent> terminal`: it is not the Terminal
    /// app, and not `foot` the app either.
    #[test]
    fn the_agent_viewer_is_not_the_terminal_app_or_foot() {
        let l = lines(&["foot: hermes terminal"]);
        assert!(!lists_app(&l, "terminal", &["foot"]));
        assert!(!lists_app(&l, "foot", &["foot"]));
    }

    /// Our own windows declare no app id, so they are known by their exact title.
    #[test]
    fn our_own_windows_are_found_by_their_title_and_not_by_words_in_one() {
        assert!(lists_app(&lines(&[": Terminal"]), "terminal", &[]));
        assert!(!lists_app(&lines(&["firefox: Files you may like - Mozilla Firefox"]), "files", &[]));
        assert!(!lists_app(&lines(&["foot: notes.txt - editor"]), "notes", &[]));
    }

    /// A third-party window is found by the app id it declares, which is not the shell's id for it.
    #[test]
    fn a_third_party_window_is_found_by_the_program_it_ran() {
        let l = lines(&["chromium: New Tab - Chromium", "Blender: Blender 4.2"]);
        assert!(lists_app(&l, "browser", &["/usr/bin/chromium"]));
        assert!(lists_app(&l, "blender", &["blender"]), "compared without case");
        assert!(!lists_app(&l, "browser", &["firefox"]));
        assert!(!lists_app(&l, "browser", &[""]), "an empty alias matches nothing");
    }

    #[test]
    fn the_describe_text_does_not_contradict_itself() {
        let src = include_str!("mind_view.rs");
        assert!(!src.contains("never on their desk\""), "N2");
        assert!(!src.contains("open on the desktop\");"), "N1");
    }

    /// S6: one failed start does not refuse every mind's app for the rest of the session.
    #[test]
    fn a_failed_start_is_tried_again_after_a_while() {
        let src = include_str!("mind_view.rs");
        assert!(src.contains("RETRY_AFTER"));
        assert!(RETRY_AFTER <= Duration::from_secs(60));
    }

    /// The startup command is run by labwc, and what it writes is what [`parse_seat`] reads.
    #[cfg(unix)]
    #[test]
    fn the_seat_script_writes_what_the_shell_reads() {
        let dir = std::env::temp_dir().join(format!("mind-view-seat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = seat_file(&dir);
        let script = dir.join("seat.sh");
        std::fs::write(&script, seat_script(&file, Path::new(CHECKOUT_CONFIG), "wayland-0")).unwrap();
        let status = Command::new("sh")
            .arg(&script)
            .env("WAYLAND_DISPLAY", "wayland-7")
            .env("DISPLAY", ":3")
            .status()
            .unwrap();
        assert!(status.success());
        let seat = parse_seat(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(seat, Seat { wayland: "wayland-7".into(), x11: Some(":3".into()) });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_nested_labwc_has_a_configuration_in_the_checkout() {
        assert!(
            Path::new(CHECKOUT_CONFIG).join("rc.xml").is_file(),
            "config/labwc-mind/rc.xml is what Mind View's labwc runs with"
        );
    }

    #[test]
    fn the_empty_hint_ships_beside_the_configuration() {
        assert!(
            Path::new(CHECKOUT_CONFIG).join(EMPTY_HINT).is_file(),
            "config/labwc-mind/empty.png is what an empty Mind View shows"
        );
    }

    /// labwc 0.7.1 left wlroots' names on the window; 0.8.3 retitles it after itself.
    #[test]
    fn the_nested_window_is_known_by_either_compositor_name() {
        assert!(is_nested_window("wlroots", "wlroots - WL-1"));
        assert!(is_nested_window("labwc", "labwc - WL-1"));
        assert!(is_nested_window("wlroots", "labwc - WL-1"));
        assert!(is_nested_window("", "labwc - WL-2"));
        assert!(!is_nested_window("", "labwc - notes.txt"));
        assert!(!is_nested_window("", "wlroots - WLAN setup"));
        assert!(!is_nested_window("firefox", "Mozilla Firefox"));
    }

    /// The mind path's environment is the nested display's and nothing of the person's: what the
    /// dock's `launch` overlays after the session's own.
    #[test]
    fn the_mind_path_launches_with_mind_views_display_and_never_the_persons() {
        let seat = Seat { wayland: "wayland-1".into(), x11: Some(":2".into()) };
        let env = seat.env();
        assert!(env.contains(&("WAYLAND_DISPLAY", "wayland-1".to_string())));
        assert!(env.contains(&("DISPLAY", ":2".to_string())));
        assert!(!env.iter().any(|(_, v)| v == "wayland-0" || v == ":0"));
        // The source of the launch path: a mind's launch whose Mind View is down is refused, not
        // handed to `launch` with no seat (the person's desktop).
        let dock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/wire/dock.rs")).unwrap();
        let body = &dock[dock.find("fn spawn_launch(").unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        let worker = &body[body.find("std::thread::spawn").unwrap()..];
        assert!(
            !worker.contains("None, true)") && !worker.contains("adapter, None"),
            "a mind's launch must never fall back to the person's display:\n{worker}"
        );
        assert!(worker.contains("Some(&seat)"), "the worker launches on Mind View's seat");
    }
}
