//! The one-password start on an encrypted install: the disk's password is typed once, on the
//! Yantrik pre-boot screen, and the session starts signed in (deploy/yantrik-os/boot-unlock).
//!
//! What the installer does for it, and nothing more:
//! - crypttab names the Yantrik keyscript, which asks on the Yantrik screen and falls back to
//!   cryptsetup's text prompt with the same words. Only when the keyscript is on the disk: a
//!   crypttab naming a keyscript the initramfs cannot carry is a disk nothing asks to open.
//! - `enrol` records whose password it is (a digest of their shadow entry, never the password).
//! - the boot line shows plymouth only on an encrypted install; an unencrypted one boots as it
//!   always has, with nothing to ask.
//! - the initramfs is checked for the keyscript and the marker, and for whether the Yantrik
//!   screen or the text prompt will ask.

use super::installer::chroot_cmd;

/// The keyscript, as crypttab names it and as it sits on the installed disk.
pub const KEYSCRIPT: &str = "/usr/lib/yantrik/boot-unlock/askpass";
/// Records whose password opens the disk, run as root inside the installed system.
const ENROL: &str = "/usr/lib/yantrik/boot-unlock/enrol";

/// The crypttab options for the root: asked for in the initramfs, through the Yantrik keyscript
/// when the installed system has it. cryptsetup's default of three tries, then its own fallback.
pub fn crypttab_options(has_keyscript: bool) -> String {
    if has_keyscript {
        format!("luks,initramfs,keyscript={KEYSCRIPT}")
    } else {
        "luks,initramfs".into()
    }
}

/// Whether the installed system carries the keyscript, executable.
pub fn keyscript_installed(mount_dir: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(format!("{mount_dir}{KEYSCRIPT}"))
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The kernel's command line on the installed system. `loglevel=3`: `quiet` still prints the
/// kernel's error-level lines, and they scroll over the disk's password prompt. Encrypted, plymouth
/// draws the pre-boot screen (`splash`): on the framebuffer the firmware left when there is no GPU
/// driver yet (`plymouth.use-simpledrm`; plymouth otherwise waits for a real driver, and the Mac
/// mini's i915 is not in the initramfs), and on the screen only (a serial console it also drew on
/// could take the prompt off the one the person is looking at).
pub fn cmdline_default(encrypted: bool) -> &'static str {
    if encrypted {
        "quiet splash loglevel=3 plymouth.use-simpledrm plymouth.ignore-serial-consoles"
    } else {
        // Nothing to ask for. plymouth is in the image now, and off here, so an unencrypted install
        // boots as it did before it was: no splash, no plymouthd.
        "quiet loglevel=3 plymouth.enable=0"
    }
}

/// Record whose password opens the disk, after it is set. A failure costs only the signed-in
/// start (the lock screen asks, as it always has), so it is logged, not fatal.
pub fn enrol(mount_dir: &str, username: &str) {
    match chroot_cmd(mount_dir, &[ENROL, username]) {
        Ok(_) => tracing::info!(user = username, "Installer: the disk's password signs this account in at boot"),
        Err(e) => tracing::warn!(error = %e, "Installer: could not enrol the account for the one-password start; the lock screen will ask after boot"),
    }
}

/// How the installed initramfs will ask for the disk's password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// The Yantrik screen, falling back to text.
    Yantrik,
    /// cryptsetup's text prompt with the Yantrik words (no plymouth, or no theme in it).
    Text,
    /// cryptsetup's own prompt: the keyscript is not in use.
    Stock,
}

/// How a listing of an initramfs will ask, or what it lacks when crypttab names the keyscript:
/// without the keyscript in the initramfs nothing asks for the disk's password at all.
pub fn prompt_in(listing: &str, names_keyscript: bool) -> Result<Prompt, &'static str> {
    let lines: Vec<&str> = listing.lines().map(str::trim_end).collect();
    let has = |suffix: &str| lines.iter().any(|l| l.ends_with(suffix));
    if !has(KEYSCRIPT.trim_start_matches('/')) {
        return if names_keyscript { Err("the Yantrik password prompt (its keyscript)") } else { Ok(Prompt::Stock) };
    }
    let graphical = has("usr/share/plymouth/themes/yantrik/yantrik.script")
        && has("usr/share/plymouth/themes/yantrik/field.png")
        && lines.iter().any(|l| l.contains("/plymouth/label") && l.ends_with(".so"))
        && lines.iter().any(|l| l.ends_with("/plymouth/script.so"))
        && has("bin/plymouth")
        && has("sbin/plymouthd");
    Ok(if graphical { Prompt::Yantrik } else { Prompt::Text })
}

/// Whether the initramfs leaves the signed-in marker (scripts/local-bottom/yantrik-unlock).
/// Without it the disk still opens; the lock screen asks after boot, as it always has.
pub fn marker_in(listing: &str) -> bool {
    listing.lines().any(|l| l.trim_end().ends_with("scripts/local-bottom/yantrik-unlock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crypttab_names_the_keyscript_only_when_the_disk_has_it() {
        assert_eq!(crypttab_options(true), "luks,initramfs,keyscript=/usr/lib/yantrik/boot-unlock/askpass");
        assert_eq!(crypttab_options(false), "luks,initramfs");
        assert!(!crypttab_options(true).contains("tries="), "cryptsetup's own retries and fallback");
    }

    #[test]
    fn only_an_encrypted_install_draws_the_pre_boot_screen() {
        assert!(cmdline_default(true).split_whitespace().any(|w| w == "splash"));
        assert!(cmdline_default(true).contains("loglevel=3"));
        assert!(cmdline_default(true).contains("plymouth.use-simpledrm"), "the firmware's framebuffer, no GPU driver");
        assert!(!cmdline_default(false).contains("splash"), "unencrypted boots as it did: nothing to ask");
        assert!(cmdline_default(false).contains("loglevel=3"));
        assert!(cmdline_default(false).contains("plymouth.enable=0"), "and plymouth, now in the image, stays off");
    }

    /// The pre-boot screen shows the account's name and layout, written into the initramfs on the
    /// unencrypted /boot: the installer says so where encryption is chosen.
    #[test]
    fn the_installer_says_the_pre_boot_screen_shows_name_and_layout() {
        let slint = include_str!("../../../yantrik-ui-slint/ui/installer.slint");
        assert!(slint.contains("on a screen that shows your name and keyboard layout before the disk is unlocked"));
    }

    /// The text installer never encrypts, so its boot line is the unencrypted one: never `splash`,
    /// which with plymouth in the image would start plymouthd at every boot and shutdown.
    #[test]
    fn the_text_installer_boots_as_an_unencrypted_install() {
        let script = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../deploy/yantrik-os/yantrik-install.sh"));
        let lines: Vec<&str> = script.lines().filter(|l| l.contains("GRUB_CMDLINE_LINUX_DEFAULT")).collect();
        assert_eq!(lines.len(), 1, "one boot line in yantrik-install.sh: {lines:?}");
        let wanted = format!("GRUB_CMDLINE_LINUX_DEFAULT=\"{}\"", cmdline_default(false));
        assert!(lines[0].contains(&wanted), "yantrik-install.sh writes {wanted}: {}", lines[0]);
        assert!(!lines[0].split(|c: char| c.is_whitespace() || c == '"').any(|w| w == "splash"));
    }

    const BASE: &str = "cryptroot/crypttab\nusr/sbin/cryptsetup\nusr/lib/cryptsetup/askpass\n";
    const KEYSCRIPT_AND_MARKER: &str = "usr/lib/yantrik/boot-unlock/askpass\nscripts/local-bottom/yantrik-unlock\n";
    const THEME: &str = "usr/bin/plymouth\nusr/sbin/plymouthd\n\
        usr/lib/x86_64-linux-gnu/plymouth/script.so\nusr/lib/x86_64-linux-gnu/plymouth/label-pango.so\n\
        usr/share/plymouth/themes/yantrik/yantrik.script\nusr/share/plymouth/themes/yantrik/field.png\n";

    #[test]
    fn the_initramfs_says_which_prompt_asks() {
        assert_eq!(prompt_in(BASE, false), Ok(Prompt::Stock));
        assert_eq!(prompt_in(&format!("{BASE}{KEYSCRIPT_AND_MARKER}{THEME}"), true), Ok(Prompt::Yantrik));
        // No plymouth, or a theme without a way to write: the keyscript's text prompt asks.
        assert_eq!(prompt_in(&format!("{BASE}{KEYSCRIPT_AND_MARKER}"), true), Ok(Prompt::Text));
        let no_label = THEME.replace("label-pango.so", "nothing");
        assert_eq!(prompt_in(&format!("{BASE}{KEYSCRIPT_AND_MARKER}{no_label}"), true), Ok(Prompt::Text));
        // crypttab names a keyscript the initramfs does not carry: nothing would ask at boot.
        assert!(prompt_in(&format!("{BASE}{THEME}"), true).is_err());
    }

    #[test]
    fn the_marker_script_is_found_or_missed() {
        assert!(marker_in(KEYSCRIPT_AND_MARKER));
        assert!(!marker_in(BASE));
    }
}
