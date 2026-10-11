//! The battery read straight from `/sys/class/power_supply`, for a machine where UPower is not
//! on the bus.
//!
//! UPower is the better source (it has time remaining and sends a signal when anything changes)
//! but it is a separate package, and an image that lacks it must still show a battery on a
//! laptop. The kernel's own files are always there. They carry a level and a status and nothing
//! else: no time estimate, so none is shown rather than one worked out here.
//!
//! The reader takes the directory as an argument so the tests can point it at fixtures with
//! the real files' shapes.

use std::path::Path;

use crate::battery::Reading;
use crate::events::BatteryState;

pub(crate) const POWER_SUPPLY: &str = "/sys/class/power_supply";

/// The first present battery under `root`, or `None` when the machine has none (a desktop, a
/// virtual machine: the directory is empty or holds only a mains supply).
pub(crate) fn read_at(root: &Path) -> Option<Reading> {
    let mut names: Vec<String> = std::fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    // Directory order is not stable; BAT0 before BAT1 is.
    names.sort();

    let mains = mains_online(root, &names);
    for name in names.iter().filter(|n| n.starts_with("BAT")) {
        let dir = root.join(name);
        // A bay with no battery in it can still list a directory.
        if read_trimmed(&dir.join("present")).as_deref() == Some("0") {
            continue;
        }
        if let Some(kind) = read_trimmed(&dir.join("type")) {
            if kind != "Battery" {
                continue;
            }
        }
        // No level, no battery to speak of: a reading of "0%" made up here would be the
        // "battery critical" on a machine that has none.
        let level = match read_trimmed(&dir.join("capacity")).and_then(|c| c.parse::<u32>().ok()) {
            Some(c) => c.min(100) as u8,
            None => continue,
        };
        let status = read_trimmed(&dir.join("status"));
        return Some(Reading {
            level,
            state: state_from(status.as_deref(), mains, level),
            time_to_empty_mins: None,
            time_to_full_mins: None,
        });
    }
    None
}

/// Whether any mains supply says it is online. `None` when none could be read.
fn mains_online(root: &Path, names: &[String]) -> Option<bool> {
    let mut seen = false;
    for name in names.iter().filter(|n| n.starts_with("AC") || n.starts_with("ADP")) {
        match read_trimmed(&root.join(name).join("online")).as_deref() {
            Some("1") => return Some(true),
            Some(_) => seen = true,
            None => {}
        }
    }
    if seen { Some(false) } else { None }
}

/// The kernel's `status` word, with the mains supply as the tie-breaker when the word is
/// missing or says nothing ("Unknown").
pub(crate) fn state_from(status: Option<&str>, mains: Option<bool>, level: u8) -> BatteryState {
    match status.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("charging") => BatteryState::Charging,
        Some("discharging") => BatteryState::Discharging,
        Some("full") => BatteryState::Full,
        // A charge limit: on the charger, taking none.
        Some("not charging") => BatteryState::PluggedNotCharging,
        _ => match mains {
            Some(true) if level >= 100 => BatteryState::Full,
            Some(true) => BatteryState::PluggedNotCharging,
            Some(false) => BatteryState::Discharging,
            None => BatteryState::Unknown,
        },
    }
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A scratch `power_supply` directory that removes itself.
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("yos-sysfs-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Fixture(dir)
        }
        fn file(&self, rel: &str, body: &str) {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    /// A ThinkPad-shaped battery: the kernel ends every file with a newline.
    fn thinkpad(f: &Fixture, capacity: &str, status: &str) {
        f.file("BAT0/type", "Battery\n");
        f.file("BAT0/present", "1\n");
        f.file("BAT0/capacity", capacity);
        f.file("BAT0/status", status);
    }

    #[test]
    fn a_discharging_laptop_reads_its_level_and_state_and_no_invented_time() {
        let f = Fixture::new("discharging");
        thinkpad(&f, "64\n", "Discharging\n");
        f.file("AC/type", "Mains\n");
        f.file("AC/online", "0\n");
        let r = read_at(&f.0).expect("a battery");
        assert_eq!(r.level, 64);
        assert_eq!(r.state, BatteryState::Discharging);
        assert_eq!((r.time_to_empty_mins, r.time_to_full_mins), (None, None));
    }

    #[test]
    fn charging_and_full_are_told_apart() {
        let f = Fixture::new("charging");
        thinkpad(&f, "41\n", "Charging\n");
        assert_eq!(read_at(&f.0).unwrap().state, BatteryState::Charging);
        thinkpad(&f, "100\n", "Full\n");
        assert_eq!(read_at(&f.0).unwrap().state, BatteryState::Full);
    }

    /// A charge limit: plugged in at 80% and taking none. Neither "charging" nor "full".
    #[test]
    fn plugged_in_and_not_charging_is_its_own_state() {
        let f = Fixture::new("limit");
        thinkpad(&f, "80\n", "Not charging\n");
        f.file("ADP1/online", "1\n");
        let r = read_at(&f.0).unwrap();
        assert_eq!(r.state, BatteryState::PluggedNotCharging);
        assert!(r.state.on_charger());
    }

    /// Some firmware has no `status` at all. The mains supply then says what it can, and with
    /// neither the state is unknown rather than a guess.
    #[test]
    fn a_missing_status_file_falls_back_to_the_mains_supply() {
        let f = Fixture::new("nostatus");
        f.file("BAT0/type", "Battery\n");
        f.file("BAT0/capacity", "55\n");
        assert_eq!(read_at(&f.0).unwrap().state, BatteryState::Unknown);
        f.file("AC/online", "1\n");
        assert_eq!(read_at(&f.0).unwrap().state, BatteryState::PluggedNotCharging);
        f.file("AC/online", "0\n");
        assert_eq!(read_at(&f.0).unwrap().state, BatteryState::Discharging);
    }

    /// Without this a virtual machine that exposes only a mains supply, or an empty directory,
    /// would draw a battery at 0%.
    #[test]
    fn no_battery_reports_none() {
        let f = Fixture::new("none");
        assert!(read_at(&f.0).is_none(), "an empty directory");
        f.file("AC/type", "Mains\n");
        f.file("AC/online", "1\n");
        assert!(read_at(&f.0).is_none(), "a mains supply alone");
        assert!(read_at(&f.0.join("missing")).is_none(), "no such directory");
    }

    #[test]
    fn an_empty_bay_and_a_battery_with_no_level_are_not_batteries() {
        let f = Fixture::new("bay");
        f.file("BAT0/type", "Battery\n");
        f.file("BAT0/present", "0\n");
        f.file("BAT0/capacity", "0\n");
        assert!(read_at(&f.0).is_none(), "present=0");
        f.file("BAT0/present", "1\n");
        fs::remove_file(f.0.join("BAT0/capacity")).unwrap();
        assert!(read_at(&f.0).is_none(), "no capacity file");
    }

    #[test]
    fn the_second_battery_is_read_when_the_first_bay_is_empty() {
        let f = Fixture::new("two");
        f.file("BAT0/present", "0\n");
        f.file("BAT0/capacity", "0\n");
        f.file("BAT1/type", "Battery\n");
        f.file("BAT1/capacity", "90\n");
        f.file("BAT1/status", "Discharging\n");
        assert_eq!(read_at(&f.0).unwrap().level, 90);
    }

    #[test]
    fn a_level_past_100_is_clamped() {
        let f = Fixture::new("clamp");
        thinkpad(&f, "104\n", "Full\n");
        assert_eq!(read_at(&f.0).unwrap().level, 100);
    }
}
