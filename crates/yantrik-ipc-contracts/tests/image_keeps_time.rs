//! The image keeps its clock (#1171).
//!
//! `debootstrap --variant=minbase` pulls no time daemon, so an image that names none drifts until
//! TLS certificates and update signatures stop verifying. The README says the clock is synced
//! through `systemd-timesyncd` to Debian's pool, so the image build must install and enable it,
//! must not make that optional, and must not point it at a server of ours (a pool we ran would be
//! a phone-home).

use std::path::PathBuf;

fn script() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join("deploy/yantrik-os/build-debian-iso.sh")).expect("the image build script")
}

#[test]
fn the_image_installs_and_enables_the_time_daemon_and_it_is_not_optional() {
    let script = script();
    let install = script
        .lines()
        .find(|l| l.contains("apt-get install") && l.contains("systemd-timesyncd"))
        .expect("the image build installs systemd-timesyncd");
    assert!(!install.contains("|| true") && !install.contains("2>/dev/null"), "a missing clock daemon must fail the build: {install}");
    assert!(
        script.lines().any(|l| l.trim_start().starts_with("systemctl enable systemd-timesyncd")),
        "and it is enabled"
    );
}

#[test]
fn the_package_default_servers_are_used_and_none_of_ours_is_configured() {
    let script = script();
    for configured in ["NTP=", "FallbackNTP=", "timesyncd.conf", "chrony", "ntpsec"] {
        assert!(
            !script.lines().filter(|l| !l.trim_start().starts_with('#')).any(|l| l.contains(configured)),
            "the image build configures a time server ({configured}); the Debian pool is the package default"
        );
    }
}
