//! The screen's backlight: read from sysfs, set through `brightnessctl` or logind.
//!
//! A desktop monitor or a VM has no backlight, and the shell must not draw a brightness control
//! for it: a slider that moves nothing, at a level nobody measured, is a lie. `available()` is
//! the one question every caller asks first.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SYSFS_ROOT: &str = "/sys/class/backlight";

/// The lowest level the shell sets. 0% is a black screen on most panels, and a person who
/// dragged a slider to the end would have no way to see the slider again.
const FLOOR_PCT: u8 = 1;

/// One backlight device, as sysfs reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub brightness: u32,
    pub max: u32,
}

impl Device {
    /// The level as a whole percent, 0..=100.
    pub fn percent(&self) -> u8 {
        ((u64::from(self.brightness) * 100 + u64::from(self.max) / 2) / u64::from(self.max)).min(100) as u8
    }
}

fn read_number(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// The first usable backlight under `root` (devices in name order; one with no range is not
/// usable). `root` is a parameter so tests can point it at a fake sysfs.
pub fn device_in(root: &Path) -> Option<Device> {
    let mut names: Vec<PathBuf> = std::fs::read_dir(root).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    names.sort();
    names.into_iter().find_map(|dir| {
        let max = read_number(&dir.join("max_brightness")).filter(|m| *m > 0)?;
        let brightness = read_number(&dir.join("brightness"))?.min(max);
        Some(Device { name: dir.file_name()?.to_string_lossy().into_owned(), brightness, max })
    })
}

/// The percent the backlight under `root` is at, or `None` when there is none.
pub fn read_in(root: &Path) -> Option<u8> {
    device_in(root).map(|d| d.percent())
}

/// The machine's backlight level in percent, or `None` when it has no backlight.
pub fn read() -> Option<u8> {
    read_in(Path::new(SYSFS_ROOT))
}

/// Whether this machine has a backlight to control.
pub fn available() -> bool {
    device_in(Path::new(SYSFS_ROOT)).is_some()
}

/// The raw value logind is asked for: the percent of the device's own range, never 0.
pub fn raw_for(pct: u8, max: u32) -> u32 {
    let pct = u64::from(pct.clamp(FLOOR_PCT, 100));
    (((pct * u64::from(max)) + 50) / 100).max(1) as u32
}

/// Set the backlight to `pct` (clamped to 1..=100): `brightnessctl` when it is installed and
/// willing, otherwise logind's `SetBrightness`, which the logged-in session may call without
/// extra rights. Errors when there is no backlight or neither route works.
pub fn set(pct: u8) -> Result<(), String> {
    let device = device_in(Path::new(SYSFS_ROOT)).ok_or("this machine has no backlight")?;
    let pct = pct.clamp(FLOOR_PCT, 100);
    let via_ctl = Command::new("brightnessctl")
        .args(["--device", &device.name, "set", &format!("{pct}%")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if matches!(via_ctl, Ok(s) if s.success()) {
        return Ok(());
    }
    set_via_logind(&device, pct)
}

/// The `brightnessctl` arguments that move the backlight by `step` percent in one call, never
/// below the floor `set` keeps (`-n` is brightnessctl's minimum), so a held key cannot black the
/// screen out. Atomic for the same reason as `audio::step_volume`.
fn step_args(device: &str, step: i8) -> Vec<String> {
    let amount = format!("{}%{}", step.unsigned_abs(), if step < 0 { "-" } else { "+" });
    ["--device", device, "-n", "1", "set"].iter().map(|s| s.to_string()).chain([amount]).collect()
}

/// Move the backlight by `step` percent. `Ok(true)` when brightnessctl did it in one call;
/// `Ok(false)` when it is not installed or refused, and the caller falls back to reading and
/// setting (logind has no relative call).
pub fn step(step: i8) -> Result<bool, String> {
    let device = device_in(Path::new(SYSFS_ROOT)).ok_or("this machine has no backlight")?;
    let via_ctl = Command::new("brightnessctl")
        .args(step_args(&device.name, step))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    Ok(matches!(via_ctl, Ok(s) if s.success()))
}

fn set_via_logind(device: &Device, pct: u8) -> Result<(), String> {
    let connection = zbus::blocking::Connection::system().map_err(|e| format!("no system bus: {e}"))?;
    connection
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1/session/auto",
            Some("org.freedesktop.login1.Session"),
            "SetBrightness",
            &("backlight", device.name.as_str(), raw_for(pct, device.max)),
        )
        .map(|_| ())
        .map_err(|e| format!("neither brightnessctl nor logind could set the brightness: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_is_one_relative_brightnessctl_call_with_a_floor() {
        assert_eq!(step_args("intel_backlight", 5), ["--device", "intel_backlight", "-n", "1", "set", "5%+"]);
        assert_eq!(step_args("intel_backlight", -5).last().map(String::as_str), Some("5%-"));
    }

    /// A fake `/sys/class/backlight` in a directory of its own.
    struct FakeSysfs(PathBuf);
    impl FakeSysfs {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!("yos-backlight-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn device(&self, name: &str, brightness: &str, max: &str) -> &Self {
            let dir = self.0.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("brightness"), format!("{brightness}\n")).unwrap();
            std::fs::write(dir.join("max_brightness"), format!("{max}\n")).unwrap();
            self
        }
    }
    impl Drop for FakeSysfs {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn no_backlight_directory_is_no_backlight() {
        assert_eq!(read_in(Path::new("/definitely/not/here")), None);
        let empty = FakeSysfs::new("empty");
        assert_eq!(read_in(&empty.0), None, "a VM's empty class directory");
    }

    #[test]
    fn a_laptop_panel_reads_as_a_percent_of_its_own_range() {
        let fake = FakeSysfs::new("laptop");
        fake.device("intel_backlight", "48000", "96000");
        assert_eq!(read_in(&fake.0), Some(50));
        fake.device("intel_backlight", "96000", "96000");
        assert_eq!(read_in(&fake.0), Some(100));
        fake.device("intel_backlight", "1", "96000");
        assert_eq!(read_in(&fake.0), Some(0));
    }

    #[test]
    fn a_device_with_no_range_or_junk_values_is_skipped() {
        let fake = FakeSysfs::new("junk");
        fake.device("a_broken", "5", "0").device("b_junk", "bright", "100").device("c_good", "30", "100");
        let d = device_in(&fake.0).expect("the usable one is found");
        assert_eq!((d.name.as_str(), d.percent()), ("c_good", 30));
    }

    #[test]
    fn a_reading_past_the_range_is_full_not_more() {
        let fake = FakeSysfs::new("over");
        fake.device("acpi_video0", "300", "255");
        assert_eq!(read_in(&fake.0), Some(100));
    }

    #[test]
    fn logind_is_asked_for_a_value_in_the_devices_own_range() {
        assert_eq!(raw_for(50, 255), 128);
        assert_eq!(raw_for(100, 96000), 96000);
        assert_eq!(raw_for(0, 255), 3, "never a black screen");
        assert_eq!(raw_for(0, 10), 1);
        assert_eq!(raw_for(250, 100), 100);
    }
}
