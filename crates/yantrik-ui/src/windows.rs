//! Windows discovered by the compositor, merged with the shell launch registry.
//! Compositor discovery is cached so surviving windows remain available after a
//! shell restart without spawning a helper on every taskbar refresh.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The title the shell's own window carries, from `title:` in yantrik-ui-slint/ui/app.slint.
///
/// Kept here so the one place that has to exclude it says why, rather than a bare string buried
/// in a filter. `pub(crate)` because `control_approvals` needs the same string for the opposite
/// reason — it asks the compositor to bring THIS window forward when an approval card goes up,
/// and has to recognise it to avoid recording the shell as the window to hand the screen back to.
pub(crate) const SHELL_WINDOW_TITLE: &str = "Yantrik OS";

/// A running window on the desktop.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowEntry {
    pub title: String,
    /// The shell's own id for the app — `blender`, `browser`, `notes` — which the dock keys its
    /// running marks by and `show_app` accepts. Lowercase, always.
    pub app_id: String,
    /// The app_id the window declared to the compositor, spelled exactly as `wlrctl toplevel
    /// list` printed it, or empty for a window that declared none (every Slint window of ours).
    ///
    /// Kept apart from `app_id` because the two disagree in case and the compositor cares:
    /// `wlrctl toplevel find app_id:Blender` matches Blender's window and `app_id:blender` does
    /// not. This is the one string that names a foreign window to wlrctl whatever its title has
    /// changed to since the list was read.
    pub wayland_app_id: String,
    pub icon_char: String,
    pub subtitle: String,
}

/// How long one reading of the compositor's window list is trusted.
///
/// Nine seconds, which is three turns of the system poll. Long enough that the taskbar refresh
/// never spawns a process on its own cadence, short enough that a window closed by hand leaves
/// the strip while the person is still looking at it.
const COMPOSITOR_TTL: Duration = Duration::from_secs(9);

/// The last reading of the compositor: its window list, which of them it said was in front, and
/// when it was taken.
///
/// Process-wide, because every surface that asks "what is open" has to get the same answer, and
/// because the one caller that must never spawn a subprocess — `shell_windows`, which runs inside
/// the `describe` closure on the UI thread — reads it without refreshing it.
///
/// The front window rides along with the list because the taskbar draws its active underline on
/// the FIRST entry and `wire::timers` reads that entry as the foreground window. Neither was ever
/// told which window labwc actually has in front — `toplevel list` carries no focus flag — so the
/// underline sat under whichever app the merge happened to sort first (#76).
static COMPOSITOR: Mutex<Option<(Instant, Vec<WindowEntry>, Option<String>)>> = Mutex::new(None);

/// Read the compositor now — its window list, and the window it says is in front — and keep both.
/// Returns how many windows it found.
///
/// Called once at startup (see `main`) and then by the taskbar refresh. The startup call is the
/// point of this whole mechanism: see `shell_windows` below.
///
/// A reading that cannot say what is in front (see [`front_now`]) keeps the last window one that
/// did: the taskbar's promise is "most recently focused", and that survives the person clicking
/// back to the desktop, when the only activated toplevel is the shell's own. The stale name
/// costs nothing when its window is gone — the merge simply finds no entry to move.
pub fn refresh_compositor_windows() -> usize {
    let found = wlrctl_windows();
    let count = found.len();
    let front = front_now();
    if let Ok(mut cache) = COMPOSITOR.lock() {
        let front = front.or_else(|| cache.as_ref().and_then(|(_, _, f)| f.clone()));
        *cache = Some((Instant::now(), found, front));
    }
    count
}

/// The last reading, without taking a new one. Empty list until something has refreshed it.
fn compositor_snapshot() -> (Vec<WindowEntry>, Option<String>) {
    COMPOSITOR
        .lock()
        .ok()
        .and_then(|c| c.as_ref().map(|(_, w, f)| (w.clone(), f.clone())))
        .unwrap_or_default()
}

/// Take a new reading if the one we have has aged out.
fn refresh_compositor_if_stale() {
    let stale = match COMPOSITOR.lock() {
        Ok(cache) => cache.as_ref().is_none_or(|(at, _, _)| at.elapsed() >= COMPOSITOR_TTL),
        Err(_) => return,
    };
    if stale {
        refresh_compositor_windows();
    }
}

/// The windows the shell has open, after taking a fresh reading of the compositor.
///
/// For the callers that are about to put the list in front of somebody — the window switcher
/// opening on a hotkey — where a list up to nine seconds stale is a list with a window in it that
/// has just been closed.
pub fn list_windows() -> Vec<WindowEntry> {
    refresh_compositor_windows();
    shell_windows()
}

/// The same list, for callers on a timer: the compositor is only asked again once the last answer
/// has aged out. The taskbar refresh runs every three seconds and must not spawn a process each
/// time it does.
pub fn list_windows_throttled() -> Vec<WindowEntry> {
    refresh_compositor_if_stale();
    shell_windows()
}

/// The launch registry, plus everything the compositor saw that the registry does not know about.
///
/// A window is the same window if the id matches, not only if the title does. The registry names
/// a window by its app id; the compositor gives back whatever the window is actually called at
/// this moment. For our own apps those agree — `app_names_agree_everywhere` holds every app's
/// `title:` to its APP_NAMES entry — but a foreign app puts what it likes in its title bar, and
/// matching on the title would list the same window twice: once as the shell remembers launching
/// it and once as the compositor sees it.
///
/// When both have the window, the compositor's account of it is the one listed. The registry
/// used to win, and it titled the window `display_name(id)`: so Blender, launched by the shell,
/// was listed as "Blender" while the compositor had it as `(Unsaved) - Blender 4.3.2`. That
/// title went into `describe shell`, a caller passed it back to `minimise_window`, and
/// `wlrctl toplevel minimize title:Blender` matched nothing — and the real title, the only
/// string wlrctl would have taken, was refused by the validator because the list did not show
/// it. No value worked for that window. The registry still says the app is open before the
/// compositor's next reading has the window; it just stops naming a window it cannot see.
///
/// A launched app pairs with a compositor window by the shell's id, or by the binary the shell
/// started: `open_app browser` runs `chromium`, and the window comes back as app_id `chromium`,
/// so on id alone the merge saw two applications and listed a phantom "Browser" beside the real
/// Chromium window, twice in the taskbar. Our own apps are single-instance (see
/// `running::mark_launched`), so one id is one window.
///
/// `front` is the window the compositor said is activated, from the same reading as `discovered`.
/// It comes first in the merged list, which is the whole of what the taskbar underlines by: the
/// bar draws its active marker on the first entry, and until #76 nothing ordered the list by
/// focus at all, so the marker sat on whatever the registry's alphabetical sort put first —
/// Calendar in every screenshot of one person's working morning — while other windows took
/// turns being the one in front.
fn merge_windows(
    launched: &[crate::running::RunningApp],
    mut discovered: Vec<WindowEntry>,
    front: Option<&str>,
) -> Vec<WindowEntry> {
    let mut merged: Vec<WindowEntry> = launched
        .iter()
        .map(|app| {
            let app_id = app.app_id.clone();
            let seen = discovered
                .iter()
                .position(|w| w.app_id == app_id || same_program(&app.binary, &w.wayland_app_id))
                .map(|i| discovered.remove(i));
            match seen {
                Some(window) => WindowEntry {
                    subtitle: derive_context(&window.title, &app_id),
                    icon_char: icon_for_app(&app_id).to_string(),
                    title: window.title,
                    wayland_app_id: window.wayland_app_id,
                    app_id,
                },
                None => WindowEntry {
                    title: display_name(&app_id),
                    icon_char: icon_for_app(&app_id).to_string(),
                    subtitle: String::new(),
                    wayland_app_id: String::new(),
                    app_id,
                },
            }
        })
        .collect();
    merged.append(&mut discovered);
    put_front_first(&mut merged, front);
    merged
}

/// Move the window titled `front` to index 0, leaving the rest in order. Does nothing when
/// there is no answer, or when the named window is not on the list — it closed since the
/// reading, and the compositor's answer is then no one's focus.
fn put_front_first(merged: &mut Vec<WindowEntry>, front: Option<&str>) {
    let Some(i) = front.and_then(|f| merged.iter().position(|w| w.title == f)) else { return };
    if i != 0 {
        let window = merged.remove(i);
        merged.insert(0, window);
    }
}

/// Whether a window that declared `wayland_app_id` came from the binary the shell started.
///
/// A program's app_id is its binary's name, or the name with a distribution suffix on one side:
/// Debian's `chromium` is `chromium`, Ubuntu's is `chromium-browser`, `google-chrome-stable`
/// declares `google-chrome`. So the two are the same program when they are equal, or when one is
/// the other up to a `-`. The binary is compared by its file name, because the shell launches
/// Blender by the full path `find_program` resolved.
fn same_program(binary: &str, wayland_app_id: &str) -> bool {
    let bin = std::path::Path::new(binary)
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let id = wayland_app_id.to_lowercase();
    if bin.is_empty() || id.is_empty() {
        return false;
    }
    let extends = |longer: &str, shorter: &str| {
        longer.strip_prefix(shorter).is_some_and(|rest| rest.starts_with('-'))
    };
    bin == id || extends(&bin, &id) || extends(&id, &bin)
}

/// The windows the shell has open: what it launched, plus what the compositor says is on screen.
///
/// This is what `describe shell` reports and what the dock reads its running marks from, so it
/// has to be cheap — it runs on the UI thread inside the describe closure — and correct on every
/// screen, because an app stays open when the shell navigates away from the desktop. Cheap is why
/// it reads the cached compositor snapshot rather than taking one: no subprocess on this thread.
///
/// It used to be the launch registry ALONE, on the reasoning that the shell started its own
/// children and therefore knew them better than any query could. True, and it misses the case
/// that matters: the registry is an in-process `HashMap`, so it is empty every time this process
/// starts. Restart the shell under a compositor that keeps running — which is what a crash, an
/// update or `systemctl restart` does — and four apps are still on screen while `describe shell`
/// says "0 windows open" and the dock shows none of them running. The shell only ever learned of
/// a window by watching itself create it, and it had not watched these.
///
/// The compositor did watch them, and it is the only thing in the session that outlives us. So it
/// is asked, and what it says is merged in. The old objection to asking it — that the title
/// heuristic collapsed every Yantrik window onto one id — is answered by `app_id_for_title`:
/// our windows carry titles that are exactly the `APP_NAMES` entries, an invariant
/// `app_names_agree_everywhere` already enforces against the .desktop files and the app sources,
/// so the id comes from a lookup rather than a guess. `wlrctl` being absent costs us only what it
/// cost before: the registry answer, which is what this returned in the first place.
///
/// The window the last reading found activated is listed first — see `merge_windows`.
///
/// An app drawing in Mind View (#239) is not one of them: it is on the mind's display, inside the
/// one Mind View window, and a taskbar entry for it would be a button that switches to nothing.
/// The Mind View window itself is listed, as the compositor sees it.
pub fn shell_windows() -> Vec<WindowEntry> {
    let (discovered, front) = compositor_snapshot();
    let in_mind_view = crate::mind_view::app_pids();
    let mut launched = crate::running::running();
    launched.retain(|app| !in_mind_view.contains(&app.pid));
    merge_windows(&launched, discovered, front.as_deref())
}

/// The name one of our app ids goes by on screen.
///
/// These are not free-form labels: they are the exact `title:` each app's window declares in
/// `apps/<app>/ui/app.slint`, because this same string is what the taskbar hands to
/// `wlrctl toplevel focus title:…` when the entry is clicked. Five of them used to be the app's
/// short name instead — `Downloads` for a window called "Download Manager", `Music` for "Music
/// Player" — so clicking those entries matched no window and did nothing at all, silently.
///
/// If you rename a window, rename it here. There is a test below that lists both.
/// What each app this OS ships is CALLED. One list, because there were four.
///
/// The same application answered to a different name depending on which surface you were
/// looking at: the dock said "Editor", the launcher said "Text Editor", the window title said
/// "Text Editor" and the header said whatever the screen author wrote. Downloads was
/// "Download Manager" in three places and "Downloads" in a fourth. No single one of those was
/// wrong, which is exactly why it survived — it only reads as sloppy when you see two at once,
/// and you always do: the taskbar entry sits directly beneath the window it names.
///
/// Short names, because that is the family the dock already used and the dock is the surface a
/// person reads most. The suffixes went rather than being invented away: Container Manager to
/// Containers, Music Player to Music, Image Viewer to Images. The office three keep the brand
/// the dock gave them.
///
/// Keys are the shell's own app ids on the left, matching `Icons.app`, and the .desktop file
/// stems for the rest. `app_names_agree_everywhere` in the tests below reads the .desktop files
/// and each app's Window title and fails if any of them drifts from this.
pub const APP_NAMES: &[(&str, &str)] = &[
    ("arcade", "Arcade"),
    ("browser", "Browser"),
    ("calendar", "Calendar"),
    ("containers", "Containers"),
    ("documents", "yDoc"),
    ("downloads", "Downloads"),
    ("editor", "Editor"),
    ("email", "Email"),
    ("image", "Images"),
    ("music", "Music"),
    ("network", "Network"),
    ("notes", "Notes"),
    ("presentation", "yPresent"),
    ("snippets", "Snippets"),
    ("spreadsheet", "ySheets"),
    ("studio", "Studio"),
    ("sysmonitor", "System Monitor"),
    ("terminal", "Terminal"),
    ("weather", "Weather"),
];

fn display_name(app_id: &str) -> String {
    APP_NAMES
        .iter()
        .find(|(id, _)| *id == app_id)
        .map(|(_, name)| (*name).to_string())
        .unwrap_or_else(|| {
            // Unknown id (a .desktop app the shell launched): title-case its first segment.
            let mut c = app_id.replace(['-', '_'], " ");
            if let Some(first) = c.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            c
        })
}

// ── Asking the compositor to move a window ──────────────────────────
//
// Everything below builds a `wlrctl toplevel …` command line and runs it. The shell owns none of
// this: labwc decides what is in front, what is minimized and what closes, and the only thing we
// can do is ask. So every function here reports whether the asking worked rather than assuming it.

/// How long a caller waits for `wlrctl` before answering without it.
///
/// Action handlers run on the UI thread — that is the only thread allowed to touch a Slint window
/// — and `wlrctl` is a process, so a wait here is a frozen desktop for as long as it lasts. On the
/// test machine `wlrctl toplevel list` answers in about two milliseconds; 1.2 seconds is far
/// outside that and still well inside the three seconds the control surface gives an action.
const COMPOSITOR_REPLY_LIMIT: Duration = Duration::from_millis(1200);

/// The `wlrctl toplevel <verb>` command line for one matchspec, such as `title:Notes`.
///
/// One argument each, so a title with a space travels as a title with a space. There is no shell
/// between us and wlrctl and there must not be a quoting scheme pretending there is.
fn toplevel_command(verb: &str, matchspec: &str) -> Vec<String> {
    vec!["toplevel".to_string(), verb.to_string(), matchspec.to_string()]
}

/// The `wlrctl toplevel <verb>` command line for one window, named by title.
///
/// Two things about wlrctl's matchspec, both learned the hard way and both load-bearing:
///
/// The key is spelled out, because a bare word "is assumed to be an app_id". Our Slint windows
/// declare a title and no wayland app_id at all, and a match on nothing exits as SUCCESS — which
/// is how every click on a taskbar entry used to do nothing at all, silently.
///
/// The title has to be the window's title EXACTLY. `title:` is an exact, case-sensitive
/// comparison in wlrctl 0.2.2: `wlrctl toplevel find title:editor` misses a window called
/// `Editor`, and `title:Yantrik` misses `Yantrik OS`. So every caller resolves what a person
/// typed against the open windows first (see [`window_named`]) and passes the title it found,
/// never the one it was given.
fn toplevel_args(verb: &str, title: &str) -> Vec<String> {
    toplevel_command(verb, &format!("title:{title}"))
}

/// The command line that brings a MINIMIZED window back onto the screen — the fallback for when
/// the foreign-toplevel protocol cannot be used (see [`crate::foreign_toplevel`]).
///
/// A minimized window cannot take focus while it is still minimized, and wlrctl 0.2.2 has no
/// "unminimize" verb — `maximize` is what brings it back, at the cost that is #265's title: the
/// window returns maximized instead of at the size it was minimized at. `state:minimized` narrows
/// it to windows that are actually minimized, so presenting a visible window does not resize it.
fn restore_command(matchspec: &str) -> Vec<String> {
    let mut args = toplevel_command("maximize", matchspec);
    args.push("state:minimized".to_string());
    args
}

fn restore_args(title: &str) -> Vec<String> {
    restore_command(&format!("title:{title}"))
}

/// Every matchspec that names the window called `title`, most precise first.
///
/// `title:` first, which is exact and so cannot touch a second window by mistake. Then, for a
/// window that declared an app_id to the compositor, `app_id:` spelled as the compositor spelled
/// it — because a foreign app's title is not stable. Blender is `(Unsaved) - Blender 4.3.2` until
/// the scene is saved and `scene.blend - Blender 4.3.2` after; Chromium retitles itself on every
/// tab. The list this title came from is up to nine seconds old (`COMPOSITOR_TTL`), so `title:`
/// can miss a window that is plainly there, and the app_id is what still names it. Our own Slint
/// windows declare no app_id, and their titles do not move, so for them the list has one entry.
///
/// The fallback is offered only when no other open window shares the app_id. wlrctl applies a
/// verb to every toplevel the matchspec matches, and `close app_id:chromium` with two Chromium
/// windows open would close both — the coin toss `window_to_close` exists to refuse.
fn matchspecs(title: &str, open: &[WindowEntry]) -> Vec<String> {
    let mut specs = vec![format!("title:{title}")];
    if let Some(window) = open.iter().find(|w| w.title == title) {
        let id = &window.wayland_app_id;
        let alone = !id.is_empty() && open.iter().filter(|w| &w.wayland_app_id == id).count() == 1;
        if alone {
            specs.push(format!("app_id:{id}"));
        }
    }
    specs
}

/// One `wlrctl toplevel <verb>` command line per matchspec, in the order to try them.
fn commands_for(verb: &str, specs: &[String]) -> Vec<Vec<String>> {
    specs.iter().map(|spec| toplevel_command(verb, spec)).collect()
}

/// Run one `wlrctl` command. `Err` is for a wlrctl that could not run at all — a missing binary
/// gets its own sentence, because nothing on the machine will fix itself — and a wlrctl that ran
/// and matched nothing comes back as its exit status, for the caller to try another name with.
fn wlrctl_exit(args: &[String]) -> Result<std::process::ExitStatus, String> {
    match std::process::Command::new("wlrctl").args(args).status() {
        Ok(status) => Ok(status),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(
            "wlrctl is not installed on this machine, so nothing here can move a window"
                .to_string(),
        ),
        Err(e) => Err(format!("could not run wlrctl: {e}")),
    }
}

/// Run the command lines in turn until the compositor matches one, and say, in words a caller
/// can act on, what happened.
///
/// A non-zero exit means the compositor matched no window, which is the interesting failure: it
/// says the shell and the compositor disagree about what is open. With more than one command line
/// the refusal spells out each attempt, so a caller reading "exited 1" can see that the app_id
/// was tried too and there is no third name to reach for.
fn run_first_matching(commands: &[Vec<String>]) -> Result<(), String> {
    let mut refused = Vec::new();
    for args in commands {
        let status = wlrctl_exit(args)?;
        if status.success() {
            return Ok(());
        }
        refused.push(format!(
            "`wlrctl {}` exited {}",
            args.join(" "),
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "on a signal".to_string())
        ));
    }
    Err(format!("the compositor matched no window: {}", refused.join(", then ")))
}

/// Ask the compositor for something from the UI thread, and wait a moment for the answer.
///
/// The work goes to a worker because `wlrctl` is a process; the wait is bounded because the caller
/// is the thread that paints the desktop. A timeout comes back as a timeout and not as success —
/// a caller told "done" by a shell that does not know is exactly the complaint this fixes.
fn ask_compositor(commands: Vec<Vec<String>>) -> Result<(), String> {
    let spelled = commands
        .first()
        .map(|args| format!("wlrctl {}", args.join(" ")))
        .unwrap_or_else(|| "wlrctl".to_string());
    let (answer, wait) = std::sync::mpsc::channel();
    if std::thread::Builder::new()
        .name("yos-wlrctl".to_string())
        .spawn(move || {
            let _ = answer.send(run_first_matching(&commands));
        })
        .is_err()
    {
        return Err("could not start a thread to talk to the compositor".to_string());
    }
    match wait.recv_timeout(COMPOSITOR_REPLY_LIMIT) {
        Ok(result) => result,
        Err(_) => Err(format!(
            "`{spelled}` had not answered after {} ms, so what the compositor did with it is \
             not known here",
            COMPOSITOR_REPLY_LIMIT.as_millis()
        )),
    }
}

/// Bring the window called `title` to the front, restoring it first if it was minimized. Says
/// whether the compositor had such a window.
///
/// The un-minimise goes over the wlr foreign-toplevel protocol, not wlrctl (#265): wlrctl 0.2.2's
/// only verb that brings a minimized window back is `maximize`, so a window restored from the
/// taskbar came back maximized whatever size it went away at. The protocol's `unset_minimized`
/// brings it back at its old size; the old wlrctl call stays as the fallback for a compositor
/// that does not offer the protocol, and says so in the log when it runs.
pub fn present(title: &str) -> bool {
    let specs = matchspecs(title, &shell_windows());
    // The app_id fallback matchspecs decided to offer, if it offered one: the protocol path
    // names windows by exactly the two keys wlrctl does, under the same rules.
    let app_id = specs.iter().find_map(|s| s.strip_prefix("app_id:"));
    match crate::foreign_toplevel::restore(title, app_id) {
        crate::foreign_toplevel::Restore::Restored => {}
        crate::foreign_toplevel::Restore::NothingNamed => {
            // Not a warning, as before: the usual reason is that the window was never minimized.
            tracing::debug!(window = %title, "nothing to un-minimize before focusing");
        }
        crate::foreign_toplevel::Restore::Unavailable(why) => {
            tracing::warn!(
                window = %title,
                reason = %why,
                "the foreign-toplevel protocol is unavailable; falling back to `wlrctl maximize`, \
                 which brings a minimized window back maximized rather than at its old size"
            );
            if let Err(why) =
                run_first_matching(&specs.iter().map(|s| restore_command(s)).collect::<Vec<_>>())
            {
                tracing::debug!(window = %title, reason = %why, "nothing to un-minimize before focusing");
            }
        }
    }
    // The protocol's `activate` has already asked for the focus; this is the answer the caller
    // gets — `present` promises to say whether the compositor had such a window — and it still
    // names the window by every matchspec, which is all the NothingNamed and Unavailable paths
    // have left to try.
    match run_first_matching(&commands_for("focus", &specs)) {
        Ok(()) => true,
        Err(why) => {
            tracing::warn!(window = %title, reason = %why, "could not bring a window forward");
            false
        }
    }
}

/// Bring the window of one of our apps to the front, by the id the launcher knows it by.
///
/// By the title the window list has for it, not `display_name(id)`: those are the same string
/// for our own apps and different for a foreign one — `show_app blender` asked for `title:Blender`
/// and no window is called that. An app the list does not have yet (launched a moment ago, before
/// the compositor's next reading) is still asked for by its display name, which is the title it
/// will have if it is one of ours.
pub fn present_app(app_id: &str) -> bool {
    let title = shell_windows()
        .into_iter()
        .find(|w| w.app_id == app_id)
        .map(|w| w.title)
        .unwrap_or_else(|| display_name(app_id));
    present(&title)
}

/// Bring the shell's own window to the front. `Err` says why it is still behind something.
///
/// The shell is an ordinary toplevel to labwc — see the `<margin>` note in config/labwc/rc.xml,
/// which exists because the status bar and dock are a fullscreen window rather than a layer-shell
/// panel — so it gets in front the same way any other window does, and it cannot raise itself
/// under Wayland. Every surface that puts something in front of a person needs this:
/// `control_approvals` already asks for it when a card goes up, `open_lens` when the ask bar
/// opens, and `show_screen` because a screen nobody can see has not been shown.
pub fn raise_shell() -> Result<(), String> {
    ask_compositor(vec![toplevel_args("focus", SHELL_WINDOW_TITLE)])
}

/// Ask the window called `title` to close, the way pressing its × does.
///
/// The compositor's close request, deliberately, and not a signal to a process: an app holding
/// unsaved work is entitled to put up its own dialog and stay open, and the shell has no standing
/// to overrule it. So `Ok` here means the request was delivered, never that the window went.
pub fn close(title: &str) -> Result<(), String> {
    ask_compositor(commands_for("close", &matchspecs(title, &shell_windows())))
}

/// Put the window called `title` out of the way, leaving it running.
///
/// wlrctl spells the verb the American way; this desktop's own surface does not, which is why the
/// two spellings meet here rather than anywhere a caller can see.
pub fn minimise(title: &str) -> Result<(), String> {
    ask_compositor(commands_for("minimize", &matchspecs(title, &shell_windows())))
}

/// Fill the screen with the window called `title`, the way its own maximise button does.
///
/// wlrctl 0.2.2 has no unmaximize — `maximize` is the whole of what it can do to a window's
/// size — so there is no restore half to offer beside this: the window stays maximized until
/// its app or the compositor's own binding (Super+Up, config/labwc/rc.xml) says otherwise.
/// Callers that would like a Maximise/Restore toggle cannot have one honestly, because
/// `toplevel list` does not carry the state to read the toggle back from. On a minimized
/// window `maximize` is also what brings it back (see `restore_command`), so this doubles
/// as restore-and-fill.
pub fn maximise(title: &str) -> Result<(), String> {
    ask_compositor(commands_for("maximize", &matchspecs(title, &shell_windows())))
}

// ── Which window a person meant ─────────────────────────────────────

/// Every window a caller may name, the shell's own included.
///
/// [`shell_windows`] leaves the shell out on purpose: the taskbar must not offer to switch you to
/// the desktop you are already looking at. A control surface is the opposite case — the shell's
/// window is the one thing on this machine that a mind cannot reach any other way, and
/// `focus_window title=Yantrik` answering "no open window matches" while the desktop was plainly
/// running is how that omission was found.
pub fn addressable_titles() -> Vec<String> {
    let mut titles: Vec<String> = shell_windows().into_iter().map(|w| w.title).collect();
    if !titles.iter().any(|t| t == SHELL_WINDOW_TITLE) {
        titles.push(SHELL_WINDOW_TITLE.to_string());
    }
    titles
}

/// The open windows that answer to `want`, best first.
///
/// An exact title wins outright and alone, because "Notes" is Notes even while "Notes: Handover"
/// is open. Failing that it is a substring, which is what a person typing part of a title means.
///
/// The shell comes last among the loose matches, and only among them. It is the one window that is
/// always addressable and never in the window list, so it must not shadow something the person can
/// actually see — but with nothing else matching, `Yantrik` has to reach the desktop, which is the
/// whole reason it is in the candidate list.
fn matching_titles(want: &str, open: &[String]) -> Vec<String> {
    let want = want.trim().to_lowercase();
    if want.is_empty() {
        return Vec::new();
    }
    if let Some(exact) = open.iter().find(|title| title.to_lowercase() == want) {
        return vec![exact.clone()];
    }
    let shell_matches = SHELL_WINDOW_TITLE.to_lowercase().contains(&want)
        && open.iter().any(|title| title == SHELL_WINDOW_TITLE);
    let mut found: Vec<String> = open
        .iter()
        .filter(|title| *title != SHELL_WINDOW_TITLE && title.to_lowercase().contains(&want))
        .cloned()
        .collect();
    if shell_matches {
        found.push(SHELL_WINDOW_TITLE.to_string());
    }
    found
}

/// The refusal when nothing open answers to that name.
///
/// One sentence for all three verbs: a caller being told what is open should not have to learn it
/// twice in two different wordings.
fn nothing_matches(want: &str, open: &[String]) -> String {
    format!("no open window matches `{}`; there is: {}", want.trim(), open.join(", "))
}

/// The window a caller means, or a refusal that names what is open instead.
///
/// For the verbs a person can undo by hand — focus, minimise. The first match is good enough for
/// those: guessing wrong shows itself immediately and costs one more call to put right.
pub fn window_named(want: &str, open: &[String]) -> Result<String, String> {
    matching_titles(want, open)
        .into_iter()
        .next()
        .ok_or_else(|| nothing_matches(want, open))
}

/// The window to close, or a refusal saying why nothing was closed.
///
/// Stricter than [`window_named`] on two counts, because closing is the one verb here that a
/// person cannot undo by clicking something.
///
/// An ambiguous title is refused rather than resolved to whichever window came back first: with
/// two Chromium windows open, `close_window title=chromium` picking one of them is a coin toss
/// with somebody's tab in it.
///
/// And the desktop itself is refused. `Yantrik OS` is the shell — the status bar, the dock, the
/// Lens and every screen — so closing it ends the session, which is not what anyone asking to
/// close a window means. Stepping away has `lock`.
pub fn window_to_close(want: &str, open: &[String]) -> Result<String, String> {
    let found = matching_titles(want, open);
    let title = match found.len() {
        0 => return Err(nothing_matches(want, open)),
        1 => found[0].clone(),
        _ => {
            return Err(format!(
                "`{}` matches {} open windows and closing the wrong one cannot be undone; \
                 say which: {}",
                want.trim(),
                found.len(),
                found.join(", ")
            ))
        }
    };
    if title == SHELL_WINDOW_TITLE {
        return Err(format!(
            "`{SHELL_WINDOW_TITLE}` is the desktop itself — the status bar, the dock and every \
             screen — so closing it ends the session rather than a window. Use `lock` to step \
             away, or name one of the app windows"
        ));
    }
    Ok(title)
}

/// Ask the compositor what is on screen, through `wlrctl toplevel list`.
///
/// The only account of a window this process did not start — an app a person launched from a
/// terminal, and, the case this exists for, an app that was open before the shell restarted.
/// Never called from the UI thread: `refresh_compositor_windows` is what runs it, and everything
/// else reads the cache it fills.
fn wlrctl_windows() -> Vec<WindowEntry> {
    let output = match std::process::Command::new("wlrctl")
        .args(["toplevel", "list"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .filter(|line| !line.trim().is_empty())
        // Not the shell itself. `wlrctl` lists every toplevel on the compositor, and one of them
        // is always this process — so on a machine with nothing launched, the taskbar and the
        // desktop's own "open windows" list both offered to switch you to the desktop you are
        // already looking at.
        .filter(|line| split_toplevel_line(line).0 != SHELL_WINDOW_TITLE)
        .map(toplevel_entry)
        .collect()
}

/// Which toplevel the compositor says is in front, right now, as `wlrctl toplevel list
/// state:activated`.
///
/// `state:activated` is wlrctl's own matcher for the focused toplevel. This trusts it only when
/// it answers with exactly one line naming a window that is not the shell's: two lines or none
/// means either the compositor has nothing activated or this wlrctl does not support the matcher
/// and has listed everything — and both of those are "not knowable", not "probably the first
/// one". Callers that put a person's screen somewhere based on the answer are right to do
/// nothing when it is not knowable.
///
/// Spawns a process: called from `refresh_compositor_windows`, on the compositor-reading path,
/// and off the UI thread by the approval cards when they need the answer for THIS moment rather
/// than the cached one.
pub(crate) fn front_now() -> Option<String> {
    let output = std::process::Command::new("wlrctl")
        .args(["toplevel", "list", "state:activated"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    activated_title(&String::from_utf8_lossy(&output.stdout))
}

/// The title of the single activated toplevel in a `wlrctl toplevel list state:activated`
/// answer, or `None` if the answer is not exactly one window outside the shell. Parsed with
/// [`toplevel_entry`], the same reader the full list uses, so the title is spelled the way the
/// list spells it — which is what lets the merge match it.
fn activated_title(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.len() != 1 {
        return None;
    }
    let title = toplevel_entry(lines[0]).title;
    if title.is_empty() || title == SHELL_WINDOW_TITLE {
        // The person is looking at the desktop. No app window is in front.
        return None;
    }
    Some(title)
}

/// One `wlrctl toplevel list` line, as `(title, app_id)` — the shell's id, see [`toplevel_entry`].
fn split_toplevel_line(line: &str) -> (String, String) {
    let window = toplevel_entry(line);
    (window.title, window.app_id)
}

/// One `wlrctl toplevel list` line, as the window it describes.
///
/// The format is `app_id: title`. Reading the whole line as the title put that separator into the
/// name, and our own windows set no wayland app_id at all, so the taskbar showed every one of them
/// as ": Terminal", ": Weather" — a stray colon in front of the name, on the strip of the desktop
/// people look at most.
///
/// The declared app_id is kept twice: lowercased as the shell's `app_id`, which is what every
/// table in the shell is keyed by, and verbatim as `wayland_app_id`, which is what the compositor
/// answers to — `app_id:Blender` finds Blender's window and `app_id:blender` does not.
fn toplevel_entry(line: &str) -> WindowEntry {
    let (declared_id, title) = match line.split_once(':') {
        Some((id, rest)) if !rest.trim().is_empty() => (id.trim(), rest.trim()),
        // A foreign toplevel with no separator at all is all title.
        _ => ("", line.trim()),
    };
    let title = title.to_string();
    // Prefer what the window calls itself. Our own windows call themselves nothing — Slint gives
    // them no Wayland app_id — so the title is matched against APP_NAMES next, which is a lookup
    // rather than a guess: those strings ARE the window titles our apps declare, and
    // `app_names_agree_everywhere` fails the build if one drifts.
    //
    // Guessing was the whole problem. `derive_app_id` takes the first word, so the System Monitor
    // window came back as `system`, Downloads as `download`, yDoc as `ydoc` and Images as
    // `images` — none of which is the id the dock keys its running mark by, so after a shell
    // restart those four tiles stayed dark with the apps plainly open on screen. Guessing is now
    // the last resort, for windows that are neither ours nor self-identifying.
    let app_id = if declared_id == crate::mind_view::NESTED_APP_ID {
        // The window a nested compositor draws into, which is Mind View. wlroots names it
        // itself ("wlroots - WL-1") and the title is kept as it is, because the title is what
        // the taskbar hands `wlrctl` to find it again.
        crate::mind_view::APP_ID.to_string()
    } else if !declared_id.is_empty() {
        declared_id.to_lowercase()
    } else if let Some(id) = app_id_for_title(&title) {
        id.to_string()
    } else {
        derive_app_id(&title)
    };
    WindowEntry {
        icon_char: icon_for_app(&app_id).to_string(),
        subtitle: derive_context(&title, &app_id),
        wayland_app_id: declared_id.to_string(),
        title,
        app_id,
    }
}

/// The app id whose window is titled exactly this, if it is one of ours.
///
/// Exactly, not loosely: "Notes" is Notes and "Notes: Handover" is a note open in it, and a
/// substring match would make the second one a second copy of the first in every window list.
fn app_id_for_title(title: &str) -> Option<&'static str> {
    APP_NAMES.iter().find(|(_, name)| *name == title).map(|(id, _)| *id)
}

/// Derive a normalized app_id from a window title (fallback path only).
pub(crate) fn derive_app_id(title: &str) -> String {
    // An agent's own window (Agents → Pop out) is titled after its task, and a task can say
    // "files" or "terminal" — which would mark Files or Terminal open in the taskbar. Its prefix
    // says whose window it is before the words of the task are looked at.
    if title.starts_with(crate::agents::WINDOW_TITLE_PREFIX) {
        return "agents".to_string();
    }
    let lower = title.to_lowercase();
    if lower.contains("foot") || lower.contains("terminal") {
        "terminal".to_string()
    } else if lower.contains("firefox") || lower.contains("chromium") || lower.contains("browser")
    {
        "browser".to_string()
    } else if lower.contains("file") || lower.contains("pcmanfm") || lower.contains("thunar") {
        "files".to_string()
    } else if lower.contains("yantrik") {
        "yantrik".to_string()
    } else {
        lower
            .split_whitespace()
            .next()
            .unwrap_or("unknown")
            .to_string()
    }
}

/// Map app_id to a single-char icon.
pub fn icon_for_app(app_id: &str) -> &'static str {
    match app_id {
        "terminal" => ">_",
        "browser" => "W",
        "files" => "F",
        "notes" => "\u{270E}",
        "email" => "@",
        "calendar" => "\u{25A6}",
        "weather" => "\u{2600}",
        "music" => "\u{266A}",
        "sysmonitor" => "\u{25C9}",
        "network" => "N",
        "spreadsheet" => "YS",
        "documents" => "YD",
        "presentation" => "YP",
        "yantrik" => "Y",
        "mind-view" => "\u{25CE}",
        _ => "?",
    }
}

/// Derive a contextual subtitle from a window title (fallback path only).
/// Terminal: extract CWD from "user@host:/path" pattern.
/// Browser: extract site name from "Page Title - Site" pattern.
/// Files: extract current directory.
fn derive_context(title: &str, app_id: &str) -> String {
    match app_id {
        "terminal" => {
            if let Some(idx) = title.find(':') {
                let path = title[idx + 1..].trim();
                if !path.is_empty() {
                    return path.to_string();
                }
            }
            String::new()
        }
        "browser" => {
            let sep = if title.contains(" - ") {
                " - "
            } else if title.contains(" — ") {
                " — "
            } else {
                return String::new();
            };
            title.rsplit(sep).next()
                .filter(|s| !s.eq_ignore_ascii_case("firefox") && !s.eq_ignore_ascii_case("chromium"))
                .unwrap_or("")
                .to_string()
        }
        // wlroots titles the window "wlroots - WL-1", which says nothing to a person.
        "mind-view" => "Mind View".to_string(),
        "files" => {
            if title.contains('/') {
                title.rsplit('/').next().unwrap_or("").to_string()
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::running::RunningApp;

    /// A registry entry as `running()` hands it to the merge: the id it was launched under and
    /// the binary that was spawned, which is a full path for Blender and a bare name otherwise.
    fn launched(app_id: &str, binary: &str) -> RunningApp {
        RunningApp { app_id: app_id.into(), pid: 1, binary: binary.into(), since_unix: 0 }
    }

    /// What the compositor reported, one `wlrctl toplevel list` line each.
    fn seen(lines: &[&str]) -> Vec<WindowEntry> {
        lines.iter().map(|line| toplevel_entry(line)).collect()
    }

    #[test]
    fn surviving_windows_remain_when_editor_is_launched() {
        let merged = merge_windows(
            &[launched("editor", "yantrik-text-editor")],
            seen(&[": Terminal", ": Notes", ": Editor"]),
            None,
        );
        assert_eq!(merged.iter().map(|w|w.app_id.as_str()).collect::<Vec<_>>(),["editor","terminal","notes"]);
    }


    /// The case the shell used to get wrong: the launch registry is empty because this process
    /// has just started, and four apps are on screen because the compositor did not restart.
    ///
    /// `describe shell` said "0 windows open" and the dock showed nothing running. The merge is
    /// what answers it — with an empty registry the compositor's list IS the list.
    #[test]
    fn a_shell_that_has_just_started_still_sees_the_windows_already_open() {
        let restarted_into = seen(&["notes: Notes", ": Terminal", ": Editor", "firefox: Mozilla Firefox"]);
        let merged = merge_windows(&[], restarted_into, None);
        assert_eq!(merged.len(), 4, "every window the compositor still holds is open");
        assert_eq!(
            merged.iter().map(|w| w.app_id.as_str()).collect::<Vec<_>>(),
            ["notes", "terminal", "editor", "firefox"]
        );
    }

    /// The window list and the window actions beside it agree about Blender (#113).
    ///
    /// The shell launched Blender, so the registry had it, and the registry titled it "Blender".
    /// That is what `describe shell` listed; the compositor's line for the same window was
    /// `Blender: (Unsaved) - Blender 4.3.2`. So `minimise_window title=Blender` resolved to
    /// "Blender", ran `wlrctl toplevel minimize title:Blender`, and matched nothing — and the
    /// real title, the one string wlrctl would have taken, was refused by the validator because
    /// the list did not show it. No value worked for that window.
    #[test]
    fn a_launched_app_is_listed_by_the_title_the_compositor_has_for_it() {
        let real = "(Unsaved) - Blender 4.3.2";
        let merged = merge_windows(
            &[launched("blender", "/usr/bin/blender")],
            seen(&[": Terminal", "Blender: (Unsaved) - Blender 4.3.2"]),
            None,
        );
        assert_eq!(merged.len(), 2, "one Blender window, listed once: {merged:?}");
        let blender = &merged[0];
        assert_eq!(blender.app_id, "blender", "the dock and `show_app` still know it by the shell's id");
        assert_eq!(blender.title, real, "the list shows the title the compositor can be asked for");
        assert_eq!(blender.wayland_app_id, "Blender", "spelled as the compositor spelled it");

        // What the list shows is what the validator accepts: the real title exactly, and the
        // app's name as part of it.
        let titles: Vec<String> = merged.iter().map(|w| w.title.clone()).collect();
        assert_eq!(window_named(real, &titles).unwrap(), real);
        assert_eq!(window_named("Blender", &titles).unwrap(), real);
        assert_eq!(window_to_close("blender", &titles).unwrap(), real);

        // And the compositor is asked by that title, then by the app_id it declared — with the
        // capital B, because `app_id:blender` matches nothing on labwc while `app_id:Blender`
        // does — for when the title has moved on since the list was read.
        assert_eq!(
            matchspecs(real, &merged),
            ["title:(Unsaved) - Blender 4.3.2", "app_id:Blender"]
        );
        assert_eq!(
            commands_for("minimize", &matchspecs(real, &merged)),
            [
                vec!["toplevel", "minimize", "title:(Unsaved) - Blender 4.3.2"],
                vec!["toplevel", "minimize", "app_id:Blender"],
            ]
        );
    }

    /// Until the compositor's next reading has the window, the registry still says the app is
    /// open — under the name it will have if it is one of ours.
    #[test]
    fn an_app_launched_a_moment_ago_is_listed_from_the_registry_alone() {
        let merged = merge_windows(&[launched("notes", "yantrik-notes")], seen(&[": Terminal"]), None);
        assert_eq!(merged[0].title, "Notes");
        assert_eq!(merged[0].app_id, "notes");
        assert_eq!(merged[0].wayland_app_id, "", "nothing declared, so nothing to fall back to");
        assert_eq!(matchspecs("Notes", &merged), ["title:Notes"]);
    }

    /// The phantom "Browser" (#113): `open_app browser` runs `chromium`, and the window comes
    /// back from the compositor as app_id `chromium`, so on id alone the merge saw two
    /// applications — a "Browser" with no toplevel behind it, and the real Chromium window —
    /// and the taskbar showed the one window twice.
    #[test]
    fn the_browser_the_shell_launched_is_the_chromium_window_the_compositor_sees() {
        let merged = merge_windows(
            &[launched("browser", "chromium")],
            seen(&["chromium: webgl-check.html - Chromium", ": Notes"]),
            None,
        );
        assert_eq!(merged.len(), 2, "one browser window, listed once: {merged:?}");
        assert_eq!(merged[0].app_id, "browser", "the pin's running mark is keyed by the shell's id");
        assert_eq!(merged[0].title, "webgl-check.html - Chromium");
        assert_eq!(merged[0].wayland_app_id, "chromium");
        assert_eq!(merged[0].icon_char, icon_for_app("browser"));
        assert_eq!(matchspecs("webgl-check.html - Chromium", &merged),
            ["title:webgl-check.html - Chromium", "app_id:chromium"]);
    }

    /// The stuck underline (#76): the taskbar draws its active marker on the FIRST entry of this
    /// list, and nothing had ever ordered the list by focus — the launch registry sorts itself by
    /// id, so Calendar sat under the marker in every one of a person's morning screenshots while
    /// Email, the Editor and Notes took turns being the window actually in front.
    #[test]
    fn the_window_the_compositor_says_is_in_front_is_listed_first() {
        let merged = merge_windows(
            &[
                launched("calendar", "yantrik-calendar"),
                launched("editor", "yantrik-editor"),
                launched("email", "yantrik-email"),
            ],
            seen(&[": Calendar", ": Editor", ": Email"]),
            Some("Email"),
        );
        assert_eq!(
            merged.iter().map(|w| w.app_id.as_str()).collect::<Vec<_>>(),
            ["email", "calendar", "editor"],
            "the front window leads, and the rest keep the order the merge made them in"
        );

        // A foreign window is named by its title, exactly as the same reading put it on the list.
        let merged = merge_windows(
            &[launched("notes", "yantrik-notes")],
            seen(&[": Notes", "chromium: Ask | Hacker News - Chromium"]),
            Some("Ask | Hacker News - Chromium"),
        );
        assert_eq!(merged[0].app_id, "chromium", "the browser is what labwc has in front");
    }

    /// No focus answer, or an answer about a window the list does not have, leaves the list
    /// exactly as the merge made it. A window closed since the reading is nobody's focus; the
    /// marker falls back to the plain order rather than to a guess.
    #[test]
    fn a_focus_answer_that_names_nothing_on_the_list_moves_nothing() {
        let as_merged = |front: Option<&str>| {
            merge_windows(
                &[launched("calendar", "yantrik-calendar"), launched("editor", "yantrik-editor")],
                seen(&[": Calendar", ": Editor"]),
                front,
            )
        };
        for (front, named) in [(None, "no focus answer"), (Some("Notes"), "an answer about a closed window")] {
            assert_eq!(
                as_merged(front).iter().map(|w| w.app_id.as_str()).collect::<Vec<_>>(),
                ["calendar", "editor"],
                "{named} must not reorder the list"
            );
        }
    }

    /// The `state:activated` answer is believed only when it names exactly one window, and names
    /// it the way the list spells it — same parser, so a title from one can be looked for in the
    /// other. Nothing activated, an unsupported matcher listing everything, and the desktop
    /// itself in front are all "not knowable", and say so as `None`.
    #[test]
    fn the_activated_answer_is_read_the_way_the_list_is_read() {
        assert_eq!(activated_title(": Terminal").as_deref(), Some("Terminal"));
        assert_eq!(
            activated_title("chromium: Ask | Hacker News - Chromium").as_deref(),
            Some("Ask | Hacker News - Chromium")
        );
        assert_eq!(activated_title(""), None);
        assert_eq!(activated_title(": Editor\n: Terminal\n"), None);
        assert_eq!(activated_title(": Yantrik OS"), None);
    }

    /// A program's app_id is its binary's name, give or take a distribution's suffix.
    #[test]
    fn a_window_is_paired_with_the_binary_that_was_started_for_it() {
        assert!(same_program("chromium", "chromium"));
        assert!(same_program("/usr/bin/blender", "Blender"));
        assert!(same_program("chromium-browser", "chromium"));
        assert!(same_program("google-chrome-stable", "google-chrome"));
        assert!(same_program("firefox", "firefox-esr"));
        assert!(!same_program("yantrik-notes", "Blender"));
        assert!(!same_program("chromium", "chrome"), "a prefix that is not a whole word is not the same program");
        assert!(!same_program("yantrik-notes", ""), "our windows declare nothing, and nothing pairs with nothing");
    }

    /// With two windows sharing an app_id, only the title can name one of them.
    ///
    /// wlrctl applies a verb to every toplevel the matchspec matches, so `close app_id:chromium`
    /// with two Chromium windows open closes both. The fallback is withheld rather than guessed.
    #[test]
    fn a_window_sharing_its_app_id_with_another_is_named_only_by_title() {
        let open = seen(&[
            "chromium: Northwind Cloud - Pricing - Chromium",
            "chromium: Ask | Hacker News - Chromium",
            "Blender: (Unsaved) - Blender 4.3.2",
        ]);
        assert_eq!(
            matchspecs("Ask | Hacker News - Chromium", &open),
            ["title:Ask | Hacker News - Chromium"]
        );
        assert_eq!(
            matchspecs("(Unsaved) - Blender 4.3.2", &open),
            ["title:(Unsaved) - Blender 4.3.2", "app_id:Blender"]
        );
        // The shell's own window is never in this list and is named by its title alone.
        assert_eq!(matchspecs(SHELL_WINDOW_TITLE, &open), ["title:Yantrik OS"]);
    }

    /// The declared app_id is kept as the compositor spelled it, beside the lowercased one the
    /// shell keys everything by.
    #[test]
    fn a_declared_app_id_is_kept_as_the_compositor_spelled_it() {
        let blender = toplevel_entry("Blender: (Unsaved) - Blender 4.3.2");
        assert_eq!(blender.app_id, "blender");
        assert_eq!(blender.wayland_app_id, "Blender");
        assert_eq!(toplevel_entry(": Notes").wayland_app_id, "");

        // The nested compositor's window is Mind View (#239). Its title stays what wlroots
        // declared, because that is what `wlrctl` finds it by; the subtitle says what it is.
        let mind_view = toplevel_entry("wlroots: wlroots - WL-1");
        assert_eq!(mind_view.app_id, "mind-view");
        assert_eq!(mind_view.title, "wlroots - WL-1");
        assert_eq!(mind_view.subtitle, "Mind View");
        assert_eq!(mind_view.wayland_app_id, "wlroots");
        assert_eq!(toplevel_entry("Some Foreign Window").wayland_app_id, "");
    }

    /// Every name in APP_NAMES is a window title the compositor can hand back, and it has to come
    /// back as the id the dock keys its running mark by — otherwise the app is open and its tile
    /// is dark. Five of these used to land on something else entirely.
    #[test]
    fn our_own_window_titles_resolve_to_the_id_the_dock_uses() {
        for (id, name) in APP_NAMES {
            // What labwc reports for a Slint window: no app_id, then the title.
            let (_, resolved) = split_toplevel_line(&format!(": {name}"));
            assert_eq!(&resolved, id, "the window titled {name:?} must be `{id}`");
        }
    }

    /// The five the first-word guess got wrong, named so the regression is readable.
    #[test]
    fn the_windows_the_first_word_guess_misnamed() {
        for (title, want) in [
            ("System Monitor", "sysmonitor"),
            ("Downloads", "downloads"),
            ("Images", "image"),
            ("yDoc", "documents"),
            ("yPresent", "presentation"),
        ] {
            assert_eq!(split_toplevel_line(&format!(": {title}")).1, want);
        }
    }

    #[test]
    fn a_window_with_no_app_id_is_named_without_the_separator() {
        // What labwc actually reports for our Slint windows, which set a title and no app_id.
        assert_eq!(split_toplevel_line(": Terminal").0, "Terminal");
        assert_eq!(split_toplevel_line(": Yantrik OS").0, "Yantrik OS");
        assert_eq!(split_toplevel_line(": Snippet Manager").0, "Snippet Manager");
    }

    #[test]
    fn the_shell_is_not_one_of_its_own_open_windows() {
        // wlrctl reports this process too. Offering to switch to the desktop, from the desktop,
        // is the kind of thing that makes a shell feel like it is not paying attention.
        assert_eq!(split_toplevel_line(": Yantrik OS").0, SHELL_WINDOW_TITLE);
    }

    #[test]
    fn a_foreign_window_keeps_the_id_it_declares() {
        let (title, app_id) = split_toplevel_line("firefox: Mozilla Firefox");
        assert_eq!(title, "Mozilla Firefox");
        assert_eq!(app_id, "firefox");
    }

    #[test]
    fn a_colon_in_the_title_itself_survives() {
        // Only the first separator divides the two fields; the rest belongs to the name.
        assert_eq!(split_toplevel_line(": Notes: Handover").0, "Notes: Handover");
        assert_eq!(split_toplevel_line("notes: Notes: Handover").0, "Notes: Handover");
    }

    #[test]
    fn a_line_with_no_separator_is_all_title() {
        assert_eq!(split_toplevel_line("Some Foreign Window").0, "Some Foreign Window");
    }

    /// The exact command lines the shell hands wlrctl, because every one of them has been wrong
    /// at some point and each was wrong in silence.
    ///
    /// `title:` is not decoration. Without the key, wlrctl reads the word as an app_id; our Slint
    /// windows declare no app_id; a match on nothing exits zero. That is how the taskbar came to
    /// do nothing when clicked, for every window, without a line in the log.
    #[test]
    fn the_command_lines_name_the_window_by_title() {
        assert_eq!(
            toplevel_args("focus", "Editor"),
            ["toplevel", "focus", "title:Editor"]
        );
        assert_eq!(
            toplevel_args("close", "Notes: Handover"),
            ["toplevel", "close", "title:Notes: Handover"],
            "a colon in the title belongs to the title; wlrctl splits the matchspec on the first"
        );
        // The restore line is the fallback for a compositor without the foreign-toplevel
        // protocol; the one `present` normally sends is built in foreign_toplevel.rs.
        assert_eq!(
            restore_args("System Monitor"),
            ["toplevel", "maximize", "title:System Monitor", "state:minimized"],
            "only windows that ARE minimized, or presenting a visible window resizes it"
        );
        // One argument each, so a title with a space travels as a title with a space. There is no
        // shell between us and wlrctl and there must not be a quoting scheme pretending there is.
        assert_eq!(toplevel_args("focus", "Yantrik OS").len(), 3);
    }

    /// wlrctl spells it `minimize`. This desktop's action is `minimise_window`, and the two
    /// spellings are allowed to meet in exactly one place — here.
    #[test]
    fn minimise_asks_the_compositor_to_minimize() {
        assert_eq!(
            toplevel_args("minimize", "Calendar"),
            ["toplevel", "minimize", "title:Calendar"]
        );
    }

    /// The same rendezvous for the taskbar menu's Maximise row (#232) and the `maximise_window`
    /// action beside `minimise_window`. No `state:` on the end, unlike [`restore_args`]: this
    /// maximizes whatever window answers to the title, minimized or not — and there is no
    /// unmaximize command line to pin here, because wlrctl 0.2.2 has no such verb.
    #[test]
    fn maximise_asks_the_compositor_to_maximize() {
        assert_eq!(
            toplevel_args("maximize", "Files"),
            ["toplevel", "maximize", "title:Files"]
        );
    }

    /// The shell asks for itself by the title its own window carries, and by nothing else:
    /// `title:` is an exact, case-sensitive comparison in wlrctl, so `title:Yantrik` matches
    /// no window on a machine whose shell is called `Yantrik OS`.
    #[test]
    fn the_shell_asks_for_itself_by_its_whole_title() {
        assert_eq!(
            toplevel_args("focus", SHELL_WINDOW_TITLE),
            ["toplevel", "focus", "title:Yantrik OS"]
        );
    }

    fn open(titles: &[&str]) -> Vec<String> {
        titles.iter().map(|t| (*t).to_string()).collect()
    }

    /// The desktop was reachable by no name at all.
    ///
    /// `focus_window title=Yantrik` answered "no open window matches `yantrik`" on a machine with
    /// the shell plainly running, because the window list leaves the shell out — deliberately, so
    /// the taskbar does not offer to switch you to the desktop you are on — and the control
    /// surface read that same list. The shell is a window; a caller has to be able to name it.
    #[test]
    fn the_shell_answers_to_its_own_name() {
        let desktop = open(&["Editor", "Terminal", "Notes", SHELL_WINDOW_TITLE]);
        assert_eq!(window_named("Yantrik", &desktop).unwrap(), SHELL_WINDOW_TITLE);
        assert_eq!(window_named("yantrik", &desktop).unwrap(), SHELL_WINDOW_TITLE);
        assert_eq!(window_named("Yantrik OS", &desktop).unwrap(), SHELL_WINDOW_TITLE);
        assert_eq!(window_named("  yantrik os  ", &desktop).unwrap(), SHELL_WINDOW_TITLE);
    }

    /// The desktop is always one of the windows a caller can name.
    ///
    /// `shell_windows` — what the taskbar and `describe shell` read — leaves the shell out, and
    /// that is right for both of them. The control surface read the same list, which is how the
    /// one window that is always open became the one window nothing could ask for.
    #[test]
    fn the_desktop_is_always_addressable_even_with_nothing_else_open() {
        let open = addressable_titles();
        assert!(
            open.iter().any(|t| t == SHELL_WINDOW_TITLE),
            "the shell's own window has to be nameable: {open:?}"
        );
        assert_eq!(window_named("Yantrik", &open).unwrap(), SHELL_WINDOW_TITLE);
    }

    /// An exact title wins outright, even while a longer title contains it.
    #[test]
    fn an_exact_title_beats_a_window_that_merely_contains_it() {
        let desktop = open(&["Notes: Handover", "Notes", SHELL_WINDOW_TITLE]);
        assert_eq!(window_named("notes", &desktop).unwrap(), "Notes");
        assert_eq!(window_named("handover", &desktop).unwrap(), "Notes: Handover");
    }

    /// The desktop does not shadow a window the person can see.
    ///
    /// A terminal showing `yantrik@home: ~` contains "yantrik"; so does the shell. The one on
    /// screen is the one meant, and the shell is still there when nothing else matches.
    #[test]
    fn the_shell_comes_last_among_the_loose_matches() {
        let desktop = open(&["yantrik@home: ~", SHELL_WINDOW_TITLE]);
        assert_eq!(window_named("yantrik", &desktop).unwrap(), "yantrik@home: ~");
        assert_eq!(window_named("OS", &desktop).unwrap(), SHELL_WINDOW_TITLE);
    }

    /// A refusal says what IS open, so the next call can be right.
    #[test]
    fn naming_nothing_open_says_what_is() {
        let desktop = open(&["Editor", SHELL_WINDOW_TITLE]);
        let err = window_named("gimp", &desktop).unwrap_err();
        assert!(err.contains("no open window matches `gimp`"), "{err}");
        assert!(err.contains("Editor"), "{err}");
        assert!(err.contains(SHELL_WINDOW_TITLE), "the desktop is one of the windows: {err}");
        assert!(window_named("   ", &desktop).is_err(), "an empty title matches nothing");
    }

    /// Closing is the one verb here nobody can undo with a click, so it refuses to guess.
    #[test]
    fn closing_an_ambiguous_title_is_refused_rather_than_guessed() {
        let desktop = open(&[
            "Northwind Cloud - Pricing - Chromium",
            "Ask | Hacker News - Chromium",
            SHELL_WINDOW_TITLE,
        ]);
        let err = window_to_close("chromium", &desktop).unwrap_err();
        assert!(err.contains("matches 2 open windows"), "{err}");
        assert!(err.contains("Hacker News"), "the refusal names them: {err}");
        // Naming one of them exactly still works.
        assert_eq!(
            window_to_close("Ask | Hacker News - Chromium", &desktop).unwrap(),
            "Ask | Hacker News - Chromium"
        );
        // And the same ambiguity is fine for a verb that can be undone.
        assert!(window_named("chromium", &desktop).is_ok());
    }

    /// Closing the desktop is not closing a window.
    #[test]
    fn the_desktop_itself_is_not_closable() {
        let desktop = open(&["Editor", SHELL_WINDOW_TITLE]);
        for asked in ["Yantrik", "Yantrik OS", "yantrik os"] {
            let err = window_to_close(asked, &desktop)
                .expect_err("closing the shell must be refused");
            assert!(err.contains("is the desktop itself"), "{err}");
            assert!(err.contains("lock"), "the refusal offers what was probably meant: {err}");
        }
        assert_eq!(window_to_close("Editor", &desktop).unwrap(), "Editor");
    }

    // What used to be here: a hand-written list of every app id and the window title it was
    // expected to produce, asserting display_name() matched. It did the right job and carried
    // the wrong kind of list — a second copy of the names, maintained by hand, which went stale
    // the moment the names were settled in one place.
    //
    // `app_name_tests::app_names_agree_everywhere` is the replacement. It reads the .desktop
    // entries and each app's Window title off disk and compares them to APP_NAMES, so it checks
    // the same invariant — that the taskbar label is exactly the window title, because the
    // taskbar sends that string to `wlrctl toplevel focus title:…` and a label merely CLOSE to
    // the title is a click that does nothing — without anyone having to remember to update it.
}

#[cfg(test)]
mod app_name_tests {
    use super::{display_name, APP_NAMES};
    use std::path::{Path, PathBuf};

    /// desktop-file stem -> the shell's own app id for the same application.
    ///
    /// Two naming schemes, both correct. A freedesktop entry needs a name unique across the
    /// whole machine, so ours are `yantrik-download-manager`; the shell calls the same thing
    /// `downloads`, which is what the icon set and the dock are keyed by.
    const STEM_TO_ID: &[(&str, &str)] = &[
        ("arcade", "arcade"),
        ("calendar", "calendar"),
        ("container-manager", "containers"),
        ("document-editor", "documents"),
        ("download-manager", "downloads"),
        ("email", "email"),
        ("image-viewer", "image"),
        ("music-player", "music"),
        ("network-manager", "network"),
        ("notes", "notes"),
        ("presentation", "presentation"),
        ("snippet-manager", "snippets"),
        ("spreadsheet", "spreadsheet"),
        ("studio", "studio"),
        ("system-monitor", "sysmonitor"),
        ("terminal", "terminal"),
        ("text-editor", "editor"),
        ("weather", "weather"),
    ];

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("the crate sits two levels under the repository root")
    }

    fn field(text: &str, key: &str) -> Option<String> {
        text.lines()
            .find_map(|l| l.strip_prefix(key))
            .map(|v| v.trim().to_string())
    }

    /// An application answers to ONE name, on every surface that shows it.
    ///
    /// It answered to four. The dock said "Editor", the launcher said "Text Editor", the
    /// window title said "Text Editor", and the taskbar entry beneath that window said
    /// something else again. Downloads was "Download Manager" in three places and "Downloads"
    /// in a fourth. Each was defensible alone, which is why it lasted — it only reads as
    /// sloppy when two are on screen together, and the taskbar entry sits directly under the
    /// window it names, so they always are.
    #[test]
    fn app_names_agree_everywhere() {
        let root = repo_root();
        let mut wrong = Vec::new();

        for (stem, id) in STEM_TO_ID {
            let want = APP_NAMES
                .iter()
                .find(|(k, _)| k == id)
                .map(|(_, n)| *n)
                .unwrap_or_else(|| panic!("{id} is not in APP_NAMES"));

            let entry = root.join(format!("apps/desktop-files/yantrik-{stem}.desktop"));
            let text = std::fs::read_to_string(&entry)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", entry.display()));
            if let Some(name) = field(&text, "Name=") {
                if name != want {
                    wrong.push(format!("{stem}: .desktop says {name:?}, APP_NAMES says {want:?}"));
                }
            }

            let win = root.join(format!("apps/{stem}/ui/app.slint"));
            let text = std::fs::read_to_string(&win)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", win.display()));
            let title = text
                .lines()
                .find_map(|l| l.trim().strip_prefix("title: \""))
                .and_then(|r| r.split('"').next())
                .map(|s| s.to_string());
            if let Some(title) = title {
                if title != want {
                    wrong.push(format!("{stem}: window title {title:?}, APP_NAMES says {want:?}"));
                }
            }

            if display_name(id) != want {
                wrong.push(format!(
                    "{stem}: taskbar calls it {:?}, APP_NAMES says {want:?}",
                    display_name(id)
                ));
            }
        }

        assert!(
            wrong.is_empty(),
            "one application, more than one name:\n  {}\n\n\
             APP_NAMES in this file is the list. Change it there and change the .desktop entry \
             and the app's Window title to match.",
            wrong.join("\n  ")
        );
    }

    /// And one surface, declared where the name is: every app this OS ships says in its `.desktop`
    /// file which surface it publishes, and the id its window carries here is one of the names
    /// that surface answers to on the socket bus.
    ///
    /// The same four places as above, one step further. The .desktop entry is what lists an app
    /// while it is closed (`X-Yantrik-Surface`, `crate::surfaces`), so an entry without it is an
    /// app a mind cannot find until somebody opens it; and a window id that is not among the
    /// surface's names is a taskbar button whose app `describe` cannot reach by that word.
    #[test]
    fn every_shipped_app_declares_its_surface_under_the_names_the_shell_knows() {
        let root = repo_root();
        let mut wrong = Vec::new();
        for (stem, id) in STEM_TO_ID {
            if crate::wire::dock::shelved(stem).is_some() {
                continue;
            }
            let path = root.join(format!("apps/desktop-files/yantrik-{stem}.desktop"));
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            let entry = crate::apps::parse_desktop_text(&format!("yantrik-{stem}"), &text)
                .unwrap_or_else(|| panic!("{} is not an application entry", path.display()));
            let Some(surface) = entry.surface.as_deref() else {
                wrong.push(format!("{stem}: no X-Yantrik-Surface"));
                continue;
            };
            if entry.purpose.trim().is_empty() {
                wrong.push(format!("{stem}: no X-Yantrik-Purpose"));
            }
            if surface != *id && !entry.aliases.iter().any(|a| a == id) {
                wrong.push(format!(
                    "{stem}: its window is `{id}`, and `{surface}` does not answer to that \
                     (add it to X-Yantrik-Aliases)"
                ));
            }
        }
        assert!(wrong.is_empty(), "shipped apps that a mind cannot find by name:\n  {}", wrong.join("\n  "));
    }

    /// An app the shell did not launch still gets a readable name rather than an id.
    #[test]
    fn a_foreign_app_is_title_cased_not_left_raw() {
        assert_eq!(display_name("libreoffice-writer"), "Libreoffice writer");
        assert_eq!(display_name("chromium"), "Chromium");
    }

    /// The `name = "…"` values inside one `[section]` of a manifest, in order.
    fn names_in_section(text: &str, header: &str) -> Vec<String> {
        let mut in_section = false;
        let mut out = Vec::new();
        for line in text.lines() {
            if line.starts_with('[') {
                in_section = line == header;
                continue;
            }
            if !in_section {
                continue;
            }
            let Some(rest) = line.trim_start().strip_prefix("name") else { continue };
            let Some(rest) = rest.trim_start().strip_prefix('=') else { continue };
            let value = rest.trim().trim_start_matches('"');
            out.push(value.split('"').next().unwrap_or("").to_string());
        }
        out
    }

    /// The packaging scripts ask the tree which apps exist instead of writing a list down.
    ///
    /// deploy.sh carried a hand-written list of the apps it copies, its build step carried
    /// another, and scripts/publish-components.sh carried a third; nothing checked any of
    /// them against the workspace. Arcade merged, registered in every shell table, answered
    /// on its control surface — and `./deploy.sh` reported success on a machine without it,
    /// because no package built it and the copy loop skips a binary that is not there. The
    /// #45 shape again: the desktop offers to launch something the machine does not have.
    ///
    /// All three ask deploy/yantrik-os/app-bins.sh now, which reads the apps/ members of
    /// Cargo.toml. This is the guard the shell's tables have: it derives the same answer
    /// from the manifests, runs the reader, and fails when a script stops asking or starts
    /// naming apps by hand again — because a list nobody checks is a comment.
    #[test]
    fn the_packaging_scripts_ask_the_tree_which_apps_they_ship() {
        let root = repo_root();
        let deploy_dir = root.join("deploy/yantrik-os");
        if !deploy_dir.join("shelved-bins.sh").exists() {
            return; // Packaged source without the deploy tree; nothing to check against.
        }

        // The apps as the workspace defines them: every member under apps/, named by its
        // [[bin]] targets, or by its package when it leaves the target to cargo.
        let workspace =
            std::fs::read_to_string(root.join("Cargo.toml")).expect("cannot read Cargo.toml");
        let mut in_members = false;
        let mut members = Vec::new();
        for line in workspace.lines() {
            if line.starts_with("members = [") {
                in_members = true;
                continue;
            }
            if in_members && line.starts_with(']') {
                break;
            }
            if !in_members {
                continue;
            }
            let name = line.trim().trim_start_matches('"').split('"').next().unwrap_or("");
            if name.starts_with("apps/") {
                members.push(name.to_string());
            }
        }
        assert!(
            !members.is_empty(),
            "Cargo.toml lists no apps/ members in the shape this test reads"
        );

        let mut expected = Vec::new();
        for member in &members {
            let text = std::fs::read_to_string(root.join(member).join("Cargo.toml"))
                .unwrap_or_else(|e| panic!("cannot read {member}/Cargo.toml: {e}"));
            let mut bins = names_in_section(&text, "[[bin]]");
            if bins.is_empty() {
                bins = names_in_section(&text, "[package]");
            }
            assert!(!bins.is_empty(), "read no binary name out of {member}/Cargo.toml");
            expected.extend(bins);
        }
        expected.sort();

        // The reader the scripts ask agrees with the manifests this test just read.
        let script = deploy_dir.join("app-bins.sh");
        let Ok(out) = std::process::Command::new("bash").arg(&script).output() else {
            return; // No bash to ask; the text checks below still hold the scripts to it.
        };
        assert!(
            out.status.success(),
            "app-bins.sh failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let mut got: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .map(String::from)
            .collect();
        got.sort();
        assert_eq!(
            got, expected,
            "app-bins.sh and the workspace manifests disagree about which apps exist"
        );

        // And each script asks the reader and writes no name down. An app named literally in
        // a packaging script is a copy of the list, and a copy is what went stale.
        for rel in ["deploy.sh", "scripts/publish-components.sh"] {
            let text = std::fs::read_to_string(root.join(rel))
                .unwrap_or_else(|e| panic!("cannot read {rel}: {e}"));
            assert!(
                text.contains("app-bins.sh"),
                "{rel} no longer asks app-bins.sh which apps exist, so its list is \
                 hand-written again and the next app under apps/ is silently not shipped"
            );
            for bin in &expected {
                assert!(
                    !text.contains(bin.as_str()),
                    "{rel} writes {bin} down by hand — a name in the script is a copy that \
                     can go stale; ask deploy/yantrik-os/app-bins.sh instead"
                );
            }
        }
    }
}

#[cfg(test)]
mod restore_path_tests {
    /// Minimise any app, click it in the taskbar, and it used to come back MAXIMISED (#265):
    /// wlrctl 0.2.2's only verb that brings a minimized window back is `maximize`. The restore
    /// now goes over the foreign-toplevel protocol, whose `unset_minimized` leaves the size
    /// alone, and the `maximize` line only runs when the protocol is not offered.
    ///
    /// This reads `present` itself, because the fix is which path runs first: a test of the
    /// fallback command line alone would still pass with the fallback as the only path.
    #[test]
    fn presenting_a_window_un_minimises_over_the_protocol_and_only_maximises_as_a_fallback() {
        let source = include_str!("windows.rs");
        let start = source
            .find("pub fn present(title: &str)")
            .expect("the restore path every taskbar click and `show_app` goes through");
        let end = start + source[start..].find("\npub fn ").expect("present ends where present_app begins");
        let body = &source[start..end];

        let protocol = body
            .find("foreign_toplevel::restore(")
            .unwrap_or_else(|| panic!("present must un-minimize over the foreign-toplevel protocol:\n{body}"));
        let maximise = body
            .find("restore_command")
            .unwrap_or_else(|| panic!("the wlrctl fallback stays for a compositor without the protocol:\n{body}"));
        assert!(
            protocol < maximise,
            "the protocol is the restore path and `wlrctl maximize` the fallback, not the other \
             way round — maximizing is what brings the window back at the wrong size. \
             present as written:\n{body}"
        );
        let unavailable = body
            .find("Unavailable")
            .unwrap_or_else(|| panic!("the fallback runs when the protocol is unavailable:\n{body}"));
        assert!(
            unavailable < maximise,
            "the maximize line runs only on the Unavailable branch. present as written:\n{body}"
        );
    }
}
