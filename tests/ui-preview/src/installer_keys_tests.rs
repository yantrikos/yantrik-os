use super::*;
use crate::installer_rules as rules;
use slint::{ModelRc, SharedString, VecModel};

/// The installer's primary buttons with the keyboard alone (#1165: the visitor's VM had no
/// working pointer). Welcome's Install and "Start the installed Yantrik" are Tab stops that Enter
/// presses, and on the Disk screen Tab only moves between "Beside what is there" and "Erase a
/// whole disk": Space or Enter chooses one, Tab never does.
pub fn run(window: &MinimalSoftwareWindow) -> Result<(), Box<dyn std::error::Error>> {
    use slint::platform::Key;
    // A fresh screen for each part, so no focus is left over from the part before.
    let probe = || -> Result<InstallerProbe, Box<dyn std::error::Error>> {
        let ui = InstallerProbe::new()?;
        ui.set_canvas_width(1280.0);
        ui.set_canvas_height(800.0);
        ui.on_derive_username(|n| rules::derive_username(&n).into());
        ui.on_hostname_for(|u| rules::hostname_for(&u).into());
        ui.on_check_full_name(|_| SharedString::new());
        ui.on_check_username(|_| SharedString::new());
        ui.on_check_hostname(|_| SharedString::new());
        ui.on_check_password(|_, _| SharedString::new());
        ui.on_check_timezone(|_| SharedString::new());
        ui.on_is_macos_disk(|d, l| rules::disk_in_list(&d, &l));
        ui.on_target_listed(|t, l| rules::disk_in_list(&t, &l));
        ui.on_layout_after(|_, current, _| current);
        let layouts: Vec<KeyboardChoice> = rules::layout_choices("us")
            .into_iter()
            .map(|(code, label)| KeyboardChoice { code: code.into(), label: label.into() })
            .collect();
        ui.set_keyboards(ModelRc::new(VecModel::from(layouts)));
        ui.set_existing_install("Yantrik is already installed on this computer (/dev/sda2).".into());
        ui.set_existing_startable(true);
        ui.set_target_disks(ModelRc::new(VecModel::from(vec![InstallerTargetDisk { name: "sda".into(), title: "VBOX HARDDISK".into() }])));
        ui.show()?;
        window.set_size(slint::PhysicalSize::new(1280, 800));
        Ok(ui)
    };
    let ui = probe()?;
    let press = |k: SharedString| {
        key(window, k);
        slint::platform::update_timers_and_animations();
    };
    let tab = || press(Key::Tab.into());

    // Welcome: the picker, the try-it field, then the start button, then Install.
    tab();
    tab();
    tab();
    press("\n".into());
    assert_eq!(ui.get_started_installed(), 1, "Enter on the focused \"Start the installed Yantrik\" starts it");
    tab();
    press("\n".into());
    assert_eq!(ui.get_step(), 1, "Tab reaches Install and Enter presses it");

    // Disk: Tab lands on "Beside what is there" without choosing it; Space does.
    ui.hide()?;
    let ui = probe()?;
    ui.set_step(2);
    ui.set_into_partition(false);
    slint::platform::update_timers_and_animations();
    tab();
    assert!(!ui.get_into_partition(), "Tab alone does not choose \"Beside what is there\"");
    press(" ".into());
    assert!(ui.get_into_partition(), "Space chooses the focused \"Beside what is there\"");
    ui.set_into_partition(false);
    tab();
    assert!(!ui.get_into_partition(), "nor does Tab onto \"Erase a whole disk\" change it");
    ui.hide()?;
    Ok(())
}
