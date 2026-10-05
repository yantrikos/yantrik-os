//! The Mac mini this is for, as it really is: a 1 TB disk with its 209.7 MB EFI partition at
//! sector 40, APFS shrunk to 792 GB, the 8 GB YKINSTALL partition the installer runs from, and
//! the 199 GB FAT32 partition labelled YANTRIK to install into; and beside it a decoy, a second
//! disk with a FAT partition labelled YANTRIK-backup that holds someone's files.

mod common;

use common::{on_usb, table};
use yantrik_install_target::classify::{self, preselect};
use yantrik_install_target::{plan, Checked, DiskTable, Segment, TargetSpec};

fn mac() -> DiskTable {
    table("mac-real", "mac-real")
}

/// The decoy as reading it finds it: its YANTRIK-backup partition holds photos.
fn decoy() -> DiskTable {
    let mut t = table("mac-real-decoy", "mac-real-decoy");
    t.parts[1].checked = Checked::Holds(vec!["Photos".into(), "Photos/IMG_0001.HEIC".into()]);
    t
}

fn find<'a>(s: &'a [Segment], id: &str) -> &'a Segment {
    s.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("no {id}"))
}

#[test]
fn the_mac_is_read_as_disk_utility_shows_it() {
    let t = mac();
    assert_eq!((t.parts[0].run.start, t.parts.len()), (40, 4));
    let s = classify::segments(&t, true);
    let sizes: Vec<(&str, &str, &str)> = s.iter().map(|s| (s.id.as_str(), s.kind.as_str(), s.size.as_str())).collect();
    assert_eq!(
        sizes,
        [("sda1", "esp", "209.7 MB"), ("sda2", "macos", "792 GB"), ("sda3", "medium", "8 GB"), ("sda4", "placeholder", "199 GB")],
        "the 726 MB left at the end is not drawn"
    );
    let eligible: Vec<&str> = s.iter().filter(|s| s.eligible).map(|s| s.id.as_str()).collect();
    assert_eq!(eligible, ["sda4"]);
    assert!(find(&s, "sda3").reason.contains("running from"));
    assert_eq!(find(&s, "sda4").card, "Install into /dev/sda4 (199 GB, YANTRIK, FAT32); nothing else changes");
    assert_eq!(preselect(&s).as_deref(), Some("sda4"));
}

#[test]
fn the_decoy_is_never_offered_or_chosen() {
    let mut all = classify::segments(&mac(), true);
    let d = classify::segments(&decoy(), true);
    let backup = find(&d, "sdb2");
    assert!(!backup.eligible && backup.kind == "other", "{backup:?}");
    assert!(backup.reason.contains("labelled YANTRIK-backup"), "{}", backup.reason);
    all.extend(d.clone());
    assert_eq!(preselect(&all).as_deref(), Some("sda4"));

    // Even emptied, YANTRIK-backup is not the label a placeholder carries.
    let mut empty = decoy();
    empty.parts[1].checked = Checked::Empty;
    assert!(!find(&classify::segments(&empty, true), "sdb2").eligible);

    // Relabelled exactly YANTRIK but still holding the photos: refused, and says what it holds.
    let mut relabelled = decoy();
    relabelled.parts[1].label = "YANTRIK".into();
    let r = classify::segments(&relabelled, true);
    let y = find(&r, "sdb2");
    assert!(!y.eligible && y.reason.contains("holds files (Photos, Photos/IMG_0001.HEIC)"), "{}", y.reason);
    let e = plan(&relabelled, TargetSpec::Partition(2), false, true, &relabelled.fingerprint()).unwrap_err();
    assert!(e.contains("holds files"), "{e}");

    // Relabelled and emptied, on USB: offered, never chosen; the Mac's own placeholder is.
    let mut usb = on_usb(relabelled);
    usb.parts[1].checked = Checked::Empty;
    let u = classify::segments(&usb, true);
    assert!(find(&u, "sdb2").eligible);
    let mut both = classify::segments(&mac(), true);
    both.extend(u.clone());
    assert_eq!(preselect(&both).as_deref(), Some("sda4"));
    assert_eq!(preselect(&u), None, "alone on USB, nothing is chosen");
}

#[test]
fn installing_into_the_placeholder_names_only_it() {
    let t = mac();
    let p = plan(&t, TargetSpec::Partition(4), false, true, &t.fingerprint()).unwrap();
    let cmds: Vec<String> = p.commands().into_iter().map(|c| c.join(" ")).collect();
    assert_eq!(
        cmds,
        [
            "wipefs -a /dev/sda4",
            "parted -s /dev/sda type 4 0fc63daf-8483-4772-8e79-3d69d8477de4",
            "mkfs.ext4 -q -F -L YANTRIK /dev/sda4",
        ]
    );
    assert_eq!((p.esp_path.as_str(), p.region), ("/dev/sda1", t.parts[3].run));
    // The installer's own partition, mounted where the live system runs from, is never a target.
    let e = plan(&t, TargetSpec::Partition(3), false, true, &t.fingerprint()).unwrap_err();
    assert!(e.contains("running from"), "{e}");
}

#[test]
fn booted_from_usb_ykinstall_is_just_a_fat_partition_and_still_refused() {
    let mut t = mac();
    t.parts[2].mountpoint.clear();
    let s = classify::segments(&t, true);
    let ins = find(&s, "sda3");
    assert!(!ins.eligible && ins.reason.contains("labelled YKINSTALL"), "{}", ins.reason);
}
