//! What an archive holds, checked before `archive_extract` lets tar write any of it.
//!
//! `validate_path` looked at the archive and at the folder it goes into, and tar then wrote
//! whatever names the archive carried. A member named `.config/autostart/x.desktop` extracted
//! into the home is a program the session runs at login; a symlink member `k -> ~/.ssh`
//! followed by a member `k/authorized_keys` writes through the link; a hard link member names
//! any file the person owns and makes it the archive's to overwrite. So every member is listed
//! first: any link or special file refuses the whole archive, and every name, put where it will
//! land, must pass the same `validate_write_path` a single write would - which refuses a
//! member under any hidden folder of the home, a repository's .git/config included.

use std::path::{Component, Path};

/// More members than this is not an archive the model needs to unpack unasked, and checking
/// each is a `validate_path` with a filesystem walk.
const MAX_MEMBERS: usize = 20_000;

/// Refuse extracting `archive` into `dest` unless every member is a plain file or folder whose
/// place passes `validate`.
pub fn check(archive: &str, dest: &str, validate: impl Fn(&str) -> Result<String, String>) -> Result<(), String> {
    refuse_links(&list(archive, "tzvf")?)?;
    let names = list(archive, "tzf")?;
    let names: Vec<&str> = names.lines().filter(|l| !l.is_empty()).collect();
    if names.len() > MAX_MEMBERS {
        return Err(format!("the archive holds {} members; more than {MAX_MEMBERS} is not unpacked", names.len()));
    }
    for name in names {
        member_verdict(name, dest, &validate)?;
    }
    Ok(())
}

fn list(archive: &str, flags: &str) -> Result<String, String> {
    match std::process::Command::new("tar").args([flags, archive]).output() {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(o) => Err(format!("the archive could not be read: {}", String::from_utf8_lossy(&o.stderr).trim())),
        Err(e) => Err(format!("tar not available? {e}")),
    }
}

/// Refuse a verbose listing (`tar tvf`) with anything but plain files and folders in it. The
/// type is the first letter of each line: `-` and `d` only. GNU tar marks a hard link `h` and
/// writes `link to`; BusyBox marks it with its mode and writes `->`, as both do for a symlink,
/// so either is refused on its own.
pub fn refuse_links(listing: &str) -> Result<(), String> {
    for line in listing.lines().filter(|l| !l.trim().is_empty()) {
        let plain = matches!(line.chars().next(), Some('-') | Some('d'));
        if !plain || line.contains(" -> ") || line.contains(" link to ") {
            return Err(format!(
                "the archive holds a link or a special file, which could put what follows it \
                 anywhere; nothing was extracted ({})",
                line.trim()
            ));
        }
    }
    Ok(())
}

/// Refuse one member name unless it stays below `dest` and its place there passes `validate`.
pub fn member_verdict(name: &str, dest: &str, validate: impl Fn(&str) -> Result<String, String>) -> Result<(), String> {
    let member = Path::new(name);
    if member.is_absolute() || member.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!("the archive member {name} would land outside {dest}; nothing was extracted"));
    }
    let place = Path::new(dest).join(member);
    validate(&place.to_string_lossy())
        .map(|_| ())
        .map_err(|e| format!("the archive member {name} cannot be written there: {e}; nothing was extracted"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_of_plain_files_and_folders_passes() {
        let listing = "drwxr-xr-x ann/ann 0 2026-09-28 10:00 notes/\n\
                       -rw-r--r-- ann/ann 12 2026-09-28 10:00 notes/today.txt\n";
        assert!(refuse_links(listing).is_ok());
    }

    #[test]
    fn a_symlink_or_hard_link_member_refuses_the_archive() {
        for line in [
            "lrwxrwxrwx ann/ann 0 2026-09-28 10:00 k -> /home/ann/.ssh",
            "hrw-r--r-- ann/ann 0 2026-09-28 10:00 twin link to /etc/shadow",
            "-rw-r--r-- ann/ann 0 2026-09-28 10:00 twin -> /etc/shadow",
            "crw-r--r-- root/root 0 2026-09-28 10:00 dev/null",
            "prw-r--r-- ann/ann 0 2026-09-28 10:00 fifo",
        ] {
            assert!(refuse_links(line).unwrap_err().contains("nothing was extracted"), "{line}");
        }
    }

    #[test]
    fn a_member_is_checked_where_it_will_land() {
        let validate = |p: &str| {
            if p.contains(".config/autostart") { Err("protected".to_string()) } else { Ok(p.to_string()) }
        };
        assert!(member_verdict("notes/today.txt", "/home/ann/in", validate).is_ok());
        assert!(member_verdict(".config/autostart/x.desktop", "/home/ann", validate).is_err());
        assert!(member_verdict("/etc/profile", "/home/ann", validate).is_err());
        assert!(member_verdict("a/../../.bashrc", "/home/ann/in", validate).is_err());
    }

    #[test]
    fn a_real_archive_with_a_link_in_it_is_refused_before_anything_is_written() {
        let dir = std::env::temp_dir().join(format!("yantrik-archive-members-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/notes")).unwrap();
        std::fs::write(dir.join("src/notes/today.txt"), "x").unwrap();
        let tar = |name: &str| {
            let archive = dir.join(name);
            let ok = std::process::Command::new("tar")
                .args(["czf", archive.to_str().unwrap(), "-C", dir.join("src").to_str().unwrap(), "."])
                .status()
                .unwrap()
                .success();
            assert!(ok, "tar made {name}");
            archive.to_string_lossy().into_owned()
        };
        let allow = |p: &str| Ok::<String, String>(p.to_string());
        let dest = dir.join("out").to_string_lossy().into_owned();

        assert!(check(&tar("plain.tgz"), &dest, allow).is_ok());
        std::os::unix::fs::symlink("/etc", dir.join("src/escape")).unwrap();
        assert!(check(&tar("symlink.tgz"), &dest, allow).is_err(), "a symlink member");
        std::fs::remove_file(dir.join("src/escape")).unwrap();
        std::fs::hard_link(dir.join("src/notes/today.txt"), dir.join("src/twin.txt")).unwrap();
        assert!(check(&tar("hardlink.tgz"), &dest, allow).is_err(), "a hard link member");
        assert!(!dir.join("out").exists(), "checking writes nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
