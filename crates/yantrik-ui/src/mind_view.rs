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
//! # Failing safe
//!
//! If the nested compositor cannot be started — no labwc, no config, or it does not say which
//! display it is serving within [`START_BUDGET`] — the app opens on the person's desktop as it
//! did before this existed, the reason is logged and published in `describe shell`, and Mind View
//! is not tried again until the shell restarts. A broken Mind View must never cost a mind its
//! apps, and must never cost the person the same wait twice.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long the nested compositor gets to say which display it is serving.
///
/// It is paid off the UI thread (the launch waits on a worker, see `wire::dock::spawn_launch`),
/// and only the first time: labwc on the software renderer answers in well under a second.
const START_BUDGET: Duration = Duration::from_secs(5);

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
pub fn is_nested_window(declared_id: &str, title: &str) -> bool {
    const COMPOSITORS: [&str; 2] = ["wlroots", "labwc"];
    if COMPOSITORS.iter().any(|c| declared_id.eq_ignore_ascii_case(c)) {
        return true;
    }
    title
        .split_once(" - ")
        .is_some_and(|(name, output)| COMPOSITORS.contains(&name) && output.starts_with("WL-"))
}

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
    pub fn env(&self) -> Vec<(&'static str, String)> {
        let mut env = vec![("WAYLAND_DISPLAY", self.wayland.clone())];
        if let Some(x11) = &self.x11 {
            env.push(("DISPLAY", x11.clone()));
        }
        env
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
    classify(&CallerFacts { pid, agent, attached_mind }, std::process::id())
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
}

/// Route the launch the current call asked for.
pub fn route_now(app_id: &str) -> Route {
    let desktop = Route { mind_view: None, raise_on_handover: true };
    if !crate::wire::settings::minds_open_in_mind_view() {
        return desktop;
    }
    let who = match requester_now() {
        Requester::Person => return desktop,
        Requester::Mind(who) => who,
    };
    route(
        who,
        unavailable().is_some(),
        crate::running::is_running(app_id) && !is_mind_view_app(app_id),
    )
}

/// [`route_now`] once the facts are in: a mind asked, whether Mind View has already failed this
/// session, and whether the app is already open on the person's desktop.
fn route(who: String, unavailable: bool, open_on_desktop: bool) -> Route {
    if unavailable {
        // As before Mind View existed, so a broken one costs nothing but itself.
        return Route { mind_view: None, raise_on_handover: true };
    }
    if open_on_desktop {
        // Our apps are single-instance: a second copy in Mind View would hand over to the window
        // already open and exit. The mind drives that window through its surface; it is not
        // brought over the person's work to do it.
        return Route { mind_view: None, raise_on_handover: false };
    }
    Route { mind_view: Some(who), raise_on_handover: false }
}

// ── The nested compositor ────────────────────────────────────────────

struct Nested {
    child: Child,
    seat: Seat,
}

#[derive(Default)]
struct State {
    nested: Option<Nested>,
    /// Why Mind View could not be started, once it could not. Cleared only by a restart.
    unavailable: Option<String>,
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
    with_state(|s| s.unavailable.clone())
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
        if let Some(why) = &state.unavailable {
            return Some(Err(why.clone()));
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
            tracing::warn!(reason = %why, "Mind View could not start; minds' apps open on the desktop");
            with_state(|state| state.unavailable = Some(why.clone()));
            Err(why)
        }
    }
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
    let script = dir.join("mind-view-seat.sh");
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

/// Where a window a mind opens outside the launcher is drawn, as the environment to start it
/// with: Mind View's display when minds' apps go there and it is up (started if need be), the
/// person's own otherwise. Handed to the companion's browser tools at startup, which used to write
/// the person's display into every launch and so bypassed Mind View entirely.
///
/// Can wait for Mind View to start, so never on the UI thread; the companion's tools run on its
/// own worker.
pub fn display_for_mind() -> Vec<(&'static str, String)> {
    if crate::wire::settings::minds_open_in_mind_view() {
        if let Ok(seat) = ensure() {
            return seat.env();
        }
    }
    let person = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".to_string());
    vec![("WAYLAND_DISPLAY", person)]
}

// ── What is in it ────────────────────────────────────────────────────

/// Record that `pid`, launched as `app_id`, is drawing in Mind View.
pub fn mark_launched(app_id: &str, pid: u32) {
    with_state(|s| {
        s.apps.retain(|(p, _)| *p != pid);
        s.apps.push((pid, app_id.to_string()));
    });
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

fn is_mind_view_app(app_id: &str) -> bool {
    with_state(|s| s.apps.iter().any(|(_, id)| id == app_id))
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
            "running": running,
            "display": s.nested.as_ref().filter(|_| running).map(|n| n.seat.wayland.clone()),
            "apps": apps,
            "unavailable": s.unavailable,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHELL: u32 = 4242;

    fn facts(pid: Option<u32>, agent: Option<bool>, mind: Option<&str>) -> CallerFacts {
        CallerFacts { pid, agent, attached_mind: mind.map(str::to_string) }
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
    fn a_minds_app_goes_to_mind_view_unless_it_cannot() {
        let who = || "Hermes".to_string();
        assert_eq!(route(who(), false, false), Route { mind_view: Some(who()), raise_on_handover: false });
        // Already open on the person's desktop: used where it is, and not raised over their work.
        assert_eq!(route(who(), false, true), Route { mind_view: None, raise_on_handover: false });
        // Mind View failed this session: exactly what happened before it existed.
        assert_eq!(route(who(), true, false), Route { mind_view: None, raise_on_handover: true });
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
        assert_eq!(
            seat.env(),
            vec![("WAYLAND_DISPLAY", "wayland-1".to_string()), ("DISPLAY", ":2".to_string())]
        );
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
}
