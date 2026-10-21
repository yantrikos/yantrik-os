//! Who the kernel vouches for in Private mode (`private_owner`): the path rule, the mount namespace,
//! the file, and the surface a listed reader is scoped to.

use std::path::Path;

use crate::private_owner::*;
use crate::server::PeerCred;

const DIR: &str = "/opt/yantrik/bin";

/// A caller as `facts_of` would describe it: installed, in this namespace, running that file.
fn installed(exe: &str) -> PeerFacts {
    PeerFacts { exe: exe.to_string(), same_mount_ns: true, same_file: true }
}

#[test]
fn the_readers_while_private_are_the_shell_the_apps_own_program_and_the_list() {
    let dir = Path::new(DIR);
    let own = Some("/opt/yantrik/bin/yantrik-notes");
    for yes in [
        "/opt/yantrik/bin/yantrik-ui",
        "/opt/yantrik/bin/yantrik-notes",
        "/opt/yantrik/bin/yantrik-notes (deleted)",
    ] {
        assert!(is_private_reader_in(yes, own, dir, "notes"), "{yes}");
    }
    for no in [
        "/opt/yantrik/bin/yantrik-email",
        "/home/p/target/release/yantrik-notes",
        "/usr/bin/python3.11",
        "yantrik-ui",
        "",
    ] {
        assert!(!is_private_reader_in(no, own, dir, "notes"), "{no}");
    }
    // An app's own program counts only installed: a test binary calling its own socket is no one.
    assert!(!is_private_reader_in("/build/t/deps/x-1a2b", Some("/build/t/deps/x-1a2b"), dir, "x"));
    assert!(!reads_while_private(std::process::id() as i32, "shell"), "this test binary is not installed");
    assert_eq!(PRIVATE_READERS, [("yantrik-term", "shell")], "an addition is a decision for a reader");
}

/// #926 review, L1: yantrik-term reads the shell's chat, and nothing of any app.
#[test]
fn yantrik_term_is_vouched_toward_the_shell_and_no_other_surface() {
    let dir = Path::new(DIR);
    let term = installed("/opt/yantrik/bin/yantrik-term");
    assert!(is_vouched(&term, Some("/opt/yantrik/bin/yantrik-ui"), dir, "shell"));
    for surface in ["notes", "email", "calendar", "files"] {
        assert!(!is_vouched(&term, Some("/opt/yantrik/bin/yantrik-notes"), dir, surface), "{surface}");
    }
}

/// #926 review, L1: `unshare -Urm` and a tmpfs over the install directory make any program read as
/// installed from inside; from here it is in another mount namespace, and is nobody.
#[test]
fn a_caller_in_another_mount_namespace_is_not_vouched() {
    let dir = Path::new(DIR);
    let mut spoofed = installed("/opt/yantrik/bin/yantrik-ui");
    assert!(is_vouched(&spoofed, None, dir, "notes"));
    spoofed.same_mount_ns = false;
    assert!(!is_vouched(&spoofed, None, dir, "notes"));
}

/// #926 review, L1: a same-named copy elsewhere fails the path, and one whose path names a
/// different file than it runs fails the file check.
#[test]
fn a_same_named_copy_is_not_vouched() {
    let dir = Path::new(DIR);
    assert!(!is_vouched(&installed("/tmp/yantrik-term"), None, dir, "shell"));
    assert!(!is_vouched(&installed("/home/p/.local/bin/yantrik-ui"), None, dir, "notes"));
    let other_file = PeerFacts { same_file: false, ..installed("/opt/yantrik/bin/yantrik-term") };
    assert!(!is_vouched(&other_file, None, dir, "shell"));
}

/// The facts are gathered from `/proc` as the doors read them: this process, in its own namespace,
/// running the file its path names.
#[cfg(target_os = "linux")]
#[test]
fn the_facts_of_this_process_are_its_own() {
    let me = facts_of(std::process::id() as i32).expect("/proc says who we are");
    assert!(me.same_mount_ns && me.same_file, "{me:?}");
    assert!(facts_of(0).is_none(), "pid 0 is nobody, and nobody is vouched for");
}

#[test]
fn the_owner_is_the_installed_app_or_the_shell_and_nothing_with_its_name_elsewhere() {
    let dir = Path::new(DIR);
    let owners = ["yantrik-email"];
    for yes in [
        "/opt/yantrik/bin/yantrik-email",
        "/opt/yantrik/bin/yantrik-ui",
        "/opt/yantrik/bin/yantrik-email (deleted)",
    ] {
        assert!(is_owner_in(yes, dir, &owners), "{yes}");
    }
    for no in [
        "/tmp/yantrik-email",
        "/home/p/target/release/yantrik-email",
        "/opt/yantrik/bin/yantrik-notes",
        "/opt/yantrik/bin/sub/yantrik-email",
        "yantrik-email",
        "/usr/bin/python3.11",
        "",
    ] {
        assert!(!is_owner_in(no, dir, &owners), "{no}");
    }
}

#[test]
fn out_of_private_mode_anyone_reads_and_in_it_an_unknown_caller_does_not() {
    assert!(content_for_owners(None, "notes.get", &["yantrik-notes"], false).is_ok());
    assert!(content_for_owners(None, "notes.get", &["yantrik-notes"], true).is_err());
    // This test binary is not in the install directory, so it is nobody's owner.
    let me = PeerCred { pid: std::process::id() as i32, uid: 0, gid: 0 };
    let why = content_for_owners(Some(me), "notes.get", &["yantrik-notes"], true).unwrap_err();
    assert!(why.contains("notes.get"), "{why}");
    assert!(content_for_owners(Some(me), "notes.get", &["yantrik-notes"], false).is_ok());
}
