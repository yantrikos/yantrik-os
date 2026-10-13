//! Every key `config/labwc/rc.xml` binds, and every row of `menu.xml`, does something real
//! (tests only).
//!
//! `rc_keys` keeps the screens and the file honest about which KEYS are bound. This is the other
//! half: what each binding RUNS. A labwc action name misspelt, or one from a newer labwc than the
//! machine ships, is dropped by labwc with a line in a log nobody reads, and the key does nothing.
//! An `Execute` that calls a program the image does not install fails the same way. A `yos act
//! shell show_screen screen=…` naming a screen the shell does not have is refused into a terminal
//! nobody is looking at. Any of those is a key the desktop advertises that is dead on the machine.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The file with its `<!-- … -->` comments cut out, so an action only talked about is not counted.
fn uncommented(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(open) = rest.find("<!--") {
        out.push_str(&rest[..open]);
        rest = rest[open..].find("-->").map_or("", |close| &rest[open + close + 3..]);
    }
    out.push_str(rest);
    out
}

/// Each `<action name="…">` in the file, with the rest of its tag.
fn actions(xml: &str) -> Vec<(String, String)> {
    uncommented(xml)
        .split("<action name=\"")
        .skip(1)
        .filter_map(|part| {
            let end = part.find('"')?;
            let tag_end = part.find('>').unwrap_or(part.len());
            Some((part[..end].to_string(), part[end..tag_end].to_string()))
        })
        .collect()
}

/// Each `<command>` an `Execute` runs, with XML entities put back.
fn commands(xml: &str) -> Vec<String> {
    uncommented(xml)
        .split("<command>")
        .skip(1)
        .map(|c| c.split("</command>").next().unwrap_or_default().replace("&amp;", "&"))
        .collect()
}

/// The actions labwc 0.8 implements (labwc-actions(5), the version Debian trixie ships and
/// build-debian-iso.sh installs). An action outside this list is ignored by labwc at load.
const LABWC_ACTIONS: &[&str] = &[
    "AutoPlace", "Close", "Debug", "DisableScrollWheelEmulation", "DisableTabletMouseEmulation",
    "EnableScrollWheelEmulation", "EnableTabletMouseEmulation", "Execute", "Exit", "FitToOutput",
    "Focus", "FocusOutput", "ForEach", "GoToDesktop", "GrowToEdge", "HideCursor", "Iconify", "If",
    "Kill", "Lower", "Maximize", "Move", "MoveRelative", "MoveTo", "MoveToCursor", "MoveToEdge",
    "MoveToOutput", "NextWindow", "None", "PreviousWindow", "Raise", "Reconfigure", "Resize",
    "ResizeRelative", "ResizeTo", "SendToDesktop", "SetDecorations", "Shade", "ShowMenu",
    "ShrinkToEdge", "SnapToEdge", "SnapToRegion", "ToggleAlwaysOnBottom", "ToggleAlwaysOnTop",
    "ToggleDecorations", "ToggleFullscreen", "ToggleKeybinds", "ToggleMagnify", "ToggleMaximize",
    "ToggleOmnipresent", "ToggleShade", "ToggleSnapToEdge", "ToggleSnapToRegion",
    "ToggleTabletMouseEmulation", "ToggleTearing", "UnMaximize", "UnSnap", "Unfocus", "Unshade",
    "VirtualOutputAdd", "VirtualOutputRemove", "WarpCursor", "ZoomIn", "ZoomOut",
];

/// The programs a binding may start, and the Debian package that puts each on the machine, as
/// named in deploy/yantrik-os/build-debian-iso.sh. `None` is part of the base system or the
/// shell's own install.
const PROGRAMS: &[(&str, Option<&str>)] = &[
    ("/opt/yantrik/bin/yos", None),
    ("sh", None),
    ("mkdir", None),
    ("date", None),
    ("grep", None),
    ("grim", Some("grim")),
    ("slurp", Some("slurp")),
    ("wpctl", Some("wireplumber")),
    ("brightnessctl", Some("brightnessctl")),
];

/// The programs a command line starts: its first word, and inside `sh -c '…'` the first word of
/// each command in it — after `&&`, `||`, `;`, `|`, `$(` and the shell's own `if`/`then`/`else`.
fn programs_in(command: &str) -> Vec<String> {
    let mut out = vec![command.split_whitespace().next().unwrap_or_default().to_string()];
    let Some(script) = command.strip_prefix("sh -c ") else { return out };
    let mut script = script.trim().trim_matches('\'').to_string();
    for sep in ["&&", "||", ";", "|", "$(", "\""] {
        script = script.replace(sep, "\n");
    }
    for piece in script.lines() {
        let mut words = piece.split_whitespace().skip_while(|w| ["if", "then", "else", "fi"].contains(w));
        if let Some(word) = words.next().map(|w| w.trim_end_matches(')')) {
            if !word.starts_with('~') && !word.starts_with('@') && !word.contains('%') {
                out.push(word.to_string());
            }
        }
    }
    out
}

/// The value of `key=` in a `yos act` command line, unquoted.
fn arg<'a>(command: &'a str, key: &str) -> Option<&'a str> {
    let after = command.split(&format!(" {key}=")).nth(1)?;
    Some(match after.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next().unwrap_or_default(),
        None => after.split_whitespace().next().unwrap_or_default(),
    })
}

#[test]
fn every_action_rc_xml_and_menu_xml_name_is_one_labwc_implements() {
    let mut seen = 0;
    for file in ["config/labwc/rc.xml", "config/labwc/menu.xml"] {
        for (name, _) in actions(&read(file)) {
            seen += 1;
            assert!(
                LABWC_ACTIONS.contains(&name.as_str()),
                "{file} names the action {name:?}, which labwc 0.8 does not implement: the key or \
                 menu row would do nothing"
            );
        }
    }
    assert!(seen >= 40, "found only {seen} actions; the scan is reading the wrong thing");
}

#[test]
fn every_program_a_binding_starts_is_installed_by_the_image() {
    let iso = read("deploy/yantrik-os/build-debian-iso.sh");
    let mut seen = 0;
    for command in commands(&read("config/labwc/rc.xml")) {
        for program in programs_in(&command) {
            seen += 1;
            let Some((_, package)) = PROGRAMS.iter().find(|(p, _)| *p == program) else {
                panic!("rc.xml starts `{program}`, which PROGRAMS does not know: {command}");
            };
            if let Some(package) = package {
                assert!(
                    iso.split_whitespace().any(|w| w == *package),
                    "rc.xml starts `{program}`, but the image never installs `{package}`: {command}"
                );
            }
        }
    }
    assert!(seen >= 15, "found only {seen} programs; the scan is reading the wrong thing");
}

/// `rc_keys` checks the action is published; this checks its arguments are ones it accepts.
#[test]
fn every_screen_and_desktop_a_binding_names_exists() {
    let rc = read("config/labwc/rc.xml");
    for command in commands(&rc) {
        if let Some(screen) = arg(&command, "screen") {
            assert!(
                screen == "launchpad" || crate::control::SCREENS.iter().any(|(n, _)| *n == screen),
                "rc.xml shows the screen {screen:?}, which the shell does not have: {command}"
            );
        }
        if let Some(name) = arg(&command, "name") {
            assert_eq!(crate::wire::dock::canonical_id(name), name, "rc.xml opens {name:?}, not an app id: {command}");
        }
    }
    let desktops: u32 = rc
        .split("<number>")
        .nth(1)
        .and_then(|n| n.split('<').next())
        .and_then(|n| n.trim().parse().ok())
        .expect("rc.xml says how many desktops there are");
    for (name, rest) in actions(&rc) {
        if name != "GoToDesktop" && name != "SendToDesktop" {
            continue;
        }
        let to = rest.split("to=\"").nth(1).and_then(|t| t.split('"').next()).unwrap_or_default();
        let ok = match to.parse::<u32>() {
            Ok(n) => (1..=desktops).contains(&n),
            Err(_) => ["left", "right", "current", "last"].contains(&to),
        };
        assert!(ok, "rc.xml's {name} goes to desktop {to:?}, and there are {desktops}");
    }
}

#[test]
fn the_scan_reads_commands_the_way_rc_xml_writes_them() {
    assert_eq!(
        programs_in("sh -c 'mkdir -p ~/Pictures && grim -g \"$(slurp)\" ~/x-$(date +%Y).png'"),
        ["sh", "mkdir", "grim", "slurp", "date"]
    );
    assert_eq!(
        programs_in("sh -c 'if wpctl get | grep -q M; then yos a; else yos b; fi || wpctl x'"),
        ["sh", "wpctl", "grep", "yos", "yos", "wpctl"]
    );
    assert_eq!(arg("yos act shell focus_window title=\"Mind View\"", "title"), Some("Mind View"));
    assert_eq!(actions("<!-- <action name=\"Nope\" /> --><action name=\"Close\" />")[0].0, "Close");
}
