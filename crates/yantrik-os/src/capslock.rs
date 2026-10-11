//! Whether Caps Lock is on, read from the keyboard's own LED in sysfs.
//!
//! The compositor keeps the lock state, but the LED is what the person's eye reads, and the
//! kernel exposes it as `/sys/class/leds/inputN::capslock/brightness` (0 off, anything else on).
//! A machine with no keyboard LED of that name (a VM with a virtual keyboard, a remote session)
//! has nothing to read, and the answer is `None`: the shell then says nothing rather than
//! guessing "off".

use std::path::Path;

const SYSFS_ROOT: &str = "/sys/class/leds";

/// Caps Lock under `root`: `Some(true)` when any `*::capslock` LED is lit, `Some(false)` when
/// there is at least one and none is, `None` when there is none or none could be read. Several
/// keyboards share the lock state, so one lit LED is "on". `root` is a parameter so tests can
/// point it at fixture directories.
pub fn read_in(root: &Path) -> Option<bool> {
    let mut found = None;
    for entry in std::fs::read_dir(root).ok()?.filter_map(|e| e.ok()) {
        if !entry.file_name().to_string_lossy().ends_with("::capslock") {
            continue;
        }
        let Some(level) = std::fs::read_to_string(entry.path().join("brightness"))
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
        else {
            continue;
        };
        found = Some(found.unwrap_or(false) || level > 0);
    }
    found
}

/// Whether Caps Lock is on, or `None` when this machine has no Caps Lock LED to read.
pub fn read() -> Option<bool> {
    read_in(Path::new(SYSFS_ROOT))
}

/// How long after the key's release the LED gets to catch up before it is read. The compositor
/// toggles the lock on the press and tells the keyboard, and the kernel then updates the sysfs
/// brightness: the `yos` round trip from a release binding can beat that, which made the pill
/// say "off" for a lock that had just gone on (review of #579). One wait, not a poll.
pub const SETTLE: std::time::Duration = std::time::Duration::from_millis(60);

/// Read the LED after giving it [`SETTLE`] to follow the key. Blocks for that long, so only a
/// worker calls it. `settle` and `read` are parameters so the order is testable: the read must
/// come after the wait, never before.
pub fn read_after(settle: impl FnOnce(), read: impl FnOnce() -> Option<bool>) -> Option<bool> {
    settle();
    read()
}

/// [`read`], once the LED has had [`SETTLE`] to follow the key.
pub fn read_settled() -> Option<bool> {
    read_after(|| std::thread::sleep(SETTLE), read)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_led_is_read_only_after_the_settle_never_before() {
        let log = std::cell::RefCell::new(Vec::new());
        let got = read_after(|| log.borrow_mut().push("settle"), || {
            log.borrow_mut().push("read");
            Some(true)
        });
        assert_eq!(got, Some(true));
        assert_eq!(*log.borrow(), ["settle", "read"], "a read before the settle sees the old state");
    }

    #[test]
    fn the_settle_is_short_enough_for_a_key_press_and_long_enough_for_a_kernel_led_update() {
        assert!(SETTLE >= std::time::Duration::from_millis(30) && SETTLE <= std::time::Duration::from_millis(150));
    }

    /// A fake `/sys/class/leds` in a directory of its own.
    struct FakeLeds(std::path::PathBuf);
    impl FakeLeds {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!("yos-capslock-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn led(&self, name: &str, brightness: &str) -> &Self {
            let dir = self.0.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("brightness"), format!("{brightness}\n")).unwrap();
            self
        }
    }
    impl Drop for FakeLeds {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn no_leds_directory_or_no_capslock_led_is_no_reading() {
        assert_eq!(read_in(Path::new("/definitely/not/here")), None);
        let fake = FakeLeds::new("none");
        assert_eq!(read_in(&fake.0), None, "an empty class directory");
        fake.led("input3::numlock", "1").led("input3::scrolllock", "0");
        assert_eq!(read_in(&fake.0), None, "other lock LEDs are not Caps Lock");
    }

    #[test]
    fn the_capslock_led_says_on_or_off() {
        let fake = FakeLeds::new("one");
        fake.led("input3::capslock", "0").led("input3::numlock", "1");
        assert_eq!(read_in(&fake.0), Some(false));
        fake.led("input3::capslock", "1");
        assert_eq!(read_in(&fake.0), Some(true));
    }

    #[test]
    fn one_lit_keyboard_is_on_and_a_junk_led_is_skipped() {
        let fake = FakeLeds::new("two");
        fake.led("input3::capslock", "0").led("input7::capslock", "1");
        assert_eq!(read_in(&fake.0), Some(true), "two keyboards, one lit");
        let junk = FakeLeds::new("junk");
        junk.led("input3::capslock", "lit");
        assert_eq!(read_in(&junk.0), None, "an unreadable LED is no reading, not off");
        junk.led("input7::capslock", "0");
        assert_eq!(read_in(&junk.0), Some(false), "the readable one answers");
    }
}
