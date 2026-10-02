//! The agent terminal's visible half: a terminal in Mind View showing what a mind runs.
//!
//! `agent_run` starts a command in its own PTY inside the shell (`yantrik-agent-terminal`) and
//! the Agents screen draws its card, but nothing was drawn in Mind View, the desk the person
//! watches a mind work at ("Mind View is the actual desk", design/minds-surfaces-spec-2026-10-02.md).
//! On 2 Oct 2026 a mind ran commands for minutes and Mind View stayed empty.
//!
//! The command still runs once, in the agent terminal, so its exit code and caps are unchanged.
//! Its raw terminal bytes are also appended to a per-agent log in the shell's socket directory,
//! and a `foot` on Mind View's display follows that log with `tail -F`. Nothing a mind may do
//! changes: no new action, no new grade; the output goes to the mind's own desk and nowhere else.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use crate::mind_view::Seat;
use yantrik_companion::tools::browser::MindDisplay;

/// Past this a log is started afresh when its viewer is (re)opened.
const LOG_CAP: u64 = 4 * 1024 * 1024;

/// The most one command may add to its log. A `yes` or a build log cannot fill the runtime
/// directory's tmpfs while it runs; after this the view says so once and stops.
const RUN_CAP: u64 = 1024 * 1024;

/// One mirrored agent: its log, kept open, and what this command has written so far.
struct Mirror {
    log: PathBuf,
    file: std::fs::File,
    written: u64,
    capped: bool,
    clean: Cleaner,
}

#[derive(Default)]
struct State {
    /// Each mirrored agent's log, by agent id.
    logs: HashMap<String, Mirror>,
    /// The viewer showing it in Mind View, while it is alive.
    viewers: HashMap<String, Child>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut guard = STATE.lock().unwrap_or_else(|p| p.into_inner());
    f(guard.get_or_insert_with(State::default))
}

/// Turns what a command printed into what the person may be shown as "the actual desk": text and
/// colours only. A mind's command can print `\r`, `ESC c`, `ESC [ 2 J`, `ESC [ 3 J` or an OSC
/// string, which would overwrite the line that ran, wipe the history or retitle the window, so
/// the view showed something other than what happened (PR #582 review, S4). Only SGR (colour)
/// sequences pass; every other escape is dropped, with a carriage return that is not part of a
/// line end. It keeps its place between chunks, since a PTY splits sequences anywhere.
#[derive(Default)]
struct Cleaner {
    state: CleanState,
}

#[derive(Default, PartialEq)]
enum CleanState {
    #[default]
    Text,
    Esc,
    /// Inside `ESC [`, with the bytes so far, kept only while they could still be an SGR.
    Csi(Vec<u8>),
    /// Inside a string (`ESC ]`, `ESC P`, ...), until BEL or `ESC \`.
    Str { esc: bool },
}

impl Cleaner {
    fn clean(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(bytes.len());
        for &b in bytes {
            let state = std::mem::take(&mut self.state);
            self.state = match state {
                CleanState::Text => match b {
                    0x1b => CleanState::Esc,
                    b'\n' | b'\t' => {
                        out.push(b);
                        CleanState::Text
                    }
                    // Other C0 controls (CR, BS, BEL, ...) and DEL: dropped.
                    0x00..=0x1f | 0x7f => CleanState::Text,
                    _ => {
                        out.push(b);
                        CleanState::Text
                    }
                },
                CleanState::Esc => match b {
                    b'[' => CleanState::Csi(Vec::new()),
                    b']' | b'P' | b'X' | b'^' | b'_' => CleanState::Str { esc: false },
                    // `ESC c` and every other two-byte escape: dropped.
                    _ => CleanState::Text,
                },
                CleanState::Csi(mut params) => match b {
                    b'0'..=b'9' | b';' | b':' if params.len() < 64 => {
                        params.push(b);
                        CleanState::Csi(params)
                    }
                    b'm' => {
                        out.extend_from_slice(b"\x1b[");
                        out.extend_from_slice(&params);
                        out.push(b'm');
                        CleanState::Text
                    }
                    // Any other final byte (J, H, K, ...) or an over-long or odd sequence: dropped.
                    0x40..=0x7e => CleanState::Text,
                    _ if params.len() < 64 => CleanState::Csi(params),
                    _ => CleanState::Text,
                },
                CleanState::Str { esc } => match (esc, b) {
                    (_, 0x07) | (true, b'\\') => CleanState::Text,
                    (_, 0x1b) => CleanState::Str { esc: true },
                    _ => CleanState::Str { esc: false },
                },
            };
        }
        out
    }
}

/// A command as shown on its `$` line: control characters in caret form, so a command that
/// contains `\r` or `ESC` cannot rewrite what the line says.
fn show_command(command: &str) -> String {
    command
        .chars()
        .map(|c| match c {
            '\n' => "\\n".to_string(),
            c if (c as u32) < 0x20 => format!("^{}", ((c as u8) ^ 0x40) as char),
            '\u{7f}' => "^?".to_string(),
            c if ('\u{80}'..='\u{9f}').contains(&c) => "?".to_string(),
            c => c.to_string(),
        })
        .collect()
}

/// A file name for an agent id: letters, digits, `-` and `_`, so no id can reach outside the
/// directory. An id that had to be changed (`pi.1`) gets a short hash of the original, so it
/// cannot share a log and a window title with the id it was changed into (`pi_1`).
fn file_stem(agent: &str) -> String {
    let stem: String = agent
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(64)
        .collect();
    let stem = if stem.is_empty() { "agent".to_string() } else { stem };
    if stem == agent {
        return stem;
    }
    // FNV-1a over the original id.
    let hash = agent.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    format!("{stem}-{hash:08x}")
}

/// Where `agent`'s commands are mirrored.
pub fn log_path(dir: &Path, agent: &str) -> PathBuf {
    dir.join("agent-terminal").join(format!("{}.log", file_stem(agent)))
}

/// The title of the viewer window, which is also how it is found in Mind View's window list.
pub fn title(agent: &str) -> String {
    format!("{} terminal", file_stem(agent))
}

/// What to run, and with which environment, to show `log` on `seat`. Pure, so the one property
/// that matters — it draws on Mind View and never on the person's display — is tested.
pub struct Spec {
    pub args: Vec<String>,
    /// Applied to the command: the person's session variables cleared, then the seat's set.
    pub display: MindDisplay,
    pub env_remove: Vec<&'static str>,
}

pub fn spec(seat: &Seat, log: &Path, agent: &str) -> Spec {
    Spec {
        args: vec![
            "-T".into(),
            title(agent),
            "-e".into(),
            "tail".into(),
            "-n".into(),
            "+1".into(),
            "-F".into(),
            log.display().to_string(),
        ],
        display: seat.display(),
        env_remove: vec!["SLINT_FULLSCREEN"],
    }
}

/// What starting a command's mirror came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Begin {
    /// Mirroring is off (the person's setting).
    Off,
    /// It is on and the log could not be written.
    Failed(String),
    /// The command's line is in the log, at this path.
    Log(PathBuf),
}

/// Start a command's mirror: its line in the log, in this agent's own file. Cheap, and done
/// before the command starts so the first bytes of its output cannot arrive ahead of it.
pub fn begin(agent: &str, command: &str) -> Begin {
    if !crate::wire::settings::minds_open_in_mind_view() {
        // Off: an agent mirrored earlier stops being written to.
        with_state(|state| state.logs.remove(agent));
        return Begin::Off;
    }
    let log = log_path(&yantrik_ipc_transport::server::socket_dir(), agent);
    match open_log(&log) {
        Ok(file) => begin_in(agent, command, log, file),
        Err(e) => Begin::Failed(format!("the log {} could not be written: {e}", log.display())),
    }
}

fn open_log(log: &Path) -> std::io::Result<std::fs::File> {
    let dir = log.parent().ok_or_else(|| std::io::Error::other("no directory"))?;
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    if std::fs::metadata(log).is_ok_and(|m| m.len() > LOG_CAP) {
        let _ = std::fs::remove_file(log);
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Private from the first byte, not chmod-ed after a window in which it is not.
        options.mode(0o600);
    }
    options.open(log)
}

fn begin_in(agent: &str, command: &str, log: PathBuf, mut file: std::fs::File) -> Begin {
    let line = format!("\x1b[1;36m$ {}\x1b[0m\n", show_command(command));
    if let Err(e) = file.write_all(line.as_bytes()) {
        return Begin::Failed(format!("the log {} could not be written: {e}", log.display()));
    }
    let written = line.len() as u64;
    with_state(|state| {
        state.logs.insert(
            agent.to_string(),
            Mirror { log: log.clone(), file, written, capped: false, clean: Cleaner::default() },
        )
    });
    Begin::Log(log)
}

/// The command did not start after all (the agent terminal refused it): say so in the view
/// instead of leaving a `$` line that looks as though it ran.
pub fn abandon(agent: &str) {
    mirror(agent, b"\x1b[31m(not started)\x1b[0m\n");
}

/// A mirrored agent's terminal bytes, as they come off its PTY, cleaned to text and colours (see
/// [`Cleaner`]) and held to [`RUN_CAP`] per command. Called for every job's output; does nothing
/// for an agent that is not being mirrored.
pub fn mirror(agent: &str, bytes: &[u8]) {
    with_state(|state| {
        let Some(m) = state.logs.get_mut(agent) else { return };
        if m.capped {
            return;
        }
        let mut clean = m.clean.clean(bytes);
        if m.written + clean.len() as u64 > RUN_CAP {
            let room = RUN_CAP.saturating_sub(m.written) as usize;
            clean.truncate(room.min(clean.len()));
            clean.extend_from_slice(b"\n\x1b[33m[more output not shown here; it is in the answer and on the Agents screen]\x1b[0m\n");
            m.capped = true;
        }
        m.written += clean.len() as u64;
        let _ = m.file.write_all(&clean);
    });
}

/// What `agent_run` says about where the command can be watched.
#[derive(Debug, PartialEq, Eq)]
pub enum Shown {
    /// A terminal titled so is open in Mind View, which is on `display`.
    InMindView { title: String, display: String },
    /// Being opened on a worker, so the command's answer is not held up behind Mind View's start.
    Starting { title: String },
    /// Not drawn in Mind View, and why. The command still ran, in the agent terminal.
    Not(String),
}

/// Make sure a viewer for `agent` is open in Mind View, starting Mind View and the viewer if need
/// be, and read back whether its window is listed there. Waits, so off the UI thread only.
pub fn show(agent: &str, log: &Path) -> Shown {
    // The first start of Mind View maps a window on the person's desktop, which comes up over a
    // waiting card: refused while one waits, like every action that opens a window.
    if let Err(why) = crate::card_watch::hold_windows("agent_run") {
        return Shown::Not(why);
    }
    let seat = match crate::mind_view::ensure() {
        Ok(seat) => seat,
        Err(why) => return Shown::Not(format!("Mind View is not available: {why}")),
    };
    let alive = with_state(|state| {
        state.viewers.get_mut(agent).is_some_and(|c| matches!(c.try_wait(), Ok(None)))
    });
    if !alive {
        let Some(foot) = crate::wire::dock::find_program("foot") else {
            return Shown::Not("`foot` is not installed, so there is no terminal to show it in".into());
        };
        let spec = spec(&seat, log, agent);
        let mut command = Command::new(foot);
        command.args(&spec.args);
        for key in &spec.env_remove {
            command.env_remove(key);
        }
        spec.display.apply(&mut command);
        match command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            Ok(child) => {
                let pid = child.id();
                crate::mind_view::mark_launched("agent-terminal", pid);
                with_state(|state| state.viewers.insert(agent.to_string(), child));
                reap_later(agent.to_string(), pid);
            }
            Err(e) => return Shown::Not(format!("could not start foot: {e}")),
        }
    }
    let title = title(agent);
    let mut probe = || match crate::mind_view::nested_window_lines() {
        Some(lines) if lines.iter().any(|l| l.contains(&title)) => {
            crate::mind_landing::Seen::Window(crate::mind_landing::Place::MindView)
        }
        _ => crate::mind_landing::Seen::Nothing,
    };
    match crate::mind_landing::wait_for_window(
        &mut probe,
        std::time::Duration::from_secs(4),
        std::time::Duration::from_millis(250),
    ) {
        Ok(_) => Shown::InMindView { title: title.clone(), display: seat.wayland },
        Err(_) => Shown::Not(format!("no window titled `{title}` was listed in Mind View within 4 s")),
    }
}

/// [`show`] on a worker of its own: `agent_run` answers at once with where to look, and the
/// command's `wait` does not start counting after Mind View has started.
pub fn show_later(agent: &str, log: PathBuf) -> Shown {
    let title = title(agent);
    let agent = agent.to_string();
    let spawned = std::thread::Builder::new().name("yos-agent-viewer".into()).spawn(move || {
        let shown = show(&agent, &log);
        if let Shown::Not(why) = &shown {
            tracing::info!(%agent, %why, "agent terminal not shown in Mind View");
        }
    });
    match spawned {
        Ok(_) => Shown::Starting { title },
        Err(e) => Shown::Not(format!("could not start the viewer's worker: {e}")),
    }
}

/// Forget the viewer when it exits, so the next command opens a fresh one.
fn reap_later(agent: String, pid: u32) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let gone = with_state(|state| match state.viewers.get_mut(&agent) {
            Some(child) if child.id() == pid => !matches!(child.try_wait(), Ok(None)),
            _ => true,
        });
        if gone {
            crate::mind_view::mark_exited(pid);
            with_state(|state| {
                if state.viewers.get(&agent).is_some_and(|c| c.id() == pid) {
                    state.viewers.remove(&agent);
                }
            });
            return;
        }
    });
}

/// The words `agent_run` adds to its answer.
pub fn answer_fields(shown: &Shown) -> serde_json::Value {
    match shown {
        Shown::InMindView { title, display } => serde_json::json!({
            "shown_in": "Mind View",
            "terminal_window": title,
            "mind_view_display": display,
        }),
        Shown::Starting { title } => serde_json::json!({
            "shown_in": "Mind View, starting",
            "terminal_window": title,
            "shown_note": "the terminal is being opened in Mind View; the command is already running and its output is in this answer",
        }),
        Shown::Not(why) => serde_json::json!({
            "shown_in": "nowhere on screen",
            "shown_note": format!("{why}. The command ran in the agent terminal; its output is in this answer and on the Agents screen."),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seat(x11: Option<&str>) -> Seat {
        Seat { wayland: "wayland-1".into(), x11: x11.map(str::to_string) }
    }

    fn command_for(spec: &Spec) -> Command {
        let mut command = Command::new("foot");
        for key in &spec.env_remove {
            command.env_remove(key);
        }
        spec.display.apply(&mut command);
        command
    }

    fn env_of(command: &Command, key: &str) -> Option<Option<String>> {
        command
            .get_envs()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
    }

    /// The point of the module: the viewer draws on Mind View's display and never the person's.
    #[test]
    fn the_viewer_is_started_on_mind_views_display_and_never_the_persons() {
        let s = spec(&seat(None), Path::new("/run/user/1000/yantrik/agent-terminal/hermes.log"), "hermes");
        let command = command_for(&s);
        assert_eq!(env_of(&command, "WAYLAND_DISPLAY"), Some(Some("wayland-1".to_string())));
        // Mind View without an Xwayland of its own: the person's DISPLAY is stripped, not kept.
        assert_eq!(env_of(&command, "DISPLAY"), Some(None));
        assert_eq!(env_of(&command, "WAYLAND_SOCKET"), Some(None));
        assert_eq!(env_of(&command, "XAUTHORITY"), Some(None));
        let with_x = command_for(&spec(&seat(Some(":2")), Path::new("/x.log"), "hermes"));
        assert_eq!(env_of(&with_x, "DISPLAY"), Some(Some(":2".to_string())));
    }

    #[test]
    fn the_viewer_follows_the_log_from_its_first_line() {
        let s = spec(&seat(None), Path::new("/x/hermes.log"), "hermes");
        assert_eq!(s.args[..2], ["-T".to_string(), "hermes terminal".to_string()]);
        assert_eq!(s.args[2..], ["-e", "tail", "-n", "+1", "-F", "/x/hermes.log"]);
    }

    #[test]
    fn an_agent_id_cannot_name_a_file_outside_the_directory() {
        let p = log_path(Path::new("/run/u/yantrik"), "../../etc/passwd");
        assert_eq!(p.parent().unwrap(), Path::new("/run/u/yantrik/agent-terminal"));
        assert!(!p.display().to_string().contains(".."));
        assert!(file_stem("").starts_with("agent"));
    }

    /// `pi.1` and `pi_1` used to share one log and one window title, so two agents' commands
    /// interleaved in one viewer and the title probe could match the other's window (S9).
    #[test]
    fn two_agent_ids_that_sanitise_alike_do_not_share_a_log_or_a_title() {
        assert_eq!(file_stem("pi_1"), "pi_1");
        assert_ne!(file_stem("pi.1"), file_stem("pi_1"));
        assert_ne!(title("pi.1"), title("pi_1"));
        assert_ne!(log_path(Path::new("/d"), "pi.1"), log_path(Path::new("/d"), "pi_1"));
        assert!(file_stem("pi.1").starts_with("pi_1-"));
    }

    fn mirrored(agent: &str, command: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("yos-mat-{}-{agent}", std::process::id()));
        let log = log_path(&dir, agent);
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        let file = open_log(&log).unwrap();
        assert_eq!(begin_in(agent, command, log.clone(), file), Begin::Log(log.clone()));
        (dir, log)
    }

    #[test]
    fn mirror_writes_only_for_an_agent_that_began_and_say_so_in_the_log() {
        let dir = std::env::temp_dir().join(format!("yos-mat0-{}", std::process::id()));
        let log = log_path(&dir, "never-began");
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        mirror("never-began", b"ignored");
        assert!(!log.exists(), "an agent that never began is not mirrored");
        let (dir, log) = mirrored("t-agent", "ls");
        mirror("t-agent", b"hello\r\n");
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.contains("$ ls"), "{text}");
        assert!(text.ends_with("hello\n"), "a line end is kept, a bare CR is not: {text:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_log_is_private_from_its_first_byte() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, log) = mirrored("t-mode", "true");
        assert_eq!(std::fs::metadata(&log).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// S4: a command that prints what rewrites the screen must not rewrite the person's view.
    #[test]
    fn output_that_clears_or_rewrites_the_view_is_neutralised() {
        let mut c = Cleaner::default();
        let hostile = b"real\r\x1b[2KFAKE\x1b[3J\x1bc\x1b]0;evil\x07\x1b[H\x08ok\x1b[1;31mred\x1b[0m\n";
        let shown = String::from_utf8(c.clean(hostile)).unwrap();
        assert_eq!(shown, "realFAKEok\x1b[1;31mred\x1b[0m\n");
        assert!(!shown.contains('\r') && !shown.contains("\x1b[2K") && !shown.contains("\x1b[3J"));
        assert!(!shown.contains("\x1bc") && !shown.contains("evil"));
    }

    #[test]
    fn a_sequence_split_across_chunks_is_still_neutralised() {
        let mut c = Cleaner::default();
        let mut shown = c.clean(b"a\x1b[3");
        shown.extend(c.clean(b"Jb\x1b]0;ti"));
        shown.extend(c.clean(b"tle\x07c"));
        assert_eq!(String::from_utf8(shown).unwrap(), "abc");
    }

    #[test]
    fn the_command_line_cannot_rewrite_itself() {
        let shown = show_command("echo hi\r\x1b[2K$ ls\x1bc");
        assert_eq!(shown, "echo hi^M^[[2K$ ls^[c");
        let (dir, log) = mirrored("t-spoof", "a\rb\x1b[3J");
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.contains("$ a^Mb^[[3J"), "{text:?}");
        assert!(!text.contains('\r'));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// S8: one long command cannot fill the runtime directory.
    #[test]
    fn one_command_cannot_fill_the_runtime_directory() {
        let (dir, log) = mirrored("t-cap", "yes");
        let chunk = vec![b'y'; 64 * 1024];
        for _ in 0..64 {
            mirror("t-cap", &chunk);
        }
        let size = std::fs::metadata(&log).unwrap().len();
        assert!(size < RUN_CAP + 1024, "{size}");
        assert!(std::fs::read_to_string(&log).unwrap().contains("more output not shown"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_command_that_did_not_start_is_not_left_looking_as_though_it_ran() {
        let (dir, log) = mirrored("t-abandon", "rm -rf /tmp/x");
        abandon("t-abandon");
        assert!(std::fs::read_to_string(&log).unwrap().contains("(not started)"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_command_not_shown_says_so() {
        let v = answer_fields(&Shown::Not("Mind View is not available: no labwc".into()));
        assert_eq!(v["shown_in"], "nowhere on screen");
        assert!(v["shown_note"].as_str().unwrap().contains("agent terminal"));
        let v = answer_fields(&Shown::Starting { title: "hermes terminal".into() });
        assert_eq!(v["shown_in"], "Mind View, starting");
    }

    /// S7: the viewer never opens a window while a card waits, and the answer is not held behind it.
    #[test]
    fn the_viewer_is_held_while_a_card_waits_and_opens_off_the_answers_path() {
        let src = include_str!("mind_agent_terminal.rs");
        let show_fn = &src[src.find("pub fn show(").unwrap()..];
        let hold = show_fn.find("hold_windows(\"agent_run\")").expect("show must call hold_windows first");
        let ensure = show_fn.find("ensure()").unwrap();
        assert!(hold < ensure, "hold_windows must come before Mind View is started");
        let run = include_str!("control_agent_terminal.rs");
        assert!(run.contains("show_later("), "agent_run must not wait on show()");
        assert!(!run.contains("mind_agent_terminal::show("), "agent_run must not wait on show()");
    }
}
