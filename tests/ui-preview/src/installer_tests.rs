use super::*;
use crate::installer_rules as rules;
use slint::{ModelRc, SharedString, VecModel};

/// The installer's four screens (#400), drawn from the production InstallerScreen with the
/// production rules behind every field, and driven with real key and pointer events:
///
/// - Welcome offers "Try it first" and "Install"; the layout picker opens and a row picks a
///   layout, which the shell is told about.
/// - You takes typing straight away (the name field has focus), derives the username and the
///   computer name from the name as it is typed, says a password mismatch under the field as
///   it happens, and keeps Next disabled until every field is acceptable.
/// - Disk lists the disks with what each holds, a click chooses one, and it says the disk
///   will be erased.
/// - Review sums it up, refuses a timezone the zone database does not have, and Install hands
///   the shell exactly what was chosen.
/// - Installing shows real stages and the elapsed time; at 100 it becomes Installed, whose
///   Restart now reaches the shell.
///
/// Buttons are found the way a person finds them, by pointing: the forward button of every
/// step sits at the same place right of centre, so the probe walks up that column from the
/// bottom and presses the first thing it meets.
pub fn run(window: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    for (w, h) in [(1280u32, 800u32), (800, 600)] {
        run_at(window, output, w, h)?;
        beside_macos(window, output, w, h)?;
    }
    println!("PASS: installer welcome, you, disk, review, installing and installed at 1280x800 and 800x600");
    println!("PASS: installing beside macOS: the disk's bar, the placeholder chosen, nothing chosen until something may be");
    Ok(())
}

/// One stretch of the Mac's disk as crates/yantrik-install-target describes it.
fn seg(id: &str, kind: &str, title: &str, size: &str, share: f32, kept: bool, eligible: bool) -> InstallerSegment {
    let device = if kind == "free" { "a new partition in the free space on /dev/sda".to_string() } else { format!("/dev/{id}") };
    InstallerSegment {
        id: id.into(),
        disk: "sda".into(),
        kind: kind.into(),
        title: title.into(),
        size: size.into(),
        share,
        kept,
        eligible,
        reason: if eligible { "".into() } else { "kept".into() },
        sentence: format!("Yantrik OS will be installed into {device} ({size}). Nothing else on this disk changes.").into(),
        card: format!("Install into {device} ({size}, {title}); nothing else changes").into(),
    }
}

/// The Mac mini this is for: a 1 TB disk with its 209.7 MB EFI partition, APFS shrunk to 790 GB
/// in macOS, a 200 GB FAT32 YANTRIK placeholder, and the 8 GB YANTRIK-INS partition the installer
/// booted from (so the whole-disk list is empty: #644 never offers the disk the installer runs from).
fn beside_macos(
    window: &MinimalSoftwareWindow,
    output: &str,
    width: u32,
    height: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let ui = InstallerProbe::new()?;
    ui.set_canvas_width(width as f32);
    ui.set_canvas_height(height as f32);
    ui.on_is_macos_disk(|d, l| rules::disk_in_list(&d, &l));
    ui.on_target_listed(|t, l| rules::disk_in_list(&t, &l));
    ui.on_check_timezone(|_| SharedString::new());
    ui.set_target_disks(ModelRc::new(VecModel::from(vec![InstallerTargetDisk {
        name: "sda".into(),
        title: "APPLE HDD HTS541010A9E662 · 931.5G (/dev/sda)".into(),
    }])));
    let placeholders = vec![
        seg("sda1", "esp", "EFI system", "209.7 MB", 0.0002, true, false),
        seg("sda2", "macos", "macOS (APFS)", "790 GB", 0.79, true, false),
        seg("sda3", "placeholder", "YANTRIK (FAT32)", "200 GB", 0.2, false, true),
        seg("sda4", "medium", "YANTRIK-INS (this installer)", "8 GB", 0.008, true, false),
        seg("sda@1950152680-1953525134", "free", "Free space", "1.7 GB", 0.0017, false, false),
    ];
    ui.set_segments(ModelRc::new(VecModel::from(placeholders)));
    ui.set_eligible_targets("sda3".into());
    ui.set_into_partition(true);
    ui.set_install_target("sda3".into());
    ui.set_timezone("Europe/Berlin".into());
    ui.set_step(2);

    ui.show()?;
    window.set_size(slint::PhysicalSize::new(width, height));
    let draw = || {
        slint::platform::update_timers_and_animations();
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        window.request_redraw();
        window.draw_if_needed(|r| {
            r.render(p.make_mut_slice(), width as usize);
        });
        p
    };
    let save = |name: &str| -> Result<(), Box<dyn std::error::Error>> {
        for _ in 0..4 {
            draw();
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        let pixels = draw();
        let path = output.replace(".png", &format!("-{width}x{height}-{name}.png"));
        let mut e = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()?.write_image_data(pixels.as_bytes())?;
        Ok(())
    };

    save("disk-beside-macos")?;
    assert!(ui.get_disk_ok(), "the YANTRIK placeholder may be installed into");
    // Nothing chosen: Next waits. The macOS partition is never a target, whatever is written.
    ui.set_install_target("".into());
    draw();
    assert!(!ui.get_disk_ok());
    ui.set_install_target("sda2".into());
    draw();
    assert!(!ui.get_disk_ok(), "macOS is not a target even when named");
    ui.set_install_target("sda4".into());
    draw();
    assert!(!ui.get_disk_ok(), "nor the partition the installer runs from");
    // A row is chosen by pointing at it, left of the Back button, walking up from the bottom.
    let x = width as f32 / 2.0 - 200.0;
    let mut y = height as f32 - 6.0;
    while y > 0.0 && ui.get_install_target() != "sda3" {
        click(window, x, y);
        draw();
        y -= 6.0;
    }
    assert_eq!(ui.get_install_target(), "sda3", "the placeholder's row can be chosen");
    assert!(ui.get_disk_ok());

    // Erasing a whole disk is the other choice; with no disk it offers, nothing is ready.
    ui.set_into_partition(false);
    save("disk-beside-macos-erase")?;
    assert!(!ui.get_disk_ok(), "the disk the installer runs from is not offered for erasing");
    ui.set_into_partition(true);

    // Review says where it goes, and Install no longer says it erases.
    ui.set_full_name("Ada Lovelace".into());
    ui.set_step(3);
    save("review-beside-macos")?;

    // Free space beside APFS instead of a placeholder.
    ui.set_step(2);
    ui.set_segments(ModelRc::new(VecModel::from(vec![
        seg("sda1", "esp", "EFI system", "209.7 MB", 0.0002, true, false),
        seg("sda2", "macos", "macOS (APFS)", "790 GB", 0.79, true, false),
        seg("sda@1543378392-1953525134", "free", "Free space", "210 GB", 0.21, false, true),
    ])));
    ui.set_eligible_targets("sda@1543378392-1953525134".into());
    ui.set_install_target("sda@1543378392-1953525134".into());
    save("disk-free-beside-macos")?;
    assert!(ui.get_disk_ok());

    // The Mac as it arrived: nothing to install into until macOS makes room.
    ui.set_segments(ModelRc::new(VecModel::from(vec![
        seg("sda1", "esp", "EFI system", "209.7 MB", 0.0002, true, false),
        seg("sda2", "macos", "macOS (APFS)", "1000 GB", 0.9998, true, false),
    ])));
    ui.set_eligible_targets("".into());
    ui.set_install_target("".into());
    save("disk-macos-no-room")?;
    assert!(!ui.get_disk_ok());

    ui.hide()?;
    Ok(())
}

fn run_at(
    window: &MinimalSoftwareWindow,
    output: &str,
    width: u32,
    height: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let zoneinfo = std::env::temp_dir().join(format!("yos-preview-zoneinfo-{}", std::process::id()));
    std::fs::create_dir_all(zoneinfo.join("Europe"))?;
    std::fs::write(zoneinfo.join("Europe/Berlin"), b"TZif")?;
    std::fs::write(zoneinfo.join("UTC"), b"TZif")?;

    let ui = InstallerProbe::new()?;
    ui.set_canvas_width(width as f32);
    ui.set_canvas_height(height as f32);
    let problem = |p: Option<String>| -> SharedString { p.unwrap_or_default().into() };
    ui.on_derive_username(|n| rules::derive_username(&n).into());
    ui.on_hostname_for(|u| rules::hostname_for(&u).into());
    ui.on_check_full_name(move |n| problem(rules::full_name_problem(&n)));
    ui.on_check_username(move |u| problem(rules::username_problem(&u)));
    ui.on_check_hostname(move |h| problem(rules::hostname_problem(&h)));
    ui.on_check_password(move |p, c| problem(rules::password_problem(&p, &c)));
    let zi = zoneinfo.clone();
    ui.on_check_timezone(move |t| problem(rules::timezone_problem(&t, &zi)));
    ui.on_is_macos_disk(|d, l| rules::disk_in_list(&d, &l));

    let layouts: Vec<KeyboardChoice> = rules::layout_choices("us")
        .into_iter()
        .map(|(code, label)| KeyboardChoice { code: code.into(), label: label.into() })
        .collect();
    ui.set_keyboards(ModelRc::new(VecModel::from(layouts)));
    ui.set_disks(ModelRc::new(VecModel::from(vec![
        InstallerDisk {
            name: "sda".into(),
            size: "80G".into(),
            model: "VBOX HARDDISK".into(),
            contents: "3 partitions (vfat, ntfs)".into(),
            has_data: true,
            holds_macos: false,
        },
        InstallerDisk {
            name: "nvme0n1".into(),
            size: "476.9G".into(),
            model: "Samsung SSD 980".into(),
            contents: "Empty".into(),
            has_data: false,
            holds_macos: false,
        },
    ])));
    ui.set_selected_disk("sda".into());
    ui.set_timezone("Europe/Berlin".into());

    ui.show()?;
    window.set_size(slint::PhysicalSize::new(width, height));
    let draw = || {
        slint::platform::update_timers_and_animations();
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        window.request_redraw();
        window.draw_if_needed(|r| {
            r.render(p.make_mut_slice(), width as usize);
        });
        p
    };
    let settle = || {
        for _ in 0..4 {
            draw();
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        draw()
    };
    let save = |name: &str| -> Result<(), Box<dyn std::error::Error>> {
        let pixels = settle();
        let path = output.replace(".png", &format!("-{width}x{height}-{name}.png"));
        let mut e = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()?.write_image_data(pixels.as_bytes())?;
        Ok(())
    };
    // The forward button's column: two 160px pills (or 160 + 220) 12px apart, centred.
    let forward_x = width as f32 / 2.0 + 86.0;
    let back_x = width as f32 / 2.0 - 86.0;
    // Walk up a column from the bottom, pressing, until `done` says something happened.
    let press_up = |x: f32, done: &dyn Fn() -> bool| -> bool {
        let mut y = height as f32 - 6.0;
        while y > 0.0 {
            click(window, x, y);
            draw();
            if done() {
                return true;
            }
            y -= 6.0;
        }
        false
    };

    // ── Welcome ──
    save("welcome")?;
    assert!(press_up(back_x, &|| ui.get_tried()), "Try it first is pressable at {width}x{height}");
    ui.set_keyboard_open(true);
    save("welcome-keyboards")?;
    // The list sits under the picker; pointing at it from below reaches a row before the picker.
    assert!(
        press_up(width as f32 / 2.0 - 60.0, &|| !ui.get_keyboard_open()),
        "a layout row closes the list"
    );
    let picked = ui.get_keyboard();
    assert_ne!(picked, "us", "a row other than the current layout was picked");
    assert_eq!(ui.get_chosen_layout(), picked, "the shell was told which layout to apply");
    assert!(press_up(forward_x, &|| ui.get_step() == 1), "Install moves on to You");

    // ── You ──
    settle();
    // The name field has focus on arrival: typing goes straight in.
    key(window, "Ada Lovelace".into());
    settle();
    assert_eq!(ui.get_full_name(), "Ada Lovelace", "the name field takes typing on arrival");
    assert_eq!(ui.get_username(), "ada", "the username follows the first name");
    assert_eq!(ui.get_hostname(), "ada-yantrik", "the computer name follows the username");
    assert!(!ui.get_you_ok(), "no password yet");
    ui.set_password("correct horse".into());
    ui.set_password_confirm("correct hose".into());
    save("you-mismatch")?;
    assert!(!ui.get_you_ok(), "a mismatch keeps Next disabled");
    assert!(!press_up(forward_x, &|| ui.get_step() != 1), "a disabled Next does not move on");
    ui.set_password_confirm("correct horse".into());
    settle();
    assert!(ui.get_you_ok());
    // A username of their own stops the derivation; the computer name still follows it.
    ui.set_username_touched(true);
    ui.set_username("Ada L".into());
    settle();
    assert!(!ui.get_you_ok(), "a username useradd would refuse blocks Next");
    save("you-bad-username")?;
    ui.set_username("lovelace".into());
    ui.set_full_name("Augusta Ada King".into());
    settle();
    assert_eq!(ui.get_username(), "lovelace", "a written username is not overwritten");
    assert_eq!(ui.get_hostname(), "lovelace-yantrik");
    assert!(ui.get_you_ok());
    save("you")?;
    assert!(press_up(forward_x, &|| ui.get_step() == 2), "Next moves on to Disk");

    // ── Disk ──
    save("disk")?;
    // Rows are chosen by pointing. Left of the Back button, walking up meets the second
    // disk's row first.
    assert!(
        press_up(width as f32 / 2.0 - 200.0, &|| ui.get_selected_disk() == "nvme0n1"),
        "the second disk can be chosen"
    );
    // A disk holding macOS: chosen only by the person, and erased only once they tick the box
    // that names it (installer_rules::disk_problem).
    ui.set_selected_disk("sda".into());
    ui.set_macos_disks("sda".into());
    save("disk-macos")?;
    ui.set_erase_macos_disk("sda".into());
    save("disk-macos-confirmed")?;
    ui.set_macos_disks("".into());
    ui.set_erase_macos_disk("".into());
    ui.set_selected_disk("sda".into());
    assert!(press_up(forward_x, &|| ui.get_step() == 3), "Next moves on to Review");

    // ── Review ──
    save("review")?;
    assert!(ui.get_can_install());
    ui.set_macos_disks("sda".into());
    settle();
    assert!(!ui.get_can_install(), "macOS on the chosen disk blocks Install until confirmed");
    ui.set_erase_macos_disk("nvme0n1".into());
    settle();
    assert!(!ui.get_can_install(), "a confirmation for another disk is no confirmation");
    ui.set_erase_macos_disk("sda".into());
    settle();
    assert!(ui.get_can_install(), "confirmed, the macOS disk may be erased");
    ui.set_macos_disks("".into());
    ui.set_erase_macos_disk("".into());
    settle();
    ui.set_timezone("Mars/Olympus".into());
    settle();
    assert!(!ui.get_can_install(), "a zone the system does not know blocks Install");
    save("review-bad-timezone")?;
    ui.set_timezone("Europe/Berlin".into());
    settle();
    assert!(press_up(forward_x, &|| ui.get_step() == 4), "Install starts the install");
    assert!(ui.get_installing());
    assert_eq!(
        ui.get_installed_with(),
        format!("lovelace|Augusta Ada King|lovelace-yantrik|{picked}|Europe/Berlin|sda"),
        "the shell is handed what was chosen"
    );

    // ── Installing → Installed ──
    ui.set_install_progress(42);
    ui.set_install_status("Copying system files...".into());
    save("installing")?;
    ui.set_install_progress(100);
    ui.set_installing(false);
    for _ in 0..14 {
        draw();
        std::thread::sleep(std::time::Duration::from_millis(100));
        if ui.get_step() == 5 {
            break;
        }
    }
    assert_eq!(ui.get_step(), 5, "100% becomes Installed");
    save("installed")?;
    assert!(press_up(forward_x, &|| ui.get_reboots() > 0), "Restart now reaches the shell");

    let _ = std::fs::remove_dir_all(&zoneinfo);
    ui.hide()?;
    Ok(())
}
