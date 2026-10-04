//! The installer describes itself, and can be filled in and run without a mouse.
//!
//! The first-run wizard was the one screen in the OS that only a human could operate. Everything
//! else publishes `app.describe` / `app.act`; onboarding published the single word "onboarding"
//! and nothing more. So an agent handed a freshly booted machine had exactly one option, and it
//! is the option we built this control surface to kill: photograph the screen, guess at pixel
//! coordinates, aim synthetic clicks, photograph it again to find out what happened.
//!
//! That is not a hypothetical. Driving this installer through the QEMU monitor took three hours
//! and never typed a character, because the harness was moving a pointer QEMU had not selected
//! and there was no way to tell from the outside — no caret, no focus, no state to read. A
//! surface that says what the form holds would have answered it in one call.
//!
//! What is deliberately *not* here: filling the form does not install anything. `install` is a
//! separate, `dangerous` action, because it repartitions a disk. Reading is free; erasing is not.
//!
//! The installer is four screens since #400 (installer.slint): welcome, you, disk, review. The
//! first-boot onboarding still publishes its own steps here, read-only, under their old names.

use std::path::Path;

use slint::{ComponentHandle, Model};
use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

use crate::installer_rules;
use crate::App;

/// The installer's steps, as `onboard-phase` holds them in installer mode. installer.slint
/// draws them by the same numbers.
pub mod step {
    pub const WELCOME: i32 = 0;
    pub const YOU: i32 = 1;
    pub const DISK: i32 = 2;
    pub const REVIEW: i32 = 3;
    pub const INSTALLING: i32 = 4;
    pub const INSTALLED: i32 = 5;
}

/// The installer's screens, named so a caller can navigate by intent rather than by number.
///
/// Installing and installed are consequences of pressing Install, not places to jump to; they
/// are reported by `describe` but refused by `go_to`.
const STEPS: &[(&str, i32)] = &[
    ("welcome", step::WELCOME),
    ("you", step::YOU),
    ("disk", step::DISK),
    ("review", step::REVIEW),
];

/// Earlier names for the installer's screens, from the twelve-phase wizard it replaced, so a
/// caller written against that one still lands somewhere sensible.
const STEP_ALIASES: &[(&str, &str)] = &[
    ("keyboard", "welcome"),
    ("identity", "you"),
    ("account", "you"),
    ("summary", "review"),
    ("install", "review"),
];

/// The first-boot onboarding's phases (onboarding.slint), for describing it. It has no actions
/// on the surface; these are only so `step` says something better than a number.
const ONBOARDING_STEPS: &[(&str, i32)] = &[
    ("identity", 3),
    ("interests", 4),
    ("location", 5),
    ("hardware", 6),
    ("ai-mode", 7),
    ("ai-provider", 8),
    ("ai-test", 9),
    ("ready", 10),
];

/// What the wizard is showing, in words.
pub fn step_name(installer_mode: bool, phase: i32) -> &'static str {
    if installer_mode {
        return match phase {
            step::INSTALLING => "installing",
            step::INSTALLED => "installed",
            other => STEPS
                .iter()
                .find(|(_, id)| *id == other)
                .map(|(name, _)| *name)
                .unwrap_or("unknown"),
        };
    }
    match phase {
        0 | 1 => "waking",
        2 => "greeting",
        other => ONBOARDING_STEPS
            .iter()
            .find(|(_, id)| *id == other)
            .map(|(name, _)| *name)
            .unwrap_or("unknown"),
    }
}

/// The installer step a caller asked for by name, aliases included.
fn step_for(name: &str) -> Option<i32> {
    let name = STEP_ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
        .map(|(_, real)| *real)
        .unwrap_or(name);
    STEPS.iter().find(|(n, _)| *n == name).map(|(_, id)| *id)
}

/// The fields a caller may write, and the property each one lands in.
///
/// Named for what the screen calls them, not for the Slint property: the field under "More"
/// reads "Computer name", so `hostname` and `computer_name` both work rather than only the one
/// that happens to match our internal spelling.
const FIELDS: &[&str] = &[
    "full_name",
    "username",
    "password",
    "password_confirm",
    "hostname",
    "keyboard",
    "timezone",
    "disk",
    "encrypt",
];

/// A caller's spelling of a field, reduced to the one in `FIELDS`.
fn canonical_field(field: &str) -> String {
    let field = field.trim().to_lowercase();
    match field.as_str() {
        "computer_name" | "computer" => "hostname",
        "name" => "full_name",
        "target_disk" | "target" => "disk",
        "layout" | "keyboard_layout" | "keymap" => "keyboard",
        "tz" | "time_zone" | "zone" => "timezone",
        "encryption" | "encrypted" | "luks" => "encrypt",
        other => other,
    }
    .to_string()
}

/// The disks the installer found.
fn disks(ui: &App) -> Vec<serde_json::Value> {
    ui.get_onboard_disks()
        .iter()
        .map(|d| {
            serde_json::json!({
                "name": d.name.to_string(),
                "size": d.size.to_string(),
                "model": d.model.to_string(),
                "contents": d.contents.to_string(),
                "has_data": d.has_data,
            })
        })
        .collect()
}

/// The keyboard layouts the Welcome screen offers.
fn keyboards(ui: &App) -> Vec<String> {
    ui.get_onboard_keyboards().iter().map(|k| k.code.to_string()).collect()
}

/// Why the machine cannot be installed yet, or `None` when it can.
///
/// The same rules the screens hold each field to (installer_rules.rs), in the order the screens
/// ask for them, so a caller reading `blocked_by` learns exactly what a person would learn from
/// the message under a field or the Next button that will not press.
pub fn blocked_by(ui: &App) -> Option<String> {
    if !ui.get_onboard_installer_mode() {
        return Some("this is not the installer — the machine is already installed".into());
    }
    let form = Form {
        full_name: ui.get_onboard_input_full_name().to_string(),
        username: ui.get_onboard_input_username().to_string(),
        password: ui.get_onboard_input_password().to_string(),
        password_confirm: ui.get_onboard_input_password_confirm().to_string(),
        hostname: ui.get_onboard_input_hostname().to_string(),
        timezone: ui.get_onboard_timezone().to_string(),
        selected_disk: ui.get_onboard_selected_disk().to_string(),
        disks: disks(ui)
            .iter()
            .filter_map(|d| d["name"].as_str().map(String::from))
            .collect(),
    };
    form.problem(Path::new("/usr/share/zoneinfo"))
}

/// What the installer holds, taken off the screen so the checks can be tested without one.
struct Form {
    full_name: String,
    username: String,
    password: String,
    password_confirm: String,
    hostname: String,
    timezone: String,
    selected_disk: String,
    disks: Vec<String>,
}

impl Form {
    fn problem(&self, zoneinfo: &Path) -> Option<String> {
        let lower = |s: String| {
            let mut c = s.chars();
            match c.next() {
                Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
                None => s,
            }
        };
        if let Some(why) = installer_rules::full_name_problem(&self.full_name) {
            return Some(format!("full_name: {}", lower(why)));
        }
        if let Some(why) = installer_rules::username_problem(&self.username) {
            return Some(format!("username: {}", lower(why)));
        }
        if let Some(why) = installer_rules::password_problem(&self.password, &self.password_confirm) {
            return Some(format!("password: {}", lower(why)));
        }
        if let Some(why) = installer_rules::hostname_problem(&self.hostname) {
            return Some(format!("hostname: {}", lower(why)));
        }
        if self.selected_disk.trim().is_empty() {
            return Some(if self.disks.is_empty() {
                "no disk to install to was found".into()
            } else {
                format!("no disk chosen; this machine has: {}", self.disks.join(", "))
            });
        }
        if let Some(why) = installer_rules::timezone_problem(&self.timezone, zoneinfo) {
            return Some(format!("timezone: {}", lower(why)));
        }
        None
    }
}

/// What the installer is showing right now.
///
/// Passwords travel as `true`/`false`, never as text. A control surface is read by whatever is
/// driving the machine and, on this path, written to a transcript; a password that reaches a
/// transcript is a password that has to be changed.
pub fn state(ui: &App) -> serde_json::Value {
    let phase = ui.get_onboard_phase();
    let installer = ui.get_onboard_installer_mode();
    let pw = ui.get_onboard_input_password();
    let pw2 = ui.get_onboard_input_password_confirm();

    serde_json::json!({
        "installer_mode": installer,
        "after_install": ui.get_onboard_after_install(),
        "step": step_name(installer, phase),
        "phase": phase,
        "fields": {
            "full_name": ui.get_onboard_input_full_name().to_string(),
            "username": ui.get_onboard_input_username().to_string(),
            "hostname": ui.get_onboard_input_hostname().to_string(),
            "keyboard": ui.get_onboard_keyboard().to_string(),
            "timezone": ui.get_onboard_timezone().to_string(),
            "encrypt": ui.get_onboard_encrypt(),
            "companion_name": ui.get_onboard_input_companion().to_string(),
            "location": ui.get_onboard_input_location().to_string(),
            "password_set": !pw.is_empty(),
            "password_confirm_set": !pw2.is_empty(),
            "passwords_match": pw == pw2,
        },
        "keyboards": keyboards(ui),
        "disks": disks(ui),
        "disks_scanned": ui.get_onboard_disks_scanned(),
        "selected_disk": ui.get_onboard_selected_disk().to_string(),
        // Measured in the background whether or not a screen shows it: the first-boot AI
        // setup recommends from it, and a caller deciding where to install can read it here.
        "hardware": {
            "cpu": { "ok": ui.get_onboard_ai_hw_cpu_ok(), "detail": ui.get_onboard_ai_hw_cpu_label().to_string() },
            "ram": { "ok": ui.get_onboard_ai_hw_ram_ok(), "detail": ui.get_onboard_ai_hw_ram_label().to_string() },
            "gpu": { "ok": ui.get_onboard_ai_hw_gpu_ok(), "detail": ui.get_onboard_ai_hw_gpu_label().to_string() },
            "disk": { "ok": ui.get_onboard_ai_hw_disk_ok(), "detail": ui.get_onboard_ai_hw_disk_label().to_string() },
            "network_ok": ui.get_onboard_ai_hw_network_ok(),
            "runtime_ok": ui.get_onboard_ai_hw_runtime_ok(),
            "recommends": ui.get_onboard_ai_hw_recommend().to_string(),
        },
        "ai_test": {
            "status": ui.get_onboard_ai_test_status().to_string(),
            "model": ui.get_onboard_ai_test_model().to_string(),
            "latency_ms": ui.get_onboard_ai_test_latency_ms(),
            "privacy": ui.get_onboard_ai_test_privacy().to_string(),
        },
        "installing": ui.get_onboard_installing(),
        "progress": ui.get_onboard_install_progress(),
        "status": ui.get_onboard_install_status().to_string(),
        "error": ui.get_onboard_install_error().to_string(),
        // Installed, with something the person must do (wire/installer_ownership.rs).
        "note": ui.get_onboard_install_note().to_string(),
        "blocked_by": blocked_by(ui).map(serde_json::Value::String).unwrap_or(serde_json::Value::Null),
        "steps": if installer {
            STEPS.iter().map(|(n, _)| *n).collect::<Vec<_>>()
        } else {
            ONBOARDING_STEPS.iter().map(|(n, _)| *n).collect::<Vec<_>>()
        },
        "fields_writable": if installer { FIELDS.to_vec() } else { Vec::new() },
    })
}

/// The line worth reading first when the machine is sitting in the wizard.
pub fn summary(ui: &App) -> String {
    let installer = ui.get_onboard_installer_mode();
    let step = step_name(installer, ui.get_onboard_phase());
    if !installer {
        return format!("Yantrik — first-run setup, on the {step} step");
    }
    // Finished is checked first. The flag and the phase are set by different threads a beat
    // apart, and reading them the other way round reports "installing, 100%" to anyone who
    // asks in between — a state that sounds like work still to do.
    if ui.get_onboard_phase() == step::INSTALLED {
        let note = ui.get_onboard_install_note();
        if !note.is_empty() {
            return format!("Yantrik installer — installed, waiting for a restart: {note}");
        }
        return "Yantrik installer — installed, restarting".into();
    }
    if ui.get_onboard_installing() {
        return format!(
            "Yantrik installer — installing, {}% — {}",
            ui.get_onboard_install_progress(),
            ui.get_onboard_install_status()
        );
    }
    match blocked_by(ui) {
        Some(why) => format!("Yantrik installer — on the {step} step, not ready to install: {why}"),
        None => format!(
            "Yantrik installer — on the {step} step, ready to install to {}{}",
            ui.get_onboard_selected_disk(),
            if ui.get_onboard_encrypt() { ", encrypted" } else { ", NOT encrypted" }
        ),
    }
}

/// A yes or no as a caller might write it; anything else is not an answer.
fn parse_switch(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// Add the installer's actions to the shell's control surface, on machines that have an
/// installer.
///
/// An installed desktop has no disk to install to and no wizard to fill in, and advertising
/// `installer_install` there — marked `dangerous`, offering to erase a disk — is worse than
/// useless: every caller that lists the shell's actions has to read it and work out that it
/// would refuse. The live image sets `installer-mode`; nothing else does.
pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    if !ui.get_onboard_installer_mode() {
        return surface;
    }
    let ui_for = {
        let weak = ui.as_weak();
        move || weak.upgrade().ok_or_else(|| "the shell is gone".to_string())
    };
    let set_ui = ui_for.clone();
    let goto_ui = ui_for.clone();
    let install_ui = ui_for.clone();
    let reboot_ui = ui_for;

    surface
        .action(
            Action::new(
                "installer_set",
                "Fill in one field of the installer, as if typed into it",
            )
            .arg(Param::text("field").describe(
                "full_name, username, password, password_confirm, hostname, keyboard, timezone, disk, \
                 encrypt (true or false; on unless turned off)",
            ))
            .arg(Param::text("value")),
            move |args| {
                let ui = set_ui()?;
                let field = canonical_field(args["field"].as_str().unwrap_or_default());
                let value = args["value"].as_str().unwrap_or_default().to_string();
                if ui.get_onboard_installing() {
                    return Err("an install is running; the form cannot be changed".into());
                }

                match field.as_str() {
                    // The username follows the name, and the computer's name the username, as
                    // they do for a person typing, until either is written outright. The screen
                    // derives them in its change handlers; they are derived here as well
                    // because the screen is not there to do it after "Try it first".
                    "full_name" => {
                        ui.set_onboard_input_full_name(value.clone().into());
                        if !ui.get_onboard_username_touched() {
                            let username = installer_rules::derive_username(&value);
                            if !ui.get_onboard_hostname_touched() {
                                ui.set_onboard_input_hostname(
                                    installer_rules::hostname_for(&username).into(),
                                );
                            }
                            ui.set_onboard_input_username(username.into());
                        }
                    }
                    "username" => {
                        ui.set_onboard_username_touched(true);
                        ui.set_onboard_input_username(value.clone().into());
                        if !ui.get_onboard_hostname_touched() {
                            ui.set_onboard_input_hostname(
                                installer_rules::hostname_for(&value).into(),
                            );
                        }
                    }
                    "password" => ui.set_onboard_input_password(value.clone().into()),
                    "password_confirm" => {
                        ui.set_onboard_input_password_confirm(value.clone().into())
                    }
                    "hostname" => {
                        ui.set_onboard_hostname_touched(true);
                        ui.set_onboard_input_hostname(value.clone().into());
                    }
                    "keyboard" => {
                        // One of the layouts the screen offers, as a person could pick; applied
                        // to the live session the same way their pick is.
                        let offered = keyboards(&ui);
                        let want = value.trim();
                        if !offered.iter().any(|k| k == want) {
                            return Err(format!(
                                "no keyboard layout `{want}` offered; there is: {}",
                                offered.join(", ")
                            ));
                        }
                        ui.set_onboard_keyboard(want.into());
                        ui.invoke_onboard_keyboard_chosen(want.into());
                    }
                    "timezone" => {
                        let want = value.trim();
                        if let Some(why) =
                            installer_rules::timezone_problem(want, Path::new("/usr/share/zoneinfo"))
                        {
                            return Err(why);
                        }
                        ui.set_onboard_timezone(want.into());
                    }
                    "disk" => {
                        // A typo here costs a disk, so the name has to be one the installer
                        // actually found rather than anything the caller cares to type.
                        let found = disks(&ui);
                        let names: Vec<String> = found
                            .iter()
                            .filter_map(|d| d["name"].as_str().map(String::from))
                            .collect();
                        let want = value.trim().trim_start_matches("/dev/");
                        if !names.iter().any(|n| n == want) {
                            return Err(if names.is_empty() {
                                "the installer found no disks on this machine".to_string()
                            } else {
                                format!("no disk `{want}` here; it found: {}", names.join(", "))
                            });
                        }
                        ui.set_onboard_selected_disk(want.into());
                    }
                    // The Disk screen's "Encrypt with my password". Only a plain yes or no: a
                    // value misread as "off" would leave a disk readable by whoever takes it.
                    "encrypt" => match parse_switch(&value) {
                        Some(on) => ui.set_onboard_encrypt(on),
                        None => {
                            return Err(format!(
                                "encrypt takes true or false, not `{}`",
                                value.trim()
                            ))
                        }
                    },
                    other => {
                        return Err(format!(
                            "no field `{other}` in the installer; it takes: {}",
                            FIELDS.join(", ")
                        ))
                    }
                }

                // A password that went in echoes back as a flag, never as itself.
                let shown = if field.starts_with("password") {
                    serde_json::Value::Bool(!value.is_empty())
                } else {
                    serde_json::Value::String(value)
                };
                Ok(serde_json::json!({
                    "field": field,
                    "value": shown,
                    // What the screen derived from it, so a caller setting a name sees the
                    // username and computer name that came with it.
                    "username": ui.get_onboard_input_username().to_string(),
                    "hostname": ui.get_onboard_input_hostname().to_string(),
                    "blocked_by": blocked_by(&ui).map(serde_json::Value::String)
                        .unwrap_or(serde_json::Value::Null),
                }))
            },
        )
        .action(
            Action::new("installer_go_to", "Show a step of the installer")
                .arg(Param::text("step").describe("welcome, you, disk, review")),
            move |args| {
                let ui = goto_ui()?;
                let want = args["step"].as_str().unwrap_or_default().trim().to_lowercase();
                let phase = step_for(&want).ok_or_else(|| {
                    let names: Vec<&str> = STEPS.iter().map(|(n, _)| *n).collect();
                    format!("no step called `{want}`; there is: {}", names.join(", "))
                })?;
                if ui.get_onboard_installing() {
                    return Err("an install is running; the installer cannot be moved".into());
                }
                ui.set_onboard_phase(phase);
                // Also the way back from "Try it first": the installer is screen 2, and a
                // caller asking for one of its steps wants to see it.
                if ui.get_current_screen() != 2 {
                    ui.set_current_screen(2);
                    ui.invoke_navigate(2);
                }
                Ok(serde_json::json!({ "step": step_name(true, phase) }))
            },
        )
        .action(
            // Deferred and dangerous, and both words are meant. It repartitions the chosen disk
            // and everything on it is gone; and it returns the moment the work is handed to the
            // installer thread, which then reports through `progress` and `status` for several
            // minutes. A caller that treats the reply as "installed" is wrong on both counts.
            Action::new(
                "installer_install",
                "Erase the chosen disk and install Yantrik OS onto it. What was on the disk is \
                 not recoverable.",
            )
            .risk("dangerous")
            .defers(),
            move |_| {
                let ui = install_ui()?;
                if ui.get_onboard_installing() {
                    return Err("an install is already running".into());
                }
                // The form's own checks, run before we promise anything, so a refusal names the
                // field rather than failing halfway through partitioning.
                if let Some(why) = blocked_by(&ui) {
                    return Err(why);
                }
                let disk = ui.get_onboard_selected_disk();
                ui.set_onboard_install_error(Default::default());
                ui.set_onboard_installing(true);
                ui.invoke_onboard_install_to_disk(
                    ui.get_onboard_input_username(),
                    ui.get_onboard_input_password(),
                    ui.get_onboard_input_full_name(),
                    ui.get_onboard_input_hostname(),
                    ui.get_onboard_keyboard(),
                    ui.get_onboard_timezone(),
                    disk.clone(),
                    ui.get_onboard_encrypt(),
                );
                ui.set_onboard_phase(step::INSTALLING);
                if ui.get_current_screen() != 2 {
                    ui.set_current_screen(2);
                    ui.invoke_navigate(2);
                }
                Ok(serde_json::json!({
                    "installing_to": disk.to_string(),
                    "encrypted": ui.get_onboard_encrypt(),
                    "watch": "describe the shell and read installer.progress and installer.status; \
                              the machine restarts by itself about ten seconds after it finishes",
                }))
            },
        )
        .action(
            Action::new("installer_reboot", "Reboot into the installed system")
                .risk("dangerous"),
            move |_| {
                let ui = reboot_ui()?;
                if ui.get_onboard_phase() != step::INSTALLED {
                    return Err(format!(
                        "the install has not finished — the installer is on the {} step",
                        step_name(true, ui.get_onboard_phase())
                    ));
                }
                ui.invoke_onboard_install_reboot();
                Ok(serde_json::json!({ "rebooting": true }))
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_installer_names_its_four_screens_and_what_follows() {
        let names: Vec<&str> = (0..=5).map(|p| step_name(true, p)).collect();
        assert_eq!(names, ["welcome", "you", "disk", "review", "installing", "installed"]);
        assert_eq!(step_name(true, 9), "unknown");
    }

    #[test]
    fn the_first_boot_onboarding_keeps_its_names() {
        assert_eq!(step_name(false, 0), "waking");
        assert_eq!(step_name(false, 2), "greeting");
        assert_eq!(step_name(false, 3), "identity");
        assert_eq!(step_name(false, 4), "interests");
        assert_eq!(step_name(false, 8), "ai-provider");
        assert_eq!(step_name(false, 10), "ready");
    }

    #[test]
    fn go_to_takes_the_new_names_and_the_old_ones() {
        assert_eq!(step_for("welcome"), Some(step::WELCOME));
        assert_eq!(step_for("you"), Some(step::YOU));
        assert_eq!(step_for("disk"), Some(step::DISK));
        assert_eq!(step_for("review"), Some(step::REVIEW));
        assert_eq!(step_for("identity"), Some(step::YOU));
        assert_eq!(step_for("summary"), Some(step::REVIEW));
        assert_eq!(step_for("keyboard"), Some(step::WELCOME));
        // Consequences of Install, and the old personalisation steps, are not places to go.
        for gone in ["installing", "installed", "interests", "ai-mode", "hardware", ""] {
            assert_eq!(step_for(gone), None, "{gone}");
        }
    }

    #[test]
    fn encryption_is_switched_only_by_a_plain_yes_or_no() {
        assert_eq!(canonical_field("encryption"), "encrypt");
        assert_eq!(canonical_field("LUKS"), "encrypt");
        for yes in ["true", "Yes", " on ", "1"] {
            assert_eq!(parse_switch(yes), Some(true), "{yes}");
        }
        for no in ["false", "NO", "off", "0"] {
            assert_eq!(parse_switch(no), Some(false), "{no}");
        }
        for unclear in ["", "encrypted", "maybe", "nope"] {
            assert_eq!(parse_switch(unclear), None, "{unclear}");
        }
    }

    #[test]
    fn fields_are_found_by_what_the_screen_calls_them() {
        assert_eq!(canonical_field("Computer_Name"), "hostname");
        assert_eq!(canonical_field("name"), "full_name");
        assert_eq!(canonical_field("target"), "disk");
        assert_eq!(canonical_field("layout"), "keyboard");
        assert_eq!(canonical_field("tz"), "timezone");
        assert_eq!(canonical_field(" password "), "password");
        for f in FIELDS {
            assert_eq!(canonical_field(f), *f);
        }
        // The installer no longer asks these; they are not quietly accepted.
        assert!(!FIELDS.contains(&"companion_name"));
        assert!(!FIELDS.contains(&"location"));
    }

    fn ready_form(zoneinfo: &Path) -> Form {
        std::fs::create_dir_all(zoneinfo.join("Europe")).unwrap();
        std::fs::write(zoneinfo.join("Europe/Berlin"), b"TZif").unwrap();
        Form {
            full_name: "Ada Lovelace".into(),
            username: "ada".into(),
            password: "pw".into(),
            password_confirm: "pw".into(),
            hostname: "ada-yantrik".into(),
            timezone: "Europe/Berlin".into(),
            selected_disk: "sda".into(),
            disks: vec!["sda".into(), "nvme0n1".into()],
        }
    }

    #[test]
    fn blocked_by_names_the_field_a_person_would_be_stopped_at() {
        let zi = std::env::temp_dir().join(format!("yos-ci-zoneinfo-{}", std::process::id()));
        let ok = ready_form(&zi);
        assert_eq!(ok.problem(&zi), None);

        let with = |f: &dyn Fn(&mut Form)| {
            let mut form = ready_form(&zi);
            f(&mut form);
            form.problem(&zi).expect("blocked")
        };
        assert!(with(&|f| f.username = "Ada L".into()).starts_with("username:"));
        assert!(with(&|f| f.username = "root".into()).contains("taken"));
        assert!(with(&|f| f.password_confirm = "pW".into()).contains("match"));
        assert!(with(&|f| f.password = String::new()).starts_with("password:"));
        assert!(with(&|f| f.hostname = "-bad".into()).starts_with("hostname:"));
        assert!(with(&|f| f.full_name = "a:b".into()).starts_with("full_name:"));
        assert!(with(&|f| f.timezone = "Mars/Base".into()).starts_with("timezone:"));
        assert_eq!(
            with(&|f| f.selected_disk = String::new()),
            "no disk chosen; this machine has: sda, nvme0n1"
        );
        assert_eq!(
            with(&|f| {
                f.selected_disk = String::new();
                f.disks.clear();
            }),
            "no disk to install to was found"
        );
        // Asked in the order the screens ask: the account before the disk before the clock.
        assert!(with(&|f| {
            f.username = String::new();
            f.selected_disk = String::new();
        })
        .starts_with("username:"));
        let _ = std::fs::remove_dir_all(&zi);
    }
}
