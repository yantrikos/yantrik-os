//! Whether this machine may ask the network where it is: off until the person says so.
//!
//! A place found from an IP address is a request that reveals the machine's public IP to a
//! third party (the desktop's own lookup asks ipinfo.io; a weather query with no place named
//! asks wttr.in, which finds the caller server-side). Nothing here makes either request. This is
//! the one answer every caller checks first, so the desktop, the weather tool and the weather
//! watch cannot disagree about it.
//!
//! The switch is a file, `~/.config/yantrik/location-lookup`, holding `on`. It is absent on a
//! fresh machine, so a first boot with nobody at the keyboard asks nobody anything. A feature
//! that needs a place and finds the switch off asks the person, on the shell's own approval card
//! (`allow_location_lookup`, in `yantrik-ui`), and the shell writes the file when they press
//! Allow. Nothing else writes it: [`turn_on`] is the shell's alone, and the source-scan test in
//! `tests/no_unattended_location_lookups.rs` keeps it so.
//!
//! The file is believed only when it looks like the shell's: a regular file (never a link) owned
//! by the person, mode 0600, in a directory the person owns that neither the `yantrik-minds`
//! group nor anyone else can write. A harness that runs as the person could still write it, but
//! then it is not the person's switch and the answer is no ([`verdict`] says why).
//! `YANTRIK_NO_GEOLOCATE=1` outranks everything: when set, the answer is no even when asked.

use std::path::{Path, PathBuf};

/// What a feature says about the weather lookup before it asks.
pub const ASK_WEATHER: &str = "Without a place, wttr.in (a weather service) works out where you are from \
    your internet address. Say yes to allow that, or name a city and wttr.in gets only the city name to \
    look up (it still sees your address, as any website does).";

/// What the desktop says about its own place-and-timezone lookup.
pub const ASK_DESKTOP: &str = "Finding this machine's place and timezone sends its internet address to \
    ipinfo.io (a location service). Say yes to allow that, or set your place and timezone by hand and \
    nothing is sent.";

const ENV_OFF: &str = "YANTRIK_NO_GEOLOCATE";
/// The switch's file name, under `~/.config/yantrik/`.
pub const FILE: &str = "location-lookup";
/// The group the minds run in; it must not be able to write the switch's directory.
#[cfg(unix)]
const MINDS_GROUP: &str = "yantrik-minds";

/// True when `YANTRIK_NO_GEOLOCATE` says never.
pub fn env_forbids() -> bool {
    forbids(std::env::var(ENV_OFF).ok().as_deref())
}

fn forbids(value: Option<&str>) -> bool {
    matches!(value.map(str::trim), Some("1" | "true" | "yes" | "on"))
}

/// Where the switch lives.
pub fn switch_path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/root"));
    home.join(".config/yantrik").join(FILE)
}

/// What the switch says right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// No switch, or it does not say `on`: the default.
    Off,
    /// Turned on by the person, at this local time.
    On { since: String },
    /// A file is there but is not believed, for this reason.
    Refused(&'static str),
    /// `YANTRIK_NO_GEOLOCATE` forbids it.
    Forbidden,
}

impl Verdict {
    /// The one line shown where the person can see it (`describe shell`, the log).
    pub fn line(&self) -> String {
        match self {
            Verdict::Off => "Location lookups: off".into(),
            Verdict::On { since } => format!("Location lookups: on (since {since})"),
            Verdict::Refused(why) => format!("Location lookups: off ({why})"),
            Verdict::Forbidden => format!("Location lookups: off ({ENV_OFF} is set)"),
        }
    }
}

/// The switch as it is now.
pub fn verdict() -> Verdict {
    if env_forbids() {
        return Verdict::Forbidden;
    }
    verdict_of(&switch_path())
}

/// May an IP-based lookup be made now: the person turned it on, and nothing forbids it.
pub fn allowed() -> bool {
    matches!(verdict(), Verdict::On { .. })
}

/// What the filesystem says about the switch and the directory holding it.
#[derive(Debug, Clone, Copy)]
struct Facts {
    is_file: bool,
    uid: u32,
    mode: u32,
    dir_uid: u32,
    dir_gid: u32,
    dir_mode: u32,
}

/// Why `facts` are not the person's own file, or `None` when they are. `me` is the person's uid
/// and `minds` the gid of the minds' group, when it exists.
fn doubt(facts: &Facts, me: u32, minds: Option<u32>) -> Option<&'static str> {
    if !facts.is_file {
        return Some("the switch is not a plain file");
    }
    if facts.uid != me {
        return Some("the switch is not owned by you");
    }
    if facts.mode & 0o777 != 0o600 {
        return Some("the switch is not mode 0600");
    }
    if facts.dir_uid != me {
        return Some("the switch's directory is not yours");
    }
    if facts.dir_mode & 0o002 != 0 || (facts.dir_mode & 0o020 != 0 && minds == Some(facts.dir_gid)) {
        return Some("the switch's directory can be written by others");
    }
    None
}

#[cfg(unix)]
fn minds_gid() -> Option<u32> {
    let name = std::ffi::CString::new(MINDS_GROUP).ok()?;
    // SAFETY: a NUL-terminated name; the result is read at once, before any other call.
    let group = unsafe { libc::getgrnam(name.as_ptr()) };
    // SAFETY: checked non-null just above.
    (!group.is_null()).then(|| unsafe { (*group).gr_gid })
}

#[cfg(unix)]
fn verdict_of(path: &Path) -> Verdict {
    use std::os::unix::fs::MetadataExt;
    // `symlink_metadata`: a link is looked at, not followed.
    let Ok(meta) = std::fs::symlink_metadata(path) else { return Verdict::Off };
    let Some(dir) = path.parent().and_then(|d| std::fs::metadata(d).ok()) else { return Verdict::Off };
    let facts = Facts {
        is_file: meta.file_type().is_file(),
        uid: meta.uid(),
        mode: meta.mode(),
        dir_uid: dir.uid(),
        dir_gid: dir.gid(),
        dir_mode: dir.mode(),
    };
    // SAFETY: geteuid has no preconditions.
    if let Some(why) = doubt(&facts, unsafe { libc::geteuid() }, minds_gid()) {
        return Verdict::Refused(why);
    }
    match std::fs::read_to_string(path) {
        Ok(s) if matches!(s.trim(), "on" | "1" | "yes" | "true") => Verdict::On { since: since(&meta) },
        _ => Verdict::Off,
    }
}

/// Without unix ownership there is nothing to believe: the switch stays off.
#[cfg(not(unix))]
fn verdict_of(_path: &Path) -> Verdict {
    Verdict::Off
}

#[cfg(unix)]
fn since(meta: &std::fs::Metadata) -> String {
    meta.modified()
        .map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|_| "unknown".into())
}

/// Write the switch, as the shell does when the person presses Allow. **The shell's only.** The
/// file is made 0600 under a fresh name and moved into place, so a link planted at the name is
/// replaced, never written through.
#[cfg(unix)]
pub fn turn_on() -> std::io::Result<()> {
    turn_on_at(&switch_path())
}

#[cfg(unix)]
fn turn_on_at(path: &Path) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no directory"))?;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let tmp = dir.join(format!(".{FILE}.{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
    file.write_all(b"on\n")?;
    drop(file);
    std::fs::rename(&tmp, path)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const ME: u32 = 1000;
    const MINDS: u32 = 990;

    fn good() -> Facts {
        Facts { is_file: true, uid: ME, mode: 0o100600, dir_uid: ME, dir_gid: ME, dir_mode: 0o40700 }
    }

    #[test]
    fn the_persons_own_private_file_is_believed() {
        assert_eq!(doubt(&good(), ME, Some(MINDS)), None);
    }

    #[test]
    fn each_way_the_file_can_be_not_theirs_is_refused() {
        let cases = [
            (Facts { is_file: false, ..good() }, "plain file"),
            (Facts { uid: 0, ..good() }, "not owned by you"),
            (Facts { mode: 0o100644, ..good() }, "0600"),
            (Facts { mode: 0o100660, ..good() }, "0600"),
            (Facts { dir_uid: 0, ..good() }, "not yours"),
            (Facts { dir_mode: 0o40777, ..good() }, "written by others"),
            (Facts { dir_gid: MINDS, dir_mode: 0o40770, ..good() }, "written by others"),
        ];
        for (facts, word) in cases {
            let why = doubt(&facts, ME, Some(MINDS)).unwrap_or_else(|| panic!("{facts:?} must be refused"));
            assert!(why.contains(word), "{why}");
        }
        // A group-writable directory of some other group is not the minds'.
        assert_eq!(doubt(&Facts { dir_gid: ME, dir_mode: 0o40770, ..good() }, ME, Some(MINDS)), None);
    }

    #[test]
    fn a_fresh_machine_has_no_switch_so_nothing_is_allowed() {
        let dir = scratch("fresh");
        assert_eq!(verdict_of(&dir.join(FILE)), Verdict::Off);
    }

    #[test]
    fn the_shells_own_write_is_on_and_says_since_when() {
        let dir = scratch("on");
        let file = dir.join(FILE);
        turn_on_at(&file).unwrap();
        let on = verdict_of(&file);
        assert!(matches!(on, Verdict::On { .. }), "{on:?}");
        assert!(on.line().starts_with("Location lookups: on (since 20"), "{}", on.line());
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn only_an_explicit_on_turns_it_on() {
        let dir = scratch("text");
        let file = dir.join(FILE);
        for (text, want) in [("on\n", true), ("1", true), ("off", false), ("", false), ("maybe", false)] {
            std::fs::write(&file, text).unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert_eq!(matches!(verdict_of(&file), Verdict::On { .. }), want, "{text:?}");
        }
    }

    #[test]
    fn a_file_made_with_the_wrong_mode_or_a_link_is_not_the_switch() {
        let dir = scratch("refuse");
        let file = dir.join(FILE);
        std::fs::write(&file, "on\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(verdict_of(&file), Verdict::Refused(_)), "mode 0644");
        std::fs::remove_file(&file).unwrap();

        let real = dir.join("elsewhere");
        std::fs::write(&real, "on\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&real, &file).unwrap();
        assert!(matches!(verdict_of(&file), Verdict::Refused(_)), "a symlink");
    }

    #[test]
    fn a_world_writable_directory_is_refused() {
        let dir = scratch("dir");
        let file = dir.join(FILE);
        turn_on_at(&file).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(matches!(verdict_of(&file), Verdict::Refused(_)));
    }

    #[test]
    fn writing_over_a_planted_link_replaces_it_and_never_writes_through() {
        let dir = scratch("plant");
        let victim = dir.join("victim");
        std::fs::write(&victim, "keep").unwrap();
        let file = dir.join(FILE);
        std::os::unix::fs::symlink(&victim, &file).unwrap();
        turn_on_at(&file).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        assert!(matches!(verdict_of(&file), Verdict::On { .. }));
    }

    #[test]
    fn the_environment_forbids_even_when_asked() {
        for v in ["1", "true", "yes", "on", " 1 "] {
            assert!(forbids(Some(v)), "{v:?}");
        }
        for v in [None, Some(""), Some("0"), Some("no")] {
            assert!(!forbids(v), "{v:?}");
        }
    }

    #[test]
    fn the_asks_say_what_is_sent_and_to_whom() {
        assert!(ASK_WEATHER.contains("internet address") && ASK_WEATHER.contains("wttr.in"));
        assert!(!ASK_WEATHER.contains("ipinfo.io"), "ipinfo.io plays no part in weather");
        assert!(ASK_DESKTOP.contains("internet address") && ASK_DESKTOP.contains("ipinfo.io"));
        assert!(!ASK_WEATHER.contains("nothing is sent") && !ASK_DESKTOP.contains("wttr.in"));
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-loc-consent-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }
}
