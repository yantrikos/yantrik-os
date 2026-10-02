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

/// Past this a log is started afresh when its viewer is (re)opened, so a long session cannot fill
/// the runtime directory.
const LOG_CAP: u64 = 4 * 1024 * 1024;

#[derive(Default)]
struct State {
    /// Each mirrored agent's log, by agent id.
    logs: HashMap<String, PathBuf>,
    /// The viewer showing it in Mind View, while it is alive.
    viewers: HashMap<String, Child>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut guard = STATE.lock().unwrap_or_else(|p| p.into_inner());
    f(guard.get_or_insert_with(State::default))
}

/// A file name for an agent id: letters, digits, `-` and `_`, so no id can reach outside the
/// directory.
fn file_stem(agent: &str) -> String {
    let stem: String = agent
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(64)
        .collect();
    if stem.is_empty() { "agent".to_string() } else { stem }
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
    pub env: Vec<(&'static str, String)>,
    pub env_remove: Vec<&'static str>,
}

pub fn spec(seat: &Seat, log: &Path, agent: &str) -> Spec {
    let mut env = seat.env();
    let mut env_remove = vec!["SLINT_FULLSCREEN", "WAYLAND_SOCKET"];
    if seat.x11.is_none() {
        // The person's Xwayland must not carry the viewer over to their desktop.
        env_remove.push("DISPLAY");
    }
    env.sort_by_key(|(k, _)| *k);
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
        env,
        env_remove,
    }
}

/// Start a command's mirror: its line in the log, in this agent's own file. Cheap, and done
/// before the command starts so the first bytes of its output cannot arrive ahead of it. Returns
/// the log's path, or `None` when mirroring is off or the file cannot be written.
pub fn begin(agent: &str, command: &str) -> Option<PathBuf> {
    if !crate::wire::settings::minds_open_in_mind_view() {
        return None;
    }
    let log = log_path(&yantrik_ipc_transport::server::socket_dir(), agent);
    let dir = log.parent()?;
    std::fs::create_dir_all(dir).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    if std::fs::metadata(&log).is_ok_and(|m| m.len() > LOG_CAP) {
        let _ = std::fs::remove_file(&log);
    }
    append(&log, format!("\x1b[1;36m$ {command}\x1b[0m\r\n").as_bytes())?;
    with_state(|state| state.logs.insert(agent.to_string(), log.clone()));
    Some(log)
}

/// A mirrored agent's terminal bytes, as they come off its PTY. Called for every job's output;
/// does nothing for an agent that is not being mirrored.
pub fn mirror(agent: &str, bytes: &[u8]) {
    if let Some(log) = with_state(|state| state.logs.get(agent).cloned()) {
        let _ = append(&log, bytes);
    }
}

fn append(log: &Path, bytes: &[u8]) -> Option<()> {
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(log).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(log, std::fs::Permissions::from_mode(0o600));
    }
    file.write_all(bytes).ok()
}

/// What `agent_run` says about where the command can be watched.
#[derive(Debug, PartialEq, Eq)]
pub enum Shown {
    /// A terminal titled so is open in Mind View, which is on `display`.
    InMindView { title: String, display: String },
    /// Not drawn in Mind View, and why. The command still ran, in the agent terminal.
    Not(String),
}

/// Make sure a viewer for `agent` is open in Mind View, starting Mind View and the viewer if need
/// be, and read back whether its window is listed there. Waits, so off the UI thread only.
pub fn show(agent: &str, log: &Path) -> Shown {
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
        for (key, value) in &spec.env {
            command.env(key, value);
        }
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
        Err(why) => Shown::Not(format!("the terminal in Mind View did not show up: {why}")),
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

    /// The point of the module: the viewer draws on Mind View's display and never the person's.
    #[test]
    fn the_viewer_is_started_on_mind_views_display_and_never_the_persons() {
        let s = spec(&seat(None), Path::new("/run/user/1000/yantrik/agent-terminal/hermes.log"), "hermes");
        assert!(s.env.contains(&("WAYLAND_DISPLAY", "wayland-1".to_string())));
        assert!(!s.env.iter().any(|(_, v)| v == "wayland-0"));
        // Mind View without an Xwayland of its own: the person's DISPLAY is stripped, not kept.
        assert!(s.env_remove.contains(&"DISPLAY"));
        assert!(s.env_remove.contains(&"WAYLAND_SOCKET"));
        let with_x = spec(&seat(Some(":2")), Path::new("/x.log"), "hermes");
        assert!(with_x.env.contains(&("DISPLAY", ":2".to_string())));
        assert!(!with_x.env_remove.contains(&"DISPLAY"));
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
        assert_eq!(file_stem(""), "agent");
    }

    #[test]
    fn mirror_writes_only_for_an_agent_that_began_and_say_so_in_the_log() {
        let dir = std::env::temp_dir().join(format!("yos-mat-{}", std::process::id()));
        let log = log_path(&dir, "t-agent");
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        mirror("t-agent", b"ignored");
        assert!(!log.exists(), "an agent that never began is not mirrored");
        with_state(|s| s.logs.insert("t-agent".into(), log.clone()));
        mirror("t-agent", b"hello\r\n");
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "hello\r\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_command_not_shown_says_so() {
        let v = answer_fields(&Shown::Not("Mind View is not available: no labwc".into()));
        assert_eq!(v["shown_in"], "nowhere on screen");
        assert!(v["shown_note"].as_str().unwrap().contains("agent terminal"));
    }
}
