//! What the first-run screens say about a Wi-Fi chip that is still waiting for its driver.
//!
//! The Broadcom BCM4331 in the Mac mini of 2012 (and kin) has no driver this image may carry:
//! `yantrik-broadcom-wifi` fetches one from Debian once the machine is online, and writes what
//! it is doing to `/run/yantrik/wifi-driver.status`, one `key=value` per line:
//!
//! ```text
//! state=needs-network
//! chip=BCM4331
//! driver=
//! message=This computer's Wi-Fi (BCM4331) needs a driver ... Connect an Ethernet cable ...
//! ```
//!
//! The installer's Welcome screen and the first-boot hardware scan show the message while there
//! is something for the person to do or wait for, and nothing once Wi-Fi works or there was
//! never a chip to worry about.
//!
//! Depends on nothing but `std`, like installer_rules.rs, so it is tested on its own.

/// Where `yantrik-broadcom-wifi` reports.
pub const STATUS_FILE: &str = "/run/yantrik/wifi-driver.status";

/// The note to show for a status file's contents: empty when there is nothing to say.
pub fn note_from(status: &str) -> String {
    let field = |key: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(key).and_then(|rest| rest.strip_prefix('=')))
            .map(str::trim)
            .unwrap_or("")
    };
    let message = field("message");
    match field("state") {
        "needs-network" | "installing" | "failed" if !message.is_empty() => {
            // One line, and not a novel, whatever ended up in the file.
            message.chars().filter(|c| !c.is_control()).take(400).collect()
        }
        _ => String::new(),
    }
}

/// Whether the status is final, so nobody needs to keep reading the file.
pub fn settled(status: &str) -> bool {
    status
        .lines()
        .any(|l| matches!(l.trim(), "state=ready" | "state=absent"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chip_waiting_for_its_driver_is_explained() {
        let s = "state=needs-network\nchip=BCM4331\ndriver=\nmessage=Connect an Ethernet cable.\n";
        assert_eq!(note_from(s), "Connect an Ethernet cable.");
        assert!(!settled(s));
        let s = "state=installing\nmessage=Installing the BCM4331 Wi-Fi driver from Debian.\n";
        assert!(note_from(s).starts_with("Installing"));
        let s = "state=failed\nmessage=Could not give BCM4331 a driver.\n";
        assert!(note_from(s).starts_with("Could not"));
    }

    #[test]
    fn nothing_is_said_once_wifi_works_or_there_is_no_chip() {
        let ready = "state=ready\nchip=BCM4331\ndriver=wl\nmessage=BCM4331 Wi-Fi is working (wl).\n";
        assert_eq!(note_from(ready), "");
        assert!(settled(ready));
        let absent = "state=absent\nchip=\ndriver=\nmessage=No Broadcom Wi-Fi here that needs a driver.\n";
        assert_eq!(note_from(absent), "");
        assert!(settled(absent));
        assert_eq!(note_from(""), "");
        assert!(!settled(""));
        // A key that merely starts like another is not that key.
        assert_eq!(note_from("states=needs-network\nmessage=x\n"), "");
        // No message, nothing to show.
        assert_eq!(note_from("state=needs-network\n"), "");
    }
}
