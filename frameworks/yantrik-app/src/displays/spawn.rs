//! A Mind's launch that finds no instance: the instance is started on the person's display first,
//! with no window, and the Mind's launch becomes its view helper.
//!
//! The instance has to be on the person's display: winit can only ever open windows on the
//! display a process starts with, and the person's windows are the ones that must be native. A
//! process the shell started for a Mind is on the Mind's display, so it starts another copy of
//! the same binary with the person's display and without the marker, as the person's own launch
//! would have been started, told to start with no window ([`super::WINDOWLESS_VAR`]). That copy
//! claims the instance, and shows nothing until a launch asks: the Mind's, handed over the
//! moment it is up, or later the person's.
//!
//! The instance becomes the person's long-lived app, so it runs with the person's session, not
//! the Mind seat's (#972 L4): the variables that name a display and pin the toolkits are taken
//! from the shell that started this launch, its parent, as the shell's own launches have them
//! ([`persons_session`]). Its `WAYLAND_DISPLAY` is the person's display; when its socket is not in
//! the session's runtime directory there is no person's display to start on, and the Mind's
//! launch runs the app itself, drawn on the Mind's display and masked throughout, as before.

use std::path::{Path, PathBuf};

use crate::for_mind::{FOR_MIND_VAR, PERSON_DISPLAY};

/// What names a display and pins the toolkits to one: what the shell sets for a Mind's seat and
/// clears of the person's session.
pub const SESSION_VARS: [&str; 6] = ["WAYLAND_DISPLAY", "WAYLAND_SOCKET", "DISPLAY", "XAUTHORITY", "GDK_BACKEND", "QT_QPA_PLATFORM"];

/// The person's session: each of [`SESSION_VARS`] as the shell that started this launch has it
/// (`None`: it has none), read from its environment when it is this account's process. Its
/// display is the person's, [`PERSON_DISPLAY`] when it names none; a socket handed to one process
/// is never passed on.
pub fn persons_session() -> Vec<(&'static str, Option<String>)> {
    session_from(parent_environ().as_deref())
}

fn session_from(environ: Option<&[u8]>) -> Vec<(&'static str, Option<String>)> {
    let value = |key: &str| -> Option<String> {
        environ?.split(|b| *b == 0).find_map(|entry| {
            let entry = std::str::from_utf8(entry).ok()?;
            entry.strip_prefix(key)?.strip_prefix('=').filter(|v| !v.is_empty()).map(str::to_string)
        })
    };
    SESSION_VARS
        .iter()
        .map(|key| match *key {
            "WAYLAND_SOCKET" => (*key, None),
            "WAYLAND_DISPLAY" => (*key, Some(value(key).unwrap_or_else(|| PERSON_DISPLAY.to_string()))),
            _ => (*key, value(key)),
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn parent_environ() -> Option<Vec<u8>> {
    use std::os::unix::fs::MetadataExt;
    let parent = std::os::unix::process::parent_id();
    let proc_dir = PathBuf::from(format!("/proc/{parent}"));
    // SAFETY: geteuid cannot fail and touches no memory.
    let me = unsafe { libc::geteuid() };
    if std::fs::metadata(&proc_dir).ok()?.uid() != me {
        return None;
    }
    std::fs::read(proc_dir.join("environ")).ok()
}

#[cfg(not(target_os = "linux"))]
fn parent_environ() -> Option<Vec<u8>> {
    None
}

/// The person's display, when this session has one to start the instance on.
pub fn person_display(session: &[(&'static str, Option<String>)]) -> Option<String> {
    let runtime = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    let display = session.iter().find(|(k, _)| *k == "WAYLAND_DISPLAY").and_then(|(_, v)| v.clone())?;
    reachable(&runtime, &display).then_some(display)
}

/// Whether the Wayland socket `display` is in `runtime`, the session's runtime directory.
fn reachable(runtime: &Path, display: &str) -> bool {
    !display.contains('/') && is_socket(&runtime.join(display))
}

#[cfg(unix)]
fn is_socket(path: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::metadata(path).is_ok_and(|m| m.file_type().is_socket())
}

#[cfg(not(unix))]
fn is_socket(_path: &Path) -> bool {
    false
}

/// The command that starts the instance with no window: this program, with nothing on its
/// command line (what the Mind asked for is handed over by its launch), in the person's `session`,
/// and without the Mind's marker.
pub fn command(program: &Path, session: &[(&'static str, Option<String>)]) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    for (key, value) in session {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        };
    }
    command
        .env(super::WINDOWLESS_VAR, "1")
        .env_remove(FOR_MIND_VAR)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Not in the helper's process group: a Mind's window closing ends the helper, and the
        // person's windows of the instance live on.
        command.process_group(0);
    }
    command
}

/// Starts the instance with no window, in the person's `session`.
pub fn start_instance(session: &[(&'static str, Option<String>)]) -> std::io::Result<()> {
    let program = std::env::current_exe()?;
    let child = command(&program, session).spawn()?;
    let on = session.iter().find(|(k, _)| *k == "WAYLAND_DISPLAY").and_then(|(_, v)| v.clone()).unwrap_or_default();
    tracing::info!(pid = child.id(), display = on, "started the app's instance on the person's display, with no window");
    // Not waited for here: it outlives this launch. It is reaped by whoever inherits it.
    drop(child);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(command: &std::process::Command) -> Vec<(String, Option<String>)> {
        command
            .get_envs()
            .map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned())))
            .collect()
    }

    /// L4: the instance runs in the person's session as the shell has it, not the Mind seat's.
    #[test]
    fn the_instance_starts_in_the_persons_session_without_the_minds_marks() {
        let shell: Vec<u8> = ["WAYLAND_DISPLAY=wayland-0", "DISPLAY=:0", "XAUTHORITY=/home/p/.Xauthority", "WAYLAND_SOCKET=3", "HOME=/home/p"]
            .iter()
            .flat_map(|e| e.bytes().chain([0]))
            .collect();
        let session = session_from(Some(&shell));
        let command = command(Path::new("/opt/yantrik/bin/yantrik-notes"), &session);
        let env = env_of(&command);
        let of = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        assert_eq!(of("WAYLAND_DISPLAY"), Some(Some("wayland-0".to_string())));
        assert_eq!(of("DISPLAY"), Some(Some(":0".to_string())), "the person's X display, not the Mind's");
        assert_eq!(of("XAUTHORITY"), Some(Some("/home/p/.Xauthority".to_string())));
        assert_eq!(of("GDK_BACKEND"), Some(None), "the Mind seat's toolkit pin is not kept");
        assert_eq!(of("QT_QPA_PLATFORM"), Some(None));
        assert_eq!(of("WAYLAND_SOCKET"), Some(None), "a socket handed to one process is never passed on");
        assert_eq!(of(super::super::WINDOWLESS_VAR), Some(Some("1".into())));
        assert_eq!(of(FOR_MIND_VAR), Some(None), "the marker is removed, not set");
        assert_eq!(command.get_args().count(), 0, "what the Mind asked for is handed over, not passed");
    }

    #[test]
    fn with_no_shell_to_read_the_display_is_the_persons() {
        let session = session_from(None);
        assert!(session.contains(&("WAYLAND_DISPLAY", Some(PERSON_DISPLAY.to_string()))));
        assert!(session.contains(&("DISPLAY", None)));
    }

    #[cfg(unix)]
    #[test]
    fn a_display_is_reachable_only_as_a_socket_in_the_runtime_directory() {
        let dir = std::env::temp_dir().join(format!("yantrik-spawn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!reachable(&dir, "wayland-0"), "nothing there");
        std::fs::write(dir.join("wayland-0"), b"").unwrap();
        assert!(!reachable(&dir, "wayland-0"), "a file is not a display");
        std::fs::remove_file(dir.join("wayland-0")).unwrap();
        let _listener = std::os::unix::net::UnixListener::bind(dir.join("wayland-0")).unwrap();
        assert!(reachable(&dir, "wayland-0"));
        assert!(!reachable(&dir, "../wayland-0"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
