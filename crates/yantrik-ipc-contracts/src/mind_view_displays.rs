//! Which Wayland displays are the person's and which are a Mind's (design #978, step 1).
//!
//! There is no literal. The shell is the person's session, so it records its own `WAYLAND_DISPLAY`
//! when it starts (`person-display`), and the displays Mind View's compositor serves when it
//! starts one (`mind-displays`), each in the session's private socket directory: a regular file of
//! this account's, written whole and renamed into place, readable by this account only. An app
//! reads them to decide whether it is drawn for a Mind, and so does the instance for a launch
//! handed to it.
//!
//! The rule, in order ([`Known::is_minds`]):
//! 1. a display that Mind View lists is always a Mind's;
//! 2. otherwise, with the person's display recorded, any display but that one is a Mind's;
//! 3. otherwise (the file is missing, unreadable, not ours or not a display name) the person's
//!    display is not known, and a copy is not the Mind's unless its display is one Mind View
//!    lists. That is the whole of the fail-closed rule when the record is lost: a Mind's display is
//!    still recognised by Mind View's own list, and the launch marker (`YANTRIK_FOR_MIND`, set on
//!    everything the shell starts for a Mind) is a separate signal the caller adds.
//!
//! With no display named the process is not on Wayland and there is nothing to compare.

use std::io::Write as _;
use std::path::Path;

/// The person's display, as the shell recorded it.
pub const PERSON_FILE: &str = "person-display";

/// The displays Mind View serves, one per line.
pub const MINDS_FILE: &str = "mind-displays";

/// The largest file either side reads.
const MAX_BYTES: u64 = 512;

/// A display name the files may hold: what a Wayland socket is called, and nothing that is a path.
pub fn is_display_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !name.starts_with('.')
}

/// Records the person's display in `dir`.
pub fn write_person(dir: &Path, display: &str) -> std::io::Result<()> {
    write_whole(&dir.join(PERSON_FILE), &format!("{display}\n"), is_display_name(display))
}

/// Removes the record of the person's display (the shell is not on Wayland).
pub fn forget_person(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(PERSON_FILE));
}

/// Records the displays Mind View serves in `dir`; none is an empty file.
pub fn write_minds(dir: &Path, displays: &[String]) -> std::io::Result<()> {
    let text: String = displays.iter().map(|d| format!("{d}\n")).collect();
    write_whole(&dir.join(MINDS_FILE), &text, displays.iter().all(|d| is_display_name(d)))
}

fn write_whole(path: &Path, text: &str, valid: bool) -> std::io::Result<()> {
    if !valid {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a display name"));
    }
    let temporary = path.with_extension(format!("tmp{}", std::process::id()));
    let _ = std::fs::remove_file(&temporary);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(text.as_bytes())?;
    drop(file);
    std::fs::rename(&temporary, path)
}

/// A regular file of this account's that nobody else can write, small enough to be one of ours.
fn ours(path: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid cannot fail and touches no memory.
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o022 != 0 {
            return None;
        }
    }
    std::fs::read_to_string(path).ok()
}

/// What the shell recorded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Known {
    /// The person's display, when the record is there and good.
    pub person: Option<String>,
    /// The displays Mind View serves.
    pub minds: Vec<String>,
}

impl Known {
    /// The records in `dir`. A file that is missing, not ours, group- or world-writable, too big or
    /// not made of display names is read as absent.
    pub fn read(dir: &Path) -> Known {
        let person = ours(&dir.join(PERSON_FILE))
            .and_then(|t| t.lines().next().map(|l| l.trim().to_string()))
            .filter(|d| is_display_name(d));
        let minds = ours(&dir.join(MINDS_FILE))
            .map(|t| t.lines().map(|l| l.trim().to_string()).filter(|d| is_display_name(d)).collect())
            .unwrap_or_default();
        Known { person, minds }
    }

    /// Whether a window on `display` is a Mind's. See the module documentation for the order.
    pub fn is_minds(&self, display: Option<&str>) -> bool {
        let Some(display) = display else { return false };
        if self.minds.iter().any(|d| d == display) {
            return true;
        }
        match &self.person {
            Some(person) => person != display,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("yantrik-displays-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn known(person: Option<&str>, minds: &[&str]) -> Known {
        Known { person: person.map(str::to_string), minds: minds.iter().map(|m| m.to_string()).collect() }
    }

    /// The person on any display is the person, and Mind View's seat is a Mind's.
    #[test]
    fn the_person_on_any_display_is_the_person_and_the_seat_is_a_mind() {
        for person in ["wayland-0", "wayland-1", "wayland-7"] {
            let k = known(Some(person), &["wayland-9"]);
            assert!(!k.is_minds(Some(person)), "{person}");
            assert!(k.is_minds(Some("wayland-9")));
            assert!(k.is_minds(Some("wayland-other")), "a third display is not the person's");
        }
        assert!(!known(Some("wayland-1"), &[]).is_minds(None), "not on Wayland: nothing to compare");
    }

    /// Mind View's list wins over the person's record.
    #[test]
    fn a_display_mind_view_lists_is_always_a_minds() {
        assert!(known(Some("wayland-1"), &["wayland-1"]).is_minds(Some("wayland-1")));
        assert!(known(None, &["wayland-1"]).is_minds(Some("wayland-1")));
    }

    /// Both branches of the rule when the person's record is lost.
    #[test]
    fn without_the_persons_record_only_mind_views_own_list_makes_a_display_a_minds() {
        assert!(!known(None, &["wayland-1"]).is_minds(Some("wayland-0")), "absent from the seats");
        assert!(!known(None, &[]).is_minds(Some("wayland-0")));
        assert!(known(None, &["wayland-1"]).is_minds(Some("wayland-1")), "listed");
    }

    #[test]
    fn the_records_are_written_whole_read_back_and_only_if_good() {
        let d = dir("rw");
        assert_eq!(Known::read(&d), Known::default(), "nothing recorded");
        write_person(&d, "wayland-1").unwrap();
        write_minds(&d, &["wayland-2".to_string()]).unwrap();
        assert_eq!(Known::read(&d), known(Some("wayland-1"), &["wayland-2"]));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(d.join(PERSON_FILE)).unwrap().permissions().mode() & 0o777, 0o600);
            // Group- or world-writable is not the shell's.
            std::fs::set_permissions(d.join(PERSON_FILE), std::fs::Permissions::from_mode(0o666)).unwrap();
            assert_eq!(Known::read(&d).person, None);
        }
        std::fs::write(d.join(PERSON_FILE), "a/b\n").unwrap();
        assert_eq!(Known::read(&d).person, None, "a path is not a display");
        std::fs::write(d.join(PERSON_FILE), "x".repeat(2000)).unwrap();
        assert_eq!(Known::read(&d).person, None, "too big");
        assert!(write_person(&d, "../x").is_err());
        write_minds(&d, &[]).unwrap();
        assert!(Known::read(&d).minds.is_empty());
        forget_person(&d);
        assert!(!d.join(PERSON_FILE).exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_not_a_record() {
        let d = dir("link");
        std::fs::write(d.join("elsewhere"), "wayland-1\n").unwrap();
        std::os::unix::fs::symlink(d.join("elsewhere"), d.join(PERSON_FILE)).unwrap();
        assert_eq!(Known::read(&d).person, None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
