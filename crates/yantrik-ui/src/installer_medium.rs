//! What the installer runs from, and what the person is told about it.
//!
//! The Installed screen said "Remove the installation media." whatever the installer ran from. On
//! the Mac mini it runs from YKINSTALL, a partition of the internal disk macOS is on: there is
//! nothing to remove, and trying to is how a person ends up opening a Mac. It now says what is
//! true for the medium in use. The Installing screen promised "The computer restarts by itself
//! when it is done", and the Installed screen waits for Restart now whenever it has something to
//! say; it now promises nothing it may not do.

/// The medium the live system runs from (/run/live/medium).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Medium {
    /// A partition of a disk inside the machine (YKINSTALL on the Mac).
    Internal,
    /// A USB drive, or any disk the kernel calls removable.
    Removable,
    /// A disc (sr0): live-medium-eject opens its tray as the machine restarts.
    Disc,
    /// Not known: nothing is mounted there, or it could not be told.
    Unknown,
}

/// Which medium: `disk` is the disk /run/live/medium is on (`sda`, `sr0`), `external` what
/// lsblk says of it (USB or removable).
pub fn medium(disk: Option<&str>, external: bool) -> Medium {
    match disk {
        None => Medium::Unknown,
        Some(d) if d.starts_with("sr") => Medium::Disc,
        Some(_) if external => Medium::Removable,
        Some(_) => Medium::Internal,
    }
}

/// The Installed screen's line about the medium.
pub fn finish_hint(medium: Medium) -> &'static str {
    match medium {
        Medium::Internal => "The installer stays on its own partition. There is nothing to remove.",
        Medium::Removable => {
            "Remove the USB drive once the screen goes dark, so the computer starts Yantrik OS and not the installer."
        }
        Medium::Disc => "The disc comes out as the computer restarts.",
        Medium::Unknown => "If the installer is on a USB drive or a disc, take it out once the screen goes dark.",
    }
}

/// The Installing screen's line: true whether or not the Installed screen restarts by itself.
/// installer.slint draws it as written here (the_screens_say_neither_old_sentence holds them equal).
#[cfg(test)]
pub const PROGRESS_HINT: &str = "You can leave this running. When it is done, this screen says what happens next.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_medium_is_told_from_its_disk() {
        assert_eq!(medium(Some("sda"), false), Medium::Internal, "YKINSTALL on the Mac's own disk");
        assert_eq!(medium(Some("sdb"), true), Medium::Removable);
        assert_eq!(medium(Some("sr0"), true), Medium::Disc);
        assert_eq!(medium(Some("sr0"), false), Medium::Disc);
        assert_eq!(medium(None, false), Medium::Unknown);
    }

    #[test]
    fn an_internal_installer_partition_is_never_to_be_removed() {
        let line = finish_hint(Medium::Internal).to_lowercase();
        assert!(line.contains("nothing to remove"), "{line}");
        assert!(!line.contains("remove the"), "{line}");
        assert!(!line.contains("media"), "{line}");
    }

    #[test]
    fn each_medium_is_told_what_is_true_for_it() {
        assert!(finish_hint(Medium::Removable).contains("Remove the USB drive"));
        assert!(finish_hint(Medium::Disc).contains("disc"));
        assert!(finish_hint(Medium::Unknown).starts_with("If"), "a guess is said as one");
        for m in [Medium::Internal, Medium::Removable, Medium::Disc, Medium::Unknown] {
            assert!(!finish_hint(m).contains("installation media"), "{m:?}");
        }
    }

    #[test]
    fn the_installing_screen_promises_no_restart() {
        assert!(!PROGRESS_HINT.contains("restarts by itself"));
        assert!(!PROGRESS_HINT.to_lowercase().contains("restart"));
    }

    /// The screens draw these lines, not their own: the old sentences are not in installer.slint.
    #[test]
    fn the_screens_say_neither_old_sentence() {
        let slint = include_str!("../../yantrik-ui-slint/ui/installer.slint");
        assert!(!slint.contains("restarts by itself when it is done"));
        assert!(!slint.contains("Remove the installation media"));
        assert!(slint.contains("root.media-hint"), "the Installed screen draws the medium's line");
        assert!(slint.contains(&format!("\"{PROGRESS_HINT}\"")), "the Installing screen draws this line");
        assert!(slint.contains(&format!("\"{}\"", finish_hint(Medium::Unknown))), "and, until told, the guess");
    }
}
