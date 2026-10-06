//! Every way reading a partition can fail, each kept rather than offered: blkid, wipefs and the
//! bytes for an empty partition; mounting and listing for a placeholder; sector 0 for the disk.

use super::*;
use crate::classify::{kind_of, Kind, UNREADABLE};
use crate::table::Run;
use std::cell::RefCell;

/// A runner that answers by the first rule whose text starts the command line, and fails
/// anything it was not told about. Remembers every command line it was asked.
struct Fake {
    rules: Vec<(String, Ran)>,
    asked: RefCell<Vec<String>>,
}

impl Fake {
    fn new(rules: &[(&str, Ran)]) -> Fake {
        Fake { rules: rules.iter().map(|(p, r)| (p.to_string(), r.clone())).collect(), asked: RefCell::new(vec![]) }
    }
    fn run(&self, cmd: &str, args: &[&str]) -> Ran {
        let line = format!("{cmd} {}", args.join(" "));
        self.asked.borrow_mut().push(line.clone());
        self.rules
            .iter()
            .find(|(p, _)| line.starts_with(p.as_str()))
            .map(|(_, r)| r.clone())
            .unwrap_or(Ran { code: Some(1), stdout: String::new(), stderr: format!("not faked: {line}") })
    }
    fn asked(&self, prefix: &str) -> bool {
        self.asked.borrow().iter().any(|l| l.starts_with(prefix))
    }
}

const MIB: usize = 1024 * 1024;

fn zeros() -> Ran {
    Ran::exited(0, &"\0".repeat(MIB))
}

fn blank_partition() -> Part {
    Part {
        number: 5,
        run: Run { start: 2048, end: 2048 + 40_000_000 - 1 },
        type_uuid: classify::guid::MS_BASIC_DATA.into(),
        uuid: String::new(),
        name: String::new(),
        flags: vec![],
        path: "/dev/sdz5".into(),
        fstype: String::new(),
        label: String::new(),
        mountpoint: String::new(),
        checked: Checked::NotLooked,
    }
}

/// What reading `/dev/sdz5` with these answers makes of it.
fn probed(rules: &[(&str, Ran)]) -> Part {
    let fake = Fake::new(rules);
    let mut p = blank_partition();
    probe_filesystem(&mut p, 512, &|c, a| fake.run(c, a));
    p
}

fn unreadable(p: &Part, says: &str) {
    assert!(matches!(&p.checked, Checked::Unreadable(why) if why.contains(says)), "{:?}", p.checked);
    assert_eq!(kind_of(p), Kind::Other(UNREADABLE.into()));
}

#[test]
fn a_partition_is_empty_only_when_blkid_wipefs_and_its_bytes_all_say_so() {
    let p = probed(&[("blkid -p -D", Ran::exited(2, "")), ("wipefs -n", Ran::exited(0, "")), ("dd if=/dev/sdz5", zeros())]);
    assert_eq!(p.checked, Checked::Empty);
    assert_eq!(kind_of(&p), Kind::Unformatted);
}

#[test]
fn blkid_failing_any_other_way_is_not_empty() {
    // Ambivalent (8), an I/O or usage error (4, 1), killed: none is "nothing found".
    for code in [8, 4, 1] {
        let p = probed(&[
            ("blkid -p -D", Ran { code: Some(code), stdout: String::new(), stderr: "boom".into() }),
            ("wipefs -n", Ran::exited(0, "")),
            ("dd if=/dev/sdz5", zeros()),
        ]);
        unreadable(&p, &format!("exit {code}"));
    }
    let p = probed(&[("blkid -p -D", Ran { code: None, stdout: String::new(), stderr: "signal".into() })]);
    unreadable(&p, "did not finish");
    // It answered, but with no filesystem type: something is there.
    let p = probed(&[("blkid -p -D", Ran::exited(0, "DEVNAME=/dev/sdz5\nPTTYPE=dos\n"))]);
    unreadable(&p, "not a filesystem");
}

#[test]
fn nothing_found_by_blkid_but_a_signature_for_wipefs_is_not_empty() {
    let p = probed(&[("blkid -p -D", Ran::exited(2, "")), ("wipefs -n", Ran::exited(0, "vfat\nvfat\n"))]);
    unreadable(&p, "wipefs finds vfat");
}

#[test]
fn wipefs_that_cannot_read_the_device_is_not_empty() {
    // blkid also exits 2 for a device that is not there; wipefs then fails.
    let p = probed(&[
        ("blkid -p -D", Ran::exited(2, "")),
        ("wipefs -n", Ran { code: Some(1), stdout: String::new(), stderr: "No such file or directory".into() }),
    ]);
    unreadable(&p, "wipefs -n could not read it");
}

#[test]
fn a_first_or_last_mib_with_anything_in_it_is_not_empty() {
    let mut dirty = vec![0u8; MIB];
    dirty[510] = 0x55;
    let dirty = Ran::exited(0, &String::from_utf8(dirty).unwrap());
    let p = probed(&[("blkid -p -D", Ran::exited(2, "")), ("wipefs -n", Ran::exited(0, "")), ("dd if=/dev/sdz5 bs=1048576 count=1 skip=0 ", dirty.clone())]);
    unreadable(&p, "first MiB is not blank");
    let p = probed(&[
        ("blkid -p -D", Ran::exited(2, "")),
        ("wipefs -n", Ran::exited(0, "")),
        ("dd if=/dev/sdz5 bs=1048576 count=1 skip=0 ", zeros()),
        ("dd if=/dev/sdz5", dirty),
    ]);
    unreadable(&p, "last MiB is not blank");
    // Bytes that are not UTF-8 read as U+FFFD, never as a NUL.
    let p = probed(&[("blkid -p -D", Ran::exited(2, "")), ("wipefs -n", Ran::exited(0, "")), ("dd if=/dev/sdz5", Ran::exited(0, "\u{fffd}"))]);
    unreadable(&p, "first MiB is not blank");
}

#[test]
fn a_mib_that_could_not_be_read_whole_is_not_empty() {
    let short = Ran::exited(0, &"\0".repeat(MIB - 1));
    let p = probed(&[("blkid -p -D", Ran::exited(2, "")), ("wipefs -n", Ran::exited(0, "")), ("dd if=/dev/sdz5", short)]);
    unreadable(&p, "first MiB is not blank");
    let p = probed(&[
        ("blkid -p -D", Ran::exited(2, "")),
        ("wipefs -n", Ran::exited(0, "")),
        ("dd if=/dev/sdz5", Ran { code: Some(1), stdout: String::new(), stderr: "Input/output error".into() }),
    ]);
    unreadable(&p, "could not be read: Input/output error");
}

#[test]
fn lsblk_and_blkid_disagreeing_is_not_empty() {
    let fake = Fake::new(&[("blkid -p -D", Ran::exited(2, "")), ("wipefs -n", Ran::exited(0, "")), ("dd", zeros())]);
    let mut p = blank_partition();
    p.fstype = "vfat".into();
    p.label = "YANTRIK".into();
    probe_filesystem(&mut p, 512, &|c, a| fake.run(c, a));
    unreadable(&p, "lsblk says vfat");
}

#[test]
fn blkid_overrules_a_stale_lsblk() {
    let fake = Fake::new(&[("blkid -p -D", Ran::exited(0, "DEVNAME=/dev/sdz5\nLABEL=PHOTOS\nTYPE=exfat\n"))]);
    let mut p = blank_partition();
    p.fstype = "vfat".into();
    p.label = "YANTRIK".into();
    probe_filesystem(&mut p, 512, &|c, a| fake.run(c, a));
    assert_eq!((p.fstype.as_str(), p.label.as_str()), ("exfat", "PHOTOS"));
    assert_eq!(kind_of(&p), Kind::Other("exfat".into()));
}

fn placeholder() -> Part {
    Part { fstype: "vfat".into(), label: "YANTRIK".into(), ..blank_partition() }
}

#[test]
fn a_placeholder_with_only_macos_housekeeping_is_empty_and_is_unmounted_after() {
    let listing = "d .fseventsd\0f .fseventsd/fseventsd-uuid\0d .Spotlight-V100\0f .Spotlight-V100/Store-V2/x\0\
                   d .Trashes\0d .Trashes/501\0f .DS_Store\0f .Trashes/501/.DS_Store\0f ._.Trashes\0";
    let fake = Fake::new(&[("mkdir", Ran::exited(0, "")), ("mount -o ro", Ran::exited(0, "")), ("find", Ran::exited(0, listing)), ("umount", Ran::exited(0, "")), ("rmdir", Ran::exited(0, ""))]);
    let p = placeholder();
    let checked = look_inside(&p, &|c, a| fake.run(c, a));
    assert_eq!(checked, Checked::Empty);
    assert!(fake.asked("mount -o ro,nosuid,nodev,noexec -t vfat /dev/sdz5 /run/yantrik-install-target/sdz5"));
    assert!(fake.asked("umount /run/yantrik-install-target/sdz5"));
    assert_eq!(kind_of(&Part { checked, ..p }), Kind::Placeholder);
}

#[test]
fn a_placeholder_holding_anything_else_is_kept_with_what_it_holds() {
    let listing = "d .fseventsd\0d Photos\0f Photos/IMG_0001.HEIC\0d .Trashes\0d .Trashes/501\0f .Trashes/501/tax-2025.pdf\0";
    let fake = Fake::new(&[("mkdir", Ran::exited(0, "")), ("mount -o ro", Ran::exited(0, "")), ("find", Ran::exited(0, listing)), ("umount", Ran::exited(0, ""))]);
    let checked = look_inside(&placeholder(), &|c, a| fake.run(c, a));
    assert_eq!(checked, Checked::Holds(vec!["Photos".into(), "Photos/IMG_0001.HEIC".into(), ".Trashes/501/tax-2025.pdf".into()]));
    let p = Part { checked, ..placeholder() };
    assert_eq!(kind_of(&p), Kind::Other("vfat".into()));
}

#[test]
fn a_placeholder_that_cannot_be_mounted_listed_or_unmounted_is_kept() {
    let ok0 = Ran::exited(0, "");
    let fail = Ran { code: Some(32), stdout: String::new(), stderr: "wrong fs type".into() };
    let cases: [(&[(&str, Ran)], &str); 3] = [
        (&[("mkdir", ok0.clone()), ("mount -o ro", fail.clone())], "could not be mounted read-only"),
        (&[("mkdir", ok0.clone()), ("mount -o ro", ok0.clone()), ("find", fail.clone()), ("umount", ok0.clone())], "could not be listed"),
        (&[("mkdir", ok0.clone()), ("mount -o ro", ok0.clone()), ("find", Ran::exited(0, "")), ("umount", fail.clone())], "left mounted"),
    ];
    for (rules, says) in cases {
        let fake = Fake::new(rules);
        let checked = look_inside(&placeholder(), &|c, a| fake.run(c, a));
        assert!(matches!(&checked, Checked::Unreadable(why) if why.contains(says)), "{checked:?}");
    }
    // Already mounted (by a person, in the live session): read where it is, and left mounted.
    let fake = Fake::new(&[("find /media/YANTRIK", Ran::exited(0, ""))]);
    let mounted = Part { mountpoint: "/media/YANTRIK".into(), ..placeholder() };
    assert_eq!(look_inside(&mounted, &|c, a| fake.run(c, a)), Checked::Empty);
    assert!(!fake.asked("mount") && !fake.asked("umount"));
}

#[test]
fn only_a_plain_protective_mbr_is_accepted() {
    let entry = |ty: &str| format!("00 00 02 00 {ty} ff ff ff 01 00 00 00 ff ff ff ff");
    let zero = "00 ".repeat(16);
    let protective = format!("{} {zero} {zero} {zero} 55 aa", entry("ee"));
    assert_eq!(mbr_problem("/dev/sda", &protective), None);
    // Boot Camp's hybrid: the protective entry, then FAT32 and NTFS entries.
    let hybrid = format!("{} {} {} {zero} 55 aa", entry("ee"), entry("0c"), entry("07"));
    let e = mbr_problem("/dev/sda", &hybrid).unwrap();
    assert!(e.contains("hybrid MBR") && e.contains("0C, 07"), "{e}");
    let unsigned = format!("{} {zero} {zero} {zero} 00 00", entry("ee"));
    assert!(mbr_problem("/dev/sda", &unsigned).unwrap().contains("no readable MBR"));
    let no_ee = format!("{zero} {zero} {zero} {zero} 55 aa");
    assert!(mbr_problem("/dev/sda", &no_ee).unwrap().contains("no protective MBR entry"));
    assert!(mbr_problem("/dev/sda", "").is_some());
}

#[test]
fn macos_housekeeping_is_told_from_a_persons_files() {
    let listing = "d .fseventsd\0f ._photo\0f .DS_Store\0d .Trashes\0f notes.txt\0d .TemporaryItems\0";
    assert_eq!(foreign_files(listing), ["notes.txt"]);
    assert!(foreign_files("").is_empty());
    // Only the volume's own icon, at its top: one deeper is a file someone put there.
    assert_eq!(foreign_files("f Icons/.VolumeIcon.icns\0"), ["Icons/.VolumeIcon.icns"]);
}

#[test]
fn a_fresh_placeholder_macos_made_and_mounted_is_empty() {
    // What Disk Utility and Finder leave on a FAT volume named YANTRIK once it has been mounted.
    let listing = "d .fseventsd\0f .fseventsd/fseventsd-uuid\0f .fseventsd/0000000000a1b2c3\0\
                   d .Spotlight-V100\0d .Spotlight-V100/Store-V2\0f .Spotlight-V100/VolumeConfiguration.plist\0\
                   d .TemporaryItems\0d .TemporaryItems/folders.501\0f .TemporaryItems/folders.501/Cleanup At Startup\0\
                   f .VolumeIcon.icns\0f ._.VolumeIcon.icns\0f .DS_Store\0f ._.DS_Store\0d .Trashes\0d .Trashes/501\0";
    assert!(foreign_files(listing).is_empty(), "{:?}", foreign_files(listing));
    let fake = Fake::new(&[("mkdir", Ran::exited(0, "")), ("mount -o ro", Ran::exited(0, "")), ("find", Ran::exited(0, listing)), ("umount", Ran::exited(0, ""))]);
    assert_eq!(look_inside(&placeholder(), &|c, a| fake.run(c, a)), Checked::Empty);
}
