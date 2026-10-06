//! `apply` on the real Mac's layout through a fake runner: every command it runs answered as the
//! disk would answer, and the disk made to misbehave. A wipe that fails part way is still
//! followed by the check that nothing else moved, and a partition about to be formatted must be
//! where the kernel says it is.

use std::cell::RefCell;

use yantrik_install_target::apply::{self, Ran};
use yantrik_install_target::TargetSpec;

const DISK: &str = "/dev/sdz";

/// What goes wrong, if anything.
#[derive(Default, Clone)]
struct Trouble {
    wipe_fails: bool,
    /// After writing, partition 2 (APFS) starts one sector later.
    apfs_moves: bool,
    /// After writing, the disk has this GUID.
    new_guid: Option<&'static str>,
    /// The kernel's record of sdz4's start before writing, and after.
    start_before: Option<u64>,
    start_after: Option<u64>,
}

struct Disk {
    trouble: Trouble,
    written: RefCell<bool>,
    asked: RefCell<Vec<String>>,
}

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    // A disk no test machine has, so /proc/self/mounts never matches it.
    std::fs::read_to_string(path).unwrap().replace("/dev/sda", DISK).replace("\"sda", "\"sdz")
}

const START: u64 = 1_563_433_928;
const SIZE: u64 = 388_671_875;

impl Disk {
    fn new(trouble: Trouble) -> Disk {
        Disk { trouble, written: RefCell::new(false), asked: RefCell::new(vec![]) }
    }

    fn parted(&self) -> String {
        let mut json = fixture("mac-real.parted.json");
        if *self.written.borrow() {
            // `parted type 4 <Linux filesystem>`: only the target's type changed.
            let at = json.find("\"number\": 4").unwrap();
            let tail = json[at..].replacen("ebd0a0a2-b9e5-4433-87c0-68b6b72699c7", "0fc63daf-8483-4772-8e79-3d69d8477de4", 1);
            json = format!("{}{tail}", &json[..at]);
            if self.trouble.apfs_moves {
                json = json.replace("\"start\": \"409640s\"", "\"start\": \"409641s\"");
            }
            if let Some(guid) = self.trouble.new_guid {
                json = json.replace("3b0c6a2e-5d1f-4e8a-9c7b-2a4d6f8e1c35", guid);
            }
        }
        json
    }

    fn run(&self, cmd: &str, args: &[&str]) -> Ran {
        let line = format!("{cmd} {}", args.join(" "));
        self.asked.borrow_mut().push(line.clone());
        let ok = |out: &str| Ran::exited(0, out);
        let written = *self.written.borrow();
        let start = if written { self.trouble.start_after } else { self.trouble.start_before }.unwrap_or(START);
        match line.as_str() {
            l if l.starts_with("parted -j") => ok(&self.parted()),
            l if l.starts_with("lsblk -J") => ok(&fixture("mac-real.lsblk.json")),
            l if l.starts_with("lsblk -dno TRAN") => ok("sata\n"),
            l if l.starts_with("blkid -p -D -o export /dev/sdz1") => ok("TYPE=vfat\nLABEL=EFI\n"),
            l if l.starts_with("blkid -p -D -o export /dev/sdz2") => ok("TYPE=apfs\n"),
            l if l.starts_with("blkid -p -D -o export /dev/sdz3") => ok("TYPE=vfat\nLABEL=YKINSTALL\n"),
            l if l.starts_with("blkid -p -D -o export /dev/sdz4") && !written => ok("TYPE=vfat\nLABEL=YANTRIK\n"),
            l if l.starts_with("blkid -p -D -o export /dev/sdz4") => Ran::exited(2, ""),
            l if l.starts_with("mkdir") || l.starts_with("rmdir") || l.starts_with("umount") => ok(""),
            l if l.starts_with("mount -o ro") => ok(""),
            l if l.starts_with("find /run/yantrik-install-target/sdz4") => ok("d .fseventsd\0f .DS_Store\0"),
            l if l.starts_with("df -B1") => ok("Avail\n150000000\n"),
            l if l.starts_with("od -An") => {
                let zero = "00 ".repeat(16);
                ok(&format!("00 00 02 00 ee ff ff ff 01 00 00 00 ff ff ff ff {zero} {zero} {zero} 55 aa"))
            }
            l if l.starts_with("wipefs -n") => ok(""),
            l if l.starts_with("dd if=/dev/sdz4") => ok(&"\0".repeat(1024 * 1024)),
            "wipefs -a /dev/sdz4" => {
                if self.trouble.wipe_fails {
                    return Ran { code: Some(1), stdout: String::new(), stderr: "Device or resource busy".into() };
                }
                *self.written.borrow_mut() = true;
                ok("")
            }
            l if l.starts_with("parted -s /dev/sdz type 4") => ok(""),
            l if l.starts_with("partprobe") || l.starts_with("udevadm") => ok(""),
            "cat /sys/class/block/sdz4/start" => ok(&format!("{start}\n")),
            "cat /sys/class/block/sdz4/size" => ok(&format!("{SIZE}\n")),
            "test -b /dev/sdz4" => ok(""),
            _ => Ran { code: Some(1), stdout: String::new(), stderr: format!("not faked: {line}") },
        }
    }

    fn ran(&self, prefix: &str) -> bool {
        self.asked.borrow().iter().any(|l| l.starts_with(prefix))
    }

    fn apply(&self) -> Result<apply::Placed, String> {
        let shown = apply::read_table(DISK, &|c, a| self.run(c, a)).unwrap().fingerprint();
        self.asked.borrow_mut().clear();
        apply::apply(DISK, TargetSpec::Partition(4), false, true, &shown, &|c, a| self.run(c, a))
    }
}

#[test]
fn the_placeholder_is_wiped_retyped_and_handed_back_where_the_kernel_has_it() {
    let disk = Disk::new(Trouble::default());
    let placed = disk.apply().unwrap();
    assert_eq!((placed.root.as_str(), placed.esp.as_str(), placed.boot), ("/dev/sdz4", "/dev/sdz1", None));
    assert!(disk.ran("wipefs -a /dev/sdz4") && disk.ran("parted -s /dev/sdz type 4"));
    // Looked inside before writing: the placeholder was mounted read-only and listed.
    assert!(disk.ran("mount -o ro,nosuid,nodev,noexec -t vfat /dev/sdz4"));
    assert!(!disk.ran("mkfs") && !disk.ran("cryptsetup"), "apply formats nothing");
}

#[test]
fn a_target_the_kernel_has_elsewhere_is_refused_before_the_wipe() {
    let disk = Disk::new(Trouble { start_before: Some(START + 8), ..Trouble::default() });
    let e = disk.apply().unwrap_err();
    assert!(e.contains("not 1563433928+388671875") && e.contains("nothing was written"), "{e}");
    assert!(!disk.ran("wipefs -a"), "nothing was written");
}

#[test]
fn a_root_the_kernel_has_elsewhere_after_writing_is_not_formatted() {
    let disk = Disk::new(Trouble { start_after: Some(START + 8), ..Trouble::default() });
    let e = disk.apply().unwrap_err();
    assert!(e.contains("it was not formatted"), "{e}");
}

#[test]
fn a_wipe_that_fails_is_still_followed_by_the_check_and_says_its_result() {
    let disk = Disk::new(Trouble { wipe_fails: true, ..Trouble::default() });
    let e = disk.apply().unwrap_err();
    assert!(e.contains("`wipefs -a /dev/sdz4` failed") && e.contains("Device or resource busy"), "{e}");
    assert!(e.contains("Checked afterwards: every other partition on the disk is still exactly where it was"), "{e}");
    assert!(disk.ran("partprobe"), "the table was read again after the failure");
}

#[test]
fn a_partition_that_moved_or_a_rewritten_table_is_reported() {
    let disk = Disk::new(Trouble { apfs_moves: true, ..Trouble::default() });
    let e = disk.apply().unwrap_err();
    assert!(e.contains("/dev/sdz2 is not as it was"), "{e}");
    let disk = Disk::new(Trouble { new_guid: Some("99999999-8888-4777-8666-555555555555"), ..Trouble::default() });
    let e = disk.apply().unwrap_err();
    assert!(e.contains("is not the one it was"), "{e}");
}
