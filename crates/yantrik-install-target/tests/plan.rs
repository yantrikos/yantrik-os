//! The planner over real layouts: the Mac mini Late 2012 it is for (1 TB GPT disk, a 209.7 MB EFI
//! partition, an APFS container of 1000 GB, then shrunk to 790 GB beside a 200 GB FAT32 YANTRIK
//! placeholder and an 8 GB YANTRIK-INS partition holding the installer, or beside free space),
//! and a PC with Windows and Linux. The fixtures are `parted -j unit s print free` and `lsblk -J`,
//! as `read_table` leaves them (common::looked) unless a test says otherwise. The layout of the
//! Mac as it really is, and its decoy disk, are tests/mac_real.rs.

mod common;

use common::{on_usb, parsed, table};
use yantrik_install_target::classify::{self, preselect};
use yantrik_install_target::plan::check;
use yantrik_install_target::{parse_target_id, plan, Checked, DiskTable, PartRef, Step, TargetSpec};

fn mac_full() -> DiskTable {
    table("mac-apfs-full", "mac-apfs-full")
}
fn mac_placeholders() -> DiskTable {
    table("mac-shrunk-placeholders", "mac-shrunk-placeholders")
}
fn mac_placeholders_usb_boot() -> DiskTable {
    table("mac-shrunk-placeholders", "mac-shrunk-placeholders-usb-boot")
}
fn mac_free() -> DiskTable {
    table("mac-shrunk-free", "mac-shrunk-free")
}
fn pc() -> DiskTable {
    table("pc-windows-linux", "pc-windows-linux")
}

fn all() -> Vec<DiskTable> {
    vec![mac_full(), mac_placeholders(), mac_placeholders_usb_boot(), mac_free(), pc()]
}

fn words(p: &yantrik_install_target::Plan) -> Vec<String> {
    p.commands().into_iter().map(|c| c.join(" ")).collect()
}

#[test]
fn the_mac_is_read_as_macos_shows_it() {
    let t = mac_full();
    assert_eq!(t.sectors, 1_953_525_168);
    assert_eq!(t.parts.len(), 2);
    let s = classify::segments(&t, true);
    let esp = s.iter().find(|s| s.id == "sda1").unwrap();
    assert_eq!((esp.kind.as_str(), esp.size.as_str(), esp.kept, esp.eligible), ("esp", "209.7 MB", true, false));
    let apfs = s.iter().find(|s| s.id == "sda2").unwrap();
    assert_eq!((apfs.kind.as_str(), apfs.title.as_str(), apfs.size.as_str()), ("macos", "macOS (APFS)", "1000 GB"));
    assert!(apfs.kept && !apfs.eligible);
    assert!(s.iter().all(|s| !s.eligible), "nothing to install into until macOS makes room");
    assert_eq!(preselect(&s), None);
}

#[test]
fn the_yantrik_placeholder_is_offered_and_chosen_first() {
    let t = mac_placeholders();
    let s = classify::segments(&t, true);
    let y = s.iter().find(|s| s.id == "sda3").unwrap();
    assert_eq!((y.kind.as_str(), y.title.as_str(), y.size.as_str()), ("placeholder", "YANTRIK (FAT32)", "200 GB"));
    assert!(y.eligible, "{}", y.reason);
    assert_eq!(y.sentence, "Yantrik OS will be installed into /dev/sda3 (200 GB). Nothing else on this disk changes.");
    // The partition the installer is running from, labelled YANTRIK-INS: never.
    let ins = s.iter().find(|s| s.id == "sda4").unwrap();
    assert_eq!(ins.kind, "medium");
    assert!(!ins.eligible && ins.reason.contains("running from"), "{}", ins.reason);
    assert_eq!(s.iter().find(|s| s.id == "sda2").unwrap().size, "790 GB");
    assert_eq!(preselect(&s).as_deref(), Some("sda3"));
    assert_eq!(y.card, "Install into /dev/sda3 (200 GB, YANTRIK, FAT32); nothing else changes");

    // Booted from a USB stick instead, YANTRIK-INS is not mounted, and still no placeholder: its
    // label only starts like one.
    let s = classify::segments(&mac_placeholders_usb_boot(), true);
    let ins = s.iter().find(|s| s.id == "sda4").unwrap();
    assert_eq!(ins.kind, "other");
    assert!(!ins.eligible && ins.reason.contains("labelled YANTRIK-INS"), "{}", ins.reason);
    assert_eq!(preselect(&s).as_deref(), Some("sda3"));
    // The same disk on USB: offered, but never chosen for the person.
    let s = classify::segments(&on_usb(mac_placeholders()), true);
    assert!(s.iter().find(|s| s.id == "sda3").unwrap().eligible);
    assert_eq!(preselect(&s), None);
}

#[test]
fn a_table_nobody_read_offers_nothing() {
    // Parsed from parted and lsblk alone: the placeholder was never looked inside, the EFI
    // partition's room never measured, sector 0 never read.
    for t in [parsed("mac-shrunk-placeholders", "mac-shrunk-placeholders"), parsed("mac-shrunk-free", "mac-shrunk-free")] {
        let s = classify::segments(&t, true);
        assert!(s.iter().all(|s| !s.eligible), "{:?}", s.iter().filter(|s| s.eligible).map(|s| &s.id).collect::<Vec<_>>());
        assert_eq!(preselect(&s), None);
        assert!(classify::disk_problem(&t, true).unwrap().contains("not read closely enough"));
    }
    // Probed, but the placeholder itself not looked inside.
    let mut t = mac_placeholders();
    t.parts[2].checked = Checked::NotLooked;
    let e = plan(&t, TargetSpec::Partition(3), false, true, &t.fingerprint()).unwrap_err();
    assert!(e.contains("was not looked inside"), "{e}");
}

#[test]
fn a_hybrid_mbr_a_full_efi_partition_or_no_disk_guid_refuses_the_disk() {
    let t = mac_placeholders();
    let refuse = |t: &DiskTable, says: &str| {
        let e = plan(t, TargetSpec::Partition(3), false, true, &t.fingerprint()).unwrap_err();
        assert!(e.contains(says), "{e}");
        assert!(classify::segments(t, true).iter().all(|s| !s.eligible));
    };
    let mut hybrid = t.clone();
    hybrid.probed.as_mut().unwrap().mbr_problem = Some("/dev/sda has a hybrid MBR".into());
    refuse(&hybrid, "hybrid MBR");
    let mut full = t.clone();
    full.probed.as_mut().unwrap().esp_free = Some(31_999_999);
    refuse(&full, "needs 32 MB");
    let mut unmeasured = t.clone();
    unmeasured.probed.as_mut().unwrap().esp_free = None;
    refuse(&unmeasured, "could not be looked at");
    let mut exactly = t.clone();
    exactly.probed.as_mut().unwrap().esp_free = Some(32_000_000);
    assert!(plan(&exactly, TargetSpec::Partition(3), false, true, &exactly.fingerprint()).is_ok());
    let mut no_guid = t.clone();
    no_guid.guid.clear();
    refuse(&no_guid, "GUID");
}

#[test]
fn a_placeholder_is_reformatted_where_it_is() {
    let t = mac_placeholders();
    let p = plan(&t, TargetSpec::Partition(3), false, true, &t.fingerprint()).unwrap();
    assert_eq!(
        words(&p),
        [
            "wipefs -a /dev/sda3",
            "parted -s /dev/sda type 3 0fc63daf-8483-4772-8e79-3d69d8477de4",
            "mkfs.ext4 -q -F -L YANTRIK /dev/sda3",
        ]
    );
    assert_eq!((p.esp, p.esp_path.as_str(), p.root), (1, "/dev/sda1", PartRef::Existing(3)));
    assert_eq!(p.sentence, "Yantrik OS will be installed into /dev/sda3 (200 GB). Nothing else on this disk changes.");
}

#[test]
fn encrypted_a_placeholder_is_remade_as_boot_and_root_inside_its_own_extent() {
    let t = mac_placeholders();
    let p = plan(&t, TargetSpec::Partition(3), true, true, &t.fingerprint()).unwrap();
    let c = words(&p);
    assert_eq!(c[0], "wipefs -a /dev/sda3");
    assert_eq!(c[1], "parted -s /dev/sda rm 3");
    assert_eq!(c[2], "parted -s -a none /dev/sda unit s mkpart yantrik-boot ext4 1543640536s 1545738239s");
    assert_eq!(c[3], "parted -s -a none /dev/sda unit s mkpart yantrik-root ext4 1545738240s 1934265535s");
    assert!(c[5].starts_with("cryptsetup luksFormat --type luks2 <the new partition at sector 1545738240>"));
    let sda3 = t.part(3).unwrap().run;
    for step in &p.steps {
        if let Step::MkPart { start, end, .. } = *step {
            assert!(start >= sda3.start && end <= sda3.end, "inside the placeholder: {start}-{end}");
        }
    }
}

#[test]
fn free_space_gets_one_partition_with_exact_bounds_inside_the_run() {
    let t = mac_free();
    let s = classify::segments(&t, true);
    let free = s.iter().find(|s| s.kind == "free").unwrap();
    assert_eq!(free.id, "sda@1543378392-1953525134");
    assert!(free.eligible, "{}", free.reason);
    assert_eq!(free.size, "210 GB");
    assert_eq!(preselect(&s).as_deref(), Some("sda@1543378392-1953525134"), "beside macOS, the free space");

    let (_, spec) = parse_target_id(&free.id).unwrap();
    let p = plan(&t, spec, false, true, &t.fingerprint()).unwrap();
    assert_eq!(
        words(&p),
        [
            "parted -s -a none /dev/sda unit s mkpart yantrik-root ext4 1543378944s 1953523711s",
            "mkfs.ext4 -q -F -L YANTRIK <the new partition at sector 1543378944>",
        ]
    );
    // On MiB boundaries, inside the run, short of the backup GPT, past the end of APFS.
    let (start, end) = (p.region.start, p.region.end);
    assert_eq!(start % 2048, 0);
    assert_eq!((end + 1) % 2048, 0);
    assert!(start >= 1_543_378_392 && end <= 1_953_525_134);
    assert!(start > t.part(2).unwrap().run.end);
    assert!(end <= t.last_usable());

    // Encrypted: /boot then the root, both in the run.
    let p = plan(&t, spec, true, true, &t.fingerprint()).unwrap();
    let c = words(&p);
    assert_eq!(c[0], "parted -s -a none /dev/sda unit s mkpart yantrik-boot ext4 1543378944s 1545476095s");
    assert_eq!(c[1], "parted -s -a none /dev/sda unit s mkpart yantrik-root ext4 1545476096s 1953523711s");

    // Part of the run, as the control surface may ask: that part, aligned, and no further.
    let part = TargetSpec::Free { start: 1_600_000_000, end: 1_700_000_000 };
    let p = plan(&t, part, false, true, &t.fingerprint()).unwrap();
    assert!(p.region.start >= 1_600_000_000 && p.region.end <= 1_700_000_000);
}

#[test]
fn free_space_that_is_not_free_is_refused() {
    let t = mac_free();
    let fp = t.fingerprint();
    // Reaching back into the APFS container.
    let e = plan(&t, TargetSpec::Free { start: 1_500_000_000, end: 1_900_000_000 }, false, true, &fp).unwrap_err();
    assert!(e.contains("not free space"), "{e}");
    // Past the end of the disk.
    let e = plan(&t, TargetSpec::Free { start: 1_543_378_392, end: 1_953_525_168 }, false, true, &fp).unwrap_err();
    assert!(e.contains("not free space"), "{e}");
    // The 1.7 GB left at the end of the placeholder layout.
    let t = mac_placeholders();
    let e = plan(&t, TargetSpec::Free { start: 1_950_152_680, end: 1_953_525_134 }, false, true, &t.fingerprint())
        .unwrap_err();
    assert!(e.contains("needs at least 20 GB"), "{e}");
}

#[test]
fn macos_windows_linux_and_the_efi_partition_are_never_targets() {
    let cases: Vec<(DiskTable, u32, &str)> = vec![
        (mac_full(), 2, "macOS (APFS)"),
        (mac_full(), 1, "EFI system partition"),
        (mac_placeholders(), 2, "macOS (APFS)"),
        (mac_placeholders(), 1, "EFI system partition"),
        (pc(), 1, "EFI system partition"),
        (pc(), 2, "Windows"),
        (pc(), 3, "Windows"),
        (pc(), 4, "Windows"),
        (pc(), 5, "Linux filesystem (ext4)"),
    ];
    for (t, n, says) in cases {
        for encrypt in [false, true] {
            let e = plan(&t, TargetSpec::Partition(n), encrypt, true, &t.fingerprint()).unwrap_err();
            assert!(e.contains(says), "{} {n}: {e}", t.path);
        }
    }
    // HFS+, as macOS up to 10.12 left it, by type and by filesystem.
    let mut t = mac_full();
    t.parts[1].type_uuid = "48465300-0000-11aa-aa11-00306543ecac".into();
    t.parts[1].fstype = "hfsplus".into();
    let e = plan(&t, TargetSpec::Partition(2), false, true, &t.fingerprint()).unwrap_err();
    assert!(e.contains("macOS (HFS+)"), "{e}");
    // Typed APFS with nothing blkid can read on it (an encrypted container): still macOS's.
    let mut t = mac_full();
    t.parts[1].fstype.clear();
    assert_eq!(classify::kind_of(&t.parts[1]), classify::Kind::Apfs);
}

#[test]
fn a_pc_offers_its_free_space_and_keeps_windows() {
    let t = pc();
    let s = classify::segments(&t, true);
    let eligible: Vec<&str> = s.iter().filter(|s| s.eligible).map(|s| s.id.as_str()).collect();
    assert_eq!(eligible, ["nvme0n1@804810752-1000215182"]);
    assert_eq!(s.iter().find(|s| s.id == "nvme0n1p3").unwrap().title, "Windows (Windows)");
    let (disk, spec) = parse_target_id(eligible[0]).unwrap();
    assert_eq!(disk, "nvme0n1");
    let p = plan(&t, spec, false, true, &t.fingerprint()).unwrap();
    assert_eq!(p.esp_path, "/dev/nvme0n1p1");
}

#[test]
fn the_plan_is_refused_when_the_table_changed() {
    // Shown the placeholder layout; the disk now has free space where it was.
    let shown = mac_placeholders().fingerprint();
    let now = mac_free();
    let e = plan(&now, TargetSpec::Free { start: 1_543_378_392, end: 1_953_525_134 }, false, true, &shown).unwrap_err();
    assert!(e.contains("has changed since it was shown") && e.contains("nothing was written"), "{e}");

    // The same partitions, but the placeholder was reformatted (or relabelled) in between.
    let t = mac_placeholders();
    let shown = t.fingerprint();
    let mut changed = t.clone();
    changed.parts[2].label = "PHOTOS".into();
    assert!(plan(&changed, TargetSpec::Partition(3), false, true, &shown).is_err());
    // A partition grew by one sector.
    let mut changed = t.clone();
    changed.parts[1].run.end += 1;
    assert!(plan(&changed, TargetSpec::Partition(3), false, true, &shown).is_err());
    // The table was rewritten with the same layout (a new disk GUID), a partition was recreated
    // in place (a new partition GUID), renamed, or had its flags changed.
    let edits: [&dyn Fn(&mut DiskTable); 4] = [
        &|t| t.guid = "00000000-1111-4222-8333-444444444444".into(),
        &|t| t.parts[1].uuid = "00000000-1111-4222-8333-555555555555".into(),
        &|t| t.parts[2].name = "Untitled".into(),
        &|t| t.parts[0].flags = vec!["boot".into()],
    ];
    for edit in edits {
        let mut changed = t.clone();
        edit(&mut changed);
        assert_ne!(changed.fingerprint(), shown);
        let e = plan(&changed, TargetSpec::Partition(3), false, true, &shown).unwrap_err();
        assert!(e.contains("has changed since it was shown"), "{e}");
    }
    // Unchanged: the same fingerprint, read twice.
    assert_eq!(mac_placeholders().fingerprint(), shown);
}

#[test]
fn bios_mode_and_tables_without_an_esp_are_refused() {
    let t = mac_placeholders();
    let e = plan(&t, TargetSpec::Partition(3), false, false, &t.fingerprint()).unwrap_err();
    assert!(e.contains("BIOS mode"), "{e}");
    let mut no_esp = mac_placeholders();
    no_esp.parts.remove(0);
    let e = plan(&no_esp, TargetSpec::Partition(3), false, true, &no_esp.fingerprint()).unwrap_err();
    assert!(e.contains("no EFI system partition"), "{e}");
    let mut mbr = mac_placeholders();
    mbr.label = "msdos".into();
    let e = plan(&mbr, TargetSpec::Partition(3), false, true, &mbr.fingerprint()).unwrap_err();
    assert!(e.contains("needs GPT"), "{e}");
}

/// Over every layout, every segment and both encryption choices: whatever plan comes out keeps
/// every guard. No `mklabel`; the EFI partition and every partition not chosen appear in no
/// command; new partitions stay in the free run; kept kinds give no plan at all.
#[test]
fn every_plan_from_every_layout_keeps_the_guards() {
    let mut plans = 0;
    for t in all() {
        let fp = t.fingerprint();
        for seg in classify::segments(&t, true) {
            for encrypt in [false, true] {
                let (_, spec) = parse_target_id(&seg.id).unwrap();
                let result = plan(&t, spec, encrypt, true, &fp);
                if seg.kept {
                    assert!(result.is_err(), "{} gave a plan", seg.id);
                    continue;
                }
                let Ok(p) = result else { continue };
                plans += 1;
                let cmds = words(&p);
                assert!(cmds.iter().all(|c| !c.contains("mklabel")), "{cmds:?}");
                for other in t.parts.iter().filter(|o| Some(o.number) != match spec {
                    TargetSpec::Partition(n) => Some(n),
                    _ => None,
                }) {
                    // `/dev/sda1 ` with the space: /dev/sda1 is not /dev/sda10.
                    assert!(
                        cmds.iter().all(|c| !format!("{c} ").contains(&format!("{} ", other.path))),
                        "{} appears in {cmds:?}",
                        other.path
                    );
                    if let TargetSpec::Partition(_) = spec {
                        assert!(cmds.iter().all(|c| !c.contains(&format!(" rm {} ", other.number)) && !c.ends_with(&format!(" rm {}", other.number))));
                    }
                }
                if let TargetSpec::Free { .. } = spec {
                    assert!(t.free.iter().any(|r| r.contains(&p.region)));
                    assert!(t.parts.iter().all(|o| !o.run.overlaps(&p.region)));
                }
                assert!(check(&p, &t).is_ok());
            }
        }
    }
    assert!(plans >= 6, "the layouts gave {plans} plans");
}

/// The guards in `check` catch a wrong plan on their own, whoever made it.
#[test]
fn a_plan_that_breaks_a_guard_is_refused_by_the_check() {
    let t = mac_placeholders();
    let good = plan(&t, TargetSpec::Partition(3), false, true, &t.fingerprint()).unwrap();
    let with = |f: &dyn Fn(&mut yantrik_install_target::Plan)| {
        let mut p = good.clone();
        f(&mut p);
        check(&p, &t).unwrap_err()
    };
    assert!(with(&|p| p.steps.push(Step::Format { part: PartRef::Existing(1), role: yantrik_install_target::Role::Root, encrypted: false }))
        .contains("format partition 1"));
    assert!(with(&|p| p.steps.insert(0, Step::Wipe(2))).contains("change partition 2"));
    assert!(with(&|p| p.steps.insert(0, Step::Remove(2))).contains("change partition 2"));
    assert!(with(&|p| p.steps.push(Step::MkPart { role: yantrik_install_target::Role::Root, start: 409_640, end: 500_000 }))
        .contains("outside"));
    assert!(with(&|p| p.spec = TargetSpec::Partition(2)).contains("install over /dev/sda2"));
    assert!(with(&|p| p.esp = 2).contains("not the EFI system partition"));

    let t = mac_free();
    let good = plan(&t, TargetSpec::Free { start: 1_543_378_392, end: 1_953_525_134 }, false, true, &t.fingerprint()).unwrap();
    let mut wide = good.clone();
    wide.region.start = 409_640;
    assert!(check(&wide, &t).unwrap_err().contains("outside free space"));
    let mut tail = good.clone();
    tail.region.end = 1_953_525_160;
    assert!(check(&tail, &t).is_err(), "into the backup GPT");

    // Free space claimed from sector 0, as no parted prints it: the GPT header is refused.
    let mut odd = t.clone();
    odd.free.push(yantrik_install_target::Run { start: 0, end: 39 });
    let mut head = good.clone();
    head.region = yantrik_install_target::Run { start: 0, end: 39 };
    head.steps = vec![Step::MkPart { role: yantrik_install_target::Role::Root, start: 0, end: 39 }];
    assert!(check(&head, &odd).unwrap_err().contains("space GPT keeps"));
    head.region.start = 34;
    head.steps = vec![
        Step::MkPart { role: yantrik_install_target::Role::Root, start: 34, end: 39 },
        Step::Format { part: PartRef::StartingAt(34), role: yantrik_install_target::Role::Root, encrypted: false },
    ];
    assert!(check(&head, &odd).is_ok(), "34 is the first sector GPT leaves at 512 bytes");
}

#[test]
fn targets_are_named_as_the_kernel_names_them() {
    assert_eq!(parse_target_id("sda3").unwrap(), ("sda".into(), TargetSpec::Partition(3)));
    assert_eq!(parse_target_id("/dev/nvme0n1p4").unwrap(), ("nvme0n1".into(), TargetSpec::Partition(4)));
    assert_eq!(parse_target_id("loop0p2").unwrap(), ("loop0".into(), TargetSpec::Partition(2)));
    assert_eq!(
        parse_target_id("sda@2048-4095").unwrap(),
        ("sda".into(), TargetSpec::Free { start: 2048, end: 4095 })
    );
    for whole in ["sda", "nvme0n1", "loop0", "", "sda@9-1", "sda@x-y"] {
        assert!(parse_target_id(whole).is_err(), "{whole}");
    }
}
