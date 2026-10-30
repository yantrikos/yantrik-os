//! Starting the Mind desk inside Mind View (#1023, phase 1).
//!
//! The desk is `apps/mind-desk`: a framework app whose two windows are the bars around Mind View
//! (a header and a footer). It runs on the nested compositor's display, so the nested labwc
//! starts it from its startup script, once that compositor is serving, with `WAYLAND_DISPLAY`
//! already pointing there. This is the script's part; [`config/labwc-mind/rc.xml`] is where the
//! windows are placed and the margins that keep every other window out of them, and
//! `apps/mind-desk` holds the numbers the two must agree on.
//!
//! The script starts it again if it ends, a few times, with a pause: the desk is chrome, and a
//! Mind that closes it (Alt+F4 is one of the two keys the nested compositor binds) should not
//! leave Mind View without its bars. The desk also refuses a close request of its own, so this
//! is for a crash. After the last try it stays gone, and the margins it kept are an empty
//! strip: a Mind View without a desk is as it was, with two bands of its own colour.

use std::path::Path;

/// The desk's program, as installed beside the shell.
pub const PROGRAM: &str = "yantrik-mind-desk";

/// How many times the script starts the desk, and the pause between.
pub const TRIES: u32 = 5;
pub const PAUSE_SECS: u32 = 2;

/// The lines the nested labwc's startup script runs to start the desk, or nothing when there is no
/// desk to start. `program` is the desk's absolute path.
///
/// The marker says the process is drawn for a Mind (`FOR_MIND_VAR`), though its display already
/// says so: the marker cannot be lost on the way, and the desk's one rule is that it is a Mind's.
pub fn launch_lines(program: Option<&Path>) -> String {
    let Some(program) = program else { return String::new() };
    let path = program.display().to_string();
    // A path with a quote in it is not one this script can carry, and not one the OS installs.
    if path.contains('\'') || path.contains('\n') {
        return String::new();
    }
    format!(
        "# The desk: the bars around the apps (#1023). Started again if it ends, a few times.\n\
         (for _try in $(seq 1 {TRIES}); do {marker}=1 '{path}' >/dev/null 2>&1; sleep {PAUSE_SECS}; done) &\n",
        marker = yantrik_ipc_contracts::mind_view::FOR_MIND_VAR,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desk_is_started_on_the_nested_display_as_a_minds_and_again_if_it_ends() {
        let lines = launch_lines(Some(Path::new("/opt/yantrik/bin/yantrik-mind-desk")));
        assert!(lines.contains("'/opt/yantrik/bin/yantrik-mind-desk'"), "{lines}");
        assert!(lines.contains(&format!("{}=1 ", yantrik_ipc_contracts::mind_view::FOR_MIND_VAR)), "{lines}");
        assert!(lines.contains("seq 1 5") && lines.contains("sleep 2"), "bounded, with a pause: {lines}");
        assert!(lines.trim_end().ends_with('&'), "in the background, so the script goes on: {lines}");
        // It names no display: the nested labwc's own is in the environment it runs the script in.
        assert!(!lines.contains("WAYLAND_DISPLAY"), "{lines}");
    }

    #[test]
    fn no_desk_to_start_is_no_lines_and_a_path_that_cannot_be_quoted_is_not_run() {
        assert_eq!(launch_lines(None), "");
        assert_eq!(launch_lines(Some(Path::new("/opt/it's/desk"))), "");
    }

    /// The script is `sh`, and what it starts with is the shell's own; the lines parse as `sh`.
    #[cfg(unix)]
    #[test]
    fn the_lines_are_valid_sh() {
        let lines = launch_lines(Some(Path::new("/bin/true")));
        let status = std::process::Command::new("sh").arg("-n").arg("-c").arg(&lines).status().unwrap();
        assert!(status.success(), "{lines}");
    }
}
