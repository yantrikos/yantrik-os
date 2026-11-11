//! Whether the Hermes this image ships is the one an install on this machine sets up.
//!
//! The person's desktop cannot see the mind account's home, so it cannot compare homes. It sees
//! what the install itself looks at: the root-owned marker the image build writes, and the
//! hand-over (#1076) that moves the install into the mind account, which is the condition
//! `harnesses/lib/install/hermes.sh` tests before it runs `sudo yantrik-update harness install`:
//! the `yantrik-mind` account exists, and the updater carries the `harness` command (it does only
//! when `harness-isolation.sh` sits beside it). Read from the same live files rather than from a
//! fact written at build time, so a later update or rollback of the updater changes the answer
//! with it. Without the hand-over the install runs as the person, in the person's home, and
//! downloads, whatever the image carries.

use std::path::Path;

/// Written by the image build (`deploy/yantrik-os/bake-hermes.sh`) when Hermes is in the image: a
/// root-owned file outside the mind account's home.
const BAKED_MARKER: &str = "opt/yantrik/hermes/BAKED";
/// The mind account the install is handed over to.
const MIND_ACCOUNT: &str = "yantrik-mind";
/// Beside `yantrik-update`: the updater has the `harness` command only when this is there.
const HAND_OVER: &str = "opt/yantrik/bin/harness-isolation.sh";

/// The image carries Hermes and the install on this machine runs in the home it was baked into.
pub fn baked() -> bool {
    baked_in(Path::new("/"))
}

/// [`baked`] on the machine rooted at `root`.
pub fn baked_in(root: &Path) -> bool {
    root.join(BAKED_MARKER).is_file() && root.join(HAND_OVER).is_file() && has_account(&root.join("etc/passwd"))
}

fn has_account(passwd: &Path) -> bool {
    std::fs::read_to_string(passwd)
        .map(|text| text.lines().any(|line| line.split(':').next() == Some(MIND_ACCOUNT)))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A machine's files, with only the named ones present.
    fn machine(marker: bool, account: bool, hand_over: bool) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let put = |path: &str, text: &str| {
            let at = root.path().join(path);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(at, text).unwrap();
        };
        put("etc/passwd", "root:x:0:0::/root:/bin/sh\npranab:x:1000:1000::/home/pranab:/bin/bash\n");
        if marker {
            put(BAKED_MARKER, "38d3dbce\n");
        }
        if account {
            put("etc/passwd", "root:x:0:0::/root:/bin/sh\nyantrik-mind:x:999:999::/var/lib/yantrik-mind:/usr/sbin/nologin\n");
        }
        if hand_over {
            put(HAND_OVER, "");
        }
        root
    }

    /// The input the old test never used: files the person's desktop can read. Only the marker
    /// with the hand-over and the account behind it counts.
    #[test]
    fn the_bake_counts_when_the_marker_and_the_hand_over_are_there() {
        assert!(baked_in(machine(true, true, true).path()));
    }

    #[test]
    fn the_marker_alone_promises_nothing() {
        assert!(!baked_in(machine(true, false, false).path()), "an image before #1076");
        assert!(!baked_in(machine(true, true, false).path()), "an account but an updater with no `harness`");
        assert!(!baked_in(machine(true, false, true).path()), "no account to hand over to");
    }

    #[test]
    fn a_machine_without_the_bake_is_not_baked() {
        assert!(!baked_in(machine(false, true, true).path()));
        assert!(!baked_in(Path::new("/nonexistent-root-for-the-test")));
    }

    #[test]
    fn an_account_is_matched_by_its_whole_name() {
        let root = machine(true, false, true);
        std::fs::write(root.path().join("etc/passwd"), "yantrik-mind-old:x:1:1::/:/bin/sh\n").unwrap();
        assert!(!baked_in(root.path()));
    }
}
