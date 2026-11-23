//! The seat script's test, kept out of mind_view.rs (over the size limit, #1347).

use super::*;

/// Mind View's size is set before the seat is announced, so that nothing the shell opens in it
/// meets the default size first (#1347).
#[cfg(unix)]
#[test]
fn the_nested_output_is_resized_before_the_seat_is_announced() {
    let dir = std::env::temp_dir().join(format!("mind-view-size-{}", std::process::id()));
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let file = seat_file(&dir);
    let log = dir.join("log");
    let fake = bin.join("wlr-randr");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nif [ \"$1\" = --output ]; then echo \"$* seat_written=$([ -e '{}' ] && echo yes || echo no)\" >> '{}'; exit 0; fi\nprintf 'HDMI-A-1\\n  1280x800 px, 60.0 Hz (preferred, current)\\n  Scale: 1.000000\\n'\n",
            file.display(),
            log.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let script = dir.join("seat.sh");
    std::fs::write(&script, seat_script(&file, Path::new(CHECKOUT_CONFIG), "wayland-0")).unwrap();
    let status = Command::new("sh")
        .arg(&script)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("WAYLAND_DISPLAY", "wayland-7")
        .status()
        .unwrap();
    assert!(status.success());
    let said = std::fs::read_to_string(&log).expect("the output was resized by the time the script ended");
    assert!(said.contains("--custom-mode 422x400"), "a third wide, half high: {said}");
    assert!(said.contains("seat_written=no"), "resized first: {said}");
    assert!(file.exists(), "and then announced");
    let _ = std::fs::remove_dir_all(&dir);
}
