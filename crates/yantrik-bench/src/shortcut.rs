//! A `.desktop` file on the desktop, read as an app shortcut.
//!
//! Only what the icon needs is read: the name, the program line, the icon name and whether it
//! is an application at all. The line is never run from here; the shell resolves a shortcut
//! against the app registry the dock uses and launches through the dock's own path, so a file
//! dropped on the desktop cannot start a program the dock could not.

use std::io::Read;
use std::path::Path;

/// A shortcut file is a few lines. Anything longer than this is not one.
const MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Shortcut {
    pub name: String,
    pub exec: String,
    pub icon: String,
    /// The file name without `.desktop`: `yantrik-notes`, the id the app registry knows it by.
    pub stem: String,
}

/// Read `path`. `None` when it is not a shortcut to an application: another Type, `Hidden`, or
/// no `[Desktop Entry]` group.
pub fn read(path: &Path) -> Option<Shortcut> {
    let mut text = String::new();
    std::fs::File::open(path).ok()?.take(MAX_BYTES).read_to_string(&mut text).ok()?;
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    parse(&text, &stem)
}

/// The pure half of [`read`].
pub fn parse(text: &str, stem: &str) -> Option<Shortcut> {
    let mut in_entry = false;
    let mut s = Shortcut { stem: stem.to_string(), ..Shortcut::default() };
    let (mut app, mut hidden) = (false, false);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        match key.trim() {
            // The unlocalised key only: `Name[de]` is another key and is skipped.
            "Name" if s.name.is_empty() => s.name = value.to_string(),
            "Exec" if s.exec.is_empty() => s.exec = value.to_string(),
            "Icon" if s.icon.is_empty() => s.icon = value.to_string(),
            "Type" => app = value == "Application",
            "Hidden" | "NoDisplay" if value.eq_ignore_ascii_case("true") => hidden = true,
            _ => {}
        }
    }
    (app && !hidden && !s.exec.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_application_entry_is_a_shortcut() {
        let t = "[Desktop Entry]\nType=Application\nName=Notes\nName[de]=Notizen\nExec=yantrik-notes %U\nIcon=notes\n";
        let s = parse(t, "yantrik-notes").unwrap();
        assert_eq!((s.name.as_str(), s.exec.as_str(), s.icon.as_str()), ("Notes", "yantrik-notes %U", "notes"));
        assert_eq!(s.stem, "yantrik-notes");
    }

    #[test]
    fn a_link_a_hidden_entry_and_other_groups_are_not() {
        assert!(parse("[Desktop Entry]\nType=Link\nName=x\nURL=https://x\nExec=x\n", "x").is_none());
        assert!(parse("[Desktop Entry]\nType=Application\nName=x\nExec=x\nHidden=true\n", "x").is_none());
        assert!(parse("[Desktop Action new]\nType=Application\nName=x\nExec=x\n", "x").is_none());
        assert!(parse("garbage\n", "x").is_none());
        assert!(parse("[Desktop Entry]\nType=Application\nName=x\n", "x").is_none(), "no program line");
    }

    #[test]
    fn a_huge_file_is_cut_not_read_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("big.desktop");
        let mut t = String::from("[Desktop Entry]\nType=Application\nName=Big\nExec=big\n");
        t.push_str(&"# padding\n".repeat(20_000));
        t.push_str("Icon=late\n");
        std::fs::write(&p, t).unwrap();
        let s = read(&p).unwrap();
        assert_eq!(s.icon, "", "what lies past the cap is never read");
    }
}
