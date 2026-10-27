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
//! The person's display is [`PERSON_DISPLAY`], as #959's rule takes it. When its socket is not in
//! the session's runtime directory there is no person's display to start on, and the Mind's
//! launch runs the app itself, drawn on the Mind's display and masked throughout, as before.

use std::path::{Path, PathBuf};

use crate::for_mind::{FOR_MIND_VAR, PERSON_DISPLAY};

/// The person's display, when this session has one to start the instance on.
pub fn person_display() -> Option<String> {
    let runtime = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    reachable(&runtime, PERSON_DISPLAY).then(|| PERSON_DISPLAY.to_string())
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

/// The command that starts the instance on `display` with no window: this program, with nothing
/// on its command line (what the Mind asked for is handed over by its launch), the person's
/// display, and none of what names the Mind's.
pub fn command(program: &Path, display: &str) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    command
        .env("WAYLAND_DISPLAY", display)
        .env(super::WINDOWLESS_VAR, "1")
        // Not a Mind's launch: no marker, and neither the Mind's own Xwayland nor a socket
        // handed to the Mind's process.
        .env_remove(FOR_MIND_VAR)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_SOCKET")
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

/// Starts the instance on `display` with no window.
pub fn start_instance(display: &str) -> std::io::Result<()> {
    let program = std::env::current_exe()?;
    let child = command(&program, display).spawn()?;
    let on = display;
    tracing::info!(pid = child.id(), display = on, "started the app's instance on the person's display, with no window");
    // Not waited for here: it outlives this launch. It is reaped by whoever inherits it.
    drop(child);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_instance_starts_on_the_persons_display_without_the_minds_marks() {
        let command = command(Path::new("/opt/yantrik/bin/yantrik-notes"), PERSON_DISPLAY);
        let env: Vec<(String, Option<String>)> = command
            .get_envs()
            .map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned())))
            .collect();
        let of = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        assert_eq!(of("WAYLAND_DISPLAY"), Some(Some(PERSON_DISPLAY.to_string())));
        assert_eq!(of(super::super::WINDOWLESS_VAR), Some(Some("1".into())));
        assert_eq!(of(FOR_MIND_VAR), Some(None), "the marker is removed, not set");
        assert_eq!(of("DISPLAY"), Some(None), "not the Mind's Xwayland");
        assert_eq!(command.get_args().count(), 0, "what the Mind asked for is handed over, not passed");
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
