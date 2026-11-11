//! Whether the Hermes this image ships is the one an install on this machine sets up.
//!
//! The person's desktop cannot see the mind account's home, so it cannot compare homes. It sees
//! what the install itself looks at, all of it root's and none of it the person's to change:
//!
//! - the marker the image build writes when Hermes is in the image;
//! - the hand-over (#1076) that moves the install into the mind account, which is the condition
//!   `harnesses/lib/install/hermes.sh` tests before it runs `sudo yantrik-update harness install`:
//!   the `yantrik-mind` account exists, and the updater is executable and carries the `harness`
//!   command (it does only when `harness-isolation.sh` and the two files it sources sit beside it);
//! - the shipped installer itself, which hands over only when it carries `YANTRIK_HARNESS_ISOLATED`
//!   (the test `hi_has_installer` makes in the updater): an OS rollback to an installer from before
//!   the hand-over leaves every other input in place and the install downloading again;
//! - the note a failed move leaves for root (`hi_note_failure`): a move that failed was cleaned up
//!   by deleting the baked home it had been handed, so what the marker promises is gone.
//!
//! Read from the same live files rather than from a fact written at build time, so a later update
//! or rollback changes the answer with it. Without the hand-over the install runs as the person, in
//! the person's home, and downloads, whatever the image carries.

use std::path::Path;

/// Written by the image build (`deploy/yantrik-os/bake-hermes.sh`) when Hermes is in the image: a
/// root-owned file outside the mind account's home.
const BAKED_MARKER: &str = "opt/yantrik/hermes/BAKED";
/// The mind account the install is handed over to.
const MIND_ACCOUNT: &str = "yantrik-mind";
/// The updater: `hermes.sh` hands over only when it is executable.
const UPDATER: &str = "opt/yantrik/bin/yantrik-update";
/// Beside the updater: it has the `harness` command only when these are there (`hi_parts_ok`).
const HAND_OVER: [&str; 3] = [
    "opt/yantrik/bin/harness-isolation.sh",
    "opt/yantrik/bin/harness-isolation-room.sh",
    "opt/yantrik/bin/harness-sandbox.sh",
];
/// The installer an install runs, and the word in it that means it hands over.
const INSTALLER: &str = "opt/yantrik/share/harnesses/lib/install/hermes.sh";
const INSTALLER_HANDS_OVER: &str = "YANTRIK_HARNESS_ISOLATED";
/// What a failed move leaves (0644, root's, in `HI_STATE`): there while the move stays failed.
const FAILED_NOTE: &str = "var/lib/yantrik-harness/hermes.failed";

/// The image carries Hermes and the install on this machine runs in the home it was baked into.
pub fn baked() -> bool {
    baked_in(Path::new("/"))
}

/// [`baked`] on the machine rooted at `root`.
pub fn baked_in(root: &Path) -> bool {
    root.join(BAKED_MARKER).is_file()
        && has_account(&root.join("etc/passwd"))
        && is_executable(&root.join(UPDATER))
        && HAND_OVER.iter().all(|part| root.join(part).is_file())
        && std::fs::read_to_string(root.join(INSTALLER)).is_ok_and(|text| text.contains(INSTALLER_HANDS_OVER))
        // Any entry at all, a link included: the updater's own reader refuses a link, and an
        // unreadable state is not one to promise from.
        && std::fs::symlink_metadata(root.join(FAILED_NOTE)).is_err()
}

fn has_account(passwd: &Path) -> bool {
    std::fs::read_to_string(passwd)
        .map(|text| text.lines().any(|line| line.split(':').next() == Some(MIND_ACCOUNT)))
        .unwrap_or(false)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCOUNT_LINE: &str = "yantrik-mind:x:999:999::/var/lib/yantrik-mind:/usr/sbin/nologin\n";

    fn put(root: &Path, path: &str, text: &str) {
        let at = root.join(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, text).unwrap();
    }

    /// A machine that has every input: what an image with this PR and #1076 is on first boot.
    fn baked_machine() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        put(root.path(), "etc/passwd", &format!("root:x:0:0::/root:/bin/sh\n{ACCOUNT_LINE}"));
        put(root.path(), BAKED_MARKER, "38d3dbce\n");
        put(root.path(), UPDATER, "#!/bin/sh\n");
        make_executable(&root.path().join(UPDATER));
        for part in HAND_OVER {
            put(root.path(), part, "");
        }
        put(root.path(), INSTALLER, "case \"${YANTRIK_HARNESS_ISOLATED:-}\" in\n");
        root
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(not(unix))]
    fn make_executable(_: &Path) {}

    #[test]
    fn the_bake_counts_when_every_input_is_there() {
        assert!(baked_in(baked_machine().path()));
    }

    #[test]
    fn each_input_missing_alone_means_not_baked() {
        let missing = [BAKED_MARKER, UPDATER, HAND_OVER[0], HAND_OVER[1], HAND_OVER[2], INSTALLER];
        for path in missing {
            let root = baked_machine();
            std::fs::remove_file(root.path().join(path)).unwrap();
            assert!(!baked_in(root.path()), "without {path}");
        }
        let root = baked_machine();
        put(root.path(), "etc/passwd", "root:x:0:0::/root:/bin/sh\n");
        assert!(!baked_in(root.path()), "no account to hand over to");
    }

    #[test]
    fn a_machine_without_the_bake_is_not_baked() {
        assert!(!baked_in(Path::new("/nonexistent-root-for-the-test")));
        let root = baked_machine();
        std::fs::remove_file(root.path().join(BAKED_MARKER)).unwrap();
        assert!(!baked_in(root.path()), "an image built without Hermes");
    }

    #[test]
    fn an_account_is_matched_by_its_whole_name() {
        let root = baked_machine();
        put(root.path(), "etc/passwd", "yantrik-mind-old:x:1:1::/:/bin/sh\n");
        assert!(!baked_in(root.path()));
    }

    /// Review of #1169, L-1: a rollback to an installer from before the hand-over leaves the
    /// marker, the account and the updater, and the install downloads again.
    #[test]
    fn an_installer_that_does_not_hand_over_is_not_baked() {
        let root = baked_machine();
        put(root.path(), INSTALLER, "say \"fetching Hermes\"\n");
        assert!(!baked_in(root.path()));
    }

    #[cfg(unix)]
    #[test]
    fn an_updater_that_cannot_run_is_not_baked() {
        use std::os::unix::fs::PermissionsExt;
        let root = baked_machine();
        std::fs::set_permissions(root.path().join(UPDATER), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!baked_in(root.path()), "hermes.sh tests -x on it");
    }

    /// Review of #1169, H-1: a move that failed deleted the baked home (and said so in a note for
    /// root), and every later press would still have been told "from this computer".
    #[test]
    fn a_noted_failed_move_means_the_baked_home_is_gone() {
        let root = baked_machine();
        put(root.path(), FAILED_NOTE, "failed 2026-11-10T01:00:00Z could not fetch the embedder\n");
        assert!(!baked_in(root.path()));
        std::fs::remove_file(root.path().join(FAILED_NOTE)).unwrap();
        assert!(baked_in(root.path()), "a success clears the note, and the bake counts again");
    }
}
