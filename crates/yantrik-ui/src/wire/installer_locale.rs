//! The installer's keyboard layout and timezone: found on the live system, applied to the live
//! session, written into the installed one.
//!
//! Neither was asked before #400. A person with a German keyboard typed their password on a US
//! layout without knowing it, and met the difference at the lock screen of the installed
//! machine; the installed clock ran at UTC until the desktop's own geolocation got to it after
//! the first login. Both are one question each, and both have a good default we can find
//! without asking — so the installer finds it, shows it, and lets it be changed.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::installer_rules::{
    environment_with_layout, is_plausible_layout, keyboard_file, parse_keyboard_file,
    parse_localectl_layout, timezone_problem, DEFAULT_LAYOUT,
};

const PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// The zone used when the network cannot say.
pub const FALLBACK_TIMEZONE: &str = "UTC";

/// How long the geo-IP lookup may take. It runs on a worker thread while the person is on the
/// first two screens, so it only bounds how long the Review screen could say "Detecting…".
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

/// The layout the live system is already using: /etc/default/keyboard first, which is what the
/// image and Debian's keyboard-configuration write, then `localectl`, then US.
pub fn detect_layout() -> String {
    if let Some(layout) = std::fs::read_to_string("/etc/default/keyboard")
        .ok()
        .and_then(|text| parse_keyboard_file(&text))
    {
        return layout;
    }
    if let Some(layout) = Command::new("localectl")
        .arg("status")
        .env("PATH", PATH)
        .output()
        .ok()
        .and_then(|out| parse_localectl_layout(&String::from_utf8_lossy(&out.stdout)))
    {
        return layout;
    }
    DEFAULT_LAYOUT.to_string()
}

/// Put the chosen layout under the person's fingers now, in the live session.
///
/// labwc takes its layout from `XKB_DEFAULT_LAYOUT` in its environment file and re-reads that
/// file on `--reconfigure`, so this is one line written and one signal sent: the password typed
/// two screens later is typed on the layout the installed machine will ask for it on. Best
/// effort — a live session that cannot switch still installs the right layout.
pub fn apply_to_live_session(layout: &str) {
    if !is_plausible_layout(layout) {
        tracing::warn!(layout, "Not applying an implausible keyboard layout");
        return;
    }
    let Some(home) = std::env::var_os("HOME") else { return };
    let dir = Path::new(&home).join(".config/labwc");
    let path = dir.join("environment");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let written = std::fs::create_dir_all(&dir)
        .and_then(|_| std::fs::write(&path, environment_with_layout(&existing, layout)));
    if let Err(e) = written {
        tracing::warn!(error = %e, "Could not write the live session's keyboard layout");
        return;
    }
    match Command::new("labwc").arg("--reconfigure").env("PATH", PATH).output() {
        Ok(out) if out.status.success() => {
            tracing::info!(layout, "Keyboard layout applied to the live session")
        }
        Ok(out) => tracing::warn!(
            layout,
            error = %String::from_utf8_lossy(&out.stderr).trim(),
            "labwc did not reconfigure; the layout applies to the installed system only"
        ),
        Err(e) => tracing::warn!(layout, error = %e, "Could not run labwc --reconfigure"),
    }
}

/// Where this machine probably is, as a zone name the live system has zoneinfo for.
///
/// The same lookup the text installer makes (`yantrik-install.sh`): ip-api.com's one-line
/// answer, accepted only if it names a zone under /usr/share/zoneinfo, and UTC otherwise. One
/// request, revealing the machine's public IP to one service; `YANTRIK_NO_GEOLOCATE=1` turns it
/// off, as it does for the desktop's own lookup (`wire::location`).
pub fn detect_timezone() -> String {
    let off = std::env::var("YANTRIK_NO_GEOLOCATE")
        .map(|v| matches!(v.trim(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false);
    if off {
        return FALLBACK_TIMEZONE.to_string();
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(LOOKUP_TIMEOUT)
        .timeout_read(LOOKUP_TIMEOUT)
        .build();
    let answer = agent
        .get("http://ip-api.com/line/?fields=timezone")
        .call()
        .ok()
        .and_then(|r| r.into_string().ok())
        .unwrap_or_default();
    let zone = answer.lines().next().unwrap_or("").trim().to_string();
    if timezone_problem(&zone, Path::new("/usr/share/zoneinfo")).is_none() {
        tracing::info!(timezone = %zone, "Installer: timezone detected");
        zone
    } else {
        tracing::info!(answer = %zone, "Installer: no usable timezone from the network; UTC");
        FALLBACK_TIMEZONE.to_string()
    }
}

/// Write the layout into the installed system: /etc/default/keyboard for the console and
/// systemd-localed, and the console's cached keymap when console-setup is installed. The
/// desktop's own layout goes into the person's labwc environment (`create_user`).
pub fn configure_target_keyboard(mount_dir: &str, layout: &str) -> Result<(), String> {
    super::installer::sudo_write(&format!("{mount_dir}/etc/default/keyboard"), &keyboard_file(layout))?;
    // setupcon is console-setup's, which the image does not always carry. Without it the file
    // above is still what localed reports and what a later install of console-setup reads.
    let has_setupcon = ["usr/bin/setupcon", "bin/setupcon"]
        .iter()
        .any(|p| Path::new(mount_dir).join(p).exists());
    if has_setupcon {
        if let Err(e) = super::installer::chroot_cmd(mount_dir, &["setupcon", "--save-only"]) {
            tracing::warn!(error = %e, "setupcon could not cache the console keymap");
        }
    }
    Ok(())
}

/// Set the installed system's clock to `timezone`, checked against the target's own zoneinfo.
/// A zone the target does not have becomes UTC rather than a dangling /etc/localtime.
pub fn configure_target_timezone(mount_dir: &str, timezone: &str) -> Result<String, String> {
    let zoneinfo = format!("{mount_dir}/usr/share/zoneinfo");
    let zone = if timezone_problem(timezone, Path::new(&zoneinfo)).is_none() {
        timezone
    } else {
        tracing::warn!(timezone, "The installed system has no such zone; using UTC");
        FALLBACK_TIMEZONE
    };
    super::installer::run_cmd(
        "ln",
        &["-sfn", &format!("/usr/share/zoneinfo/{zone}"), &format!("{mount_dir}/etc/localtime")],
    )?;
    super::installer::sudo_write(&format!("{mount_dir}/etc/timezone"), &format!("{zone}\n"))?;
    // Keeps tzdata's own record in step; it has nothing else to say, so its failure is not ours.
    let _ = super::installer::chroot_cmd(
        mount_dir,
        &["dpkg-reconfigure", "-f", "noninteractive", "tzdata"],
    );
    Ok(zone.to_string())
}
