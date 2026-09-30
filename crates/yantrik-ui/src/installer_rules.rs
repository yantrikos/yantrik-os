//! What the installer accepts, and what it fills in by itself.
//!
//! One set of rules, asked by three callers: the installer's screens (through pure callbacks,
//! so the message under a field and the Next button that stays disabled come from here), the
//! control surface's `blocked_by`, and the install itself. The old wizard checked three things
//! in Slint when Next was pressed and nothing else anywhere, so a username of `Ada Lovelace`
//! reached `useradd`, failed there, and the failure was swallowed: the machine installed with
//! no account on it.
//!
//! This file depends on nothing but `std`, on purpose. The headless UI preview
//! (`tests/ui-preview`) includes it by path, so the screens it draws are validated by these
//! rules and not by a stand-in that could drift from them.

/// Keyboard layouts the Welcome screen offers without scrolling a list of two hundred. The
/// detected layout is added when it is not one of these.
///
/// India has no row of its own: XKB's `in` layout is Devanagari, and most keyboards sold in
/// India are US-layout, so the US row says so instead of offering a layout that would turn
/// every key into Hindi.
pub const COMMON_LAYOUTS: &[(&str, &str)] = &[
    ("us", "English (US) — also most keyboards in India"),
    ("gb", "English (UK)"),
    ("de", "German"),
    ("fr", "French"),
    ("es", "Spanish"),
    ("it", "Italian"),
    ("jp", "Japanese"),
    ("br", "Portuguese (Brazil)"),
    ("ru", "Russian"),
];

/// The layout used when nothing better is known.
pub const DEFAULT_LAYOUT: &str = "us";

/// Longest login name `useradd` takes on Debian.
const MAX_USERNAME: usize = 32;

/// Longest hostname label (RFC 1123).
const MAX_HOSTNAME: usize = 63;

/// Names the installed system already uses for an account or a group.
///
/// `useradd -m` creates a group named after the user, so a person called `audio` would fail
/// against the existing group, and a person called `sudo` would be worse than failing. The
/// list is Debian's base accounts and groups plus the ones this image's packages add.
const RESERVED_NAMES: &[&str] = &[
    "root", "daemon", "bin", "sys", "sync", "games", "man", "lp", "mail", "news", "uucp",
    "proxy", "www-data", "backup", "list", "irc", "gnats", "nobody", "nogroup", "_apt",
    "messagebus", "sshd", "polkitd", "avahi", "pulse", "rtkit", "colord", "saned", "usbmux",
    "geoclue", "tss", "dnsmasq", "speech-dispatcher", "seat", "sudo", "adm", "tty", "disk",
    "kmem", "dialout", "fax", "voice", "cdrom", "floppy", "tape", "audio", "video", "input",
    "render", "kvm", "sgx", "plugdev", "netdev", "staff", "users", "shadow", "utmp", "operator",
    "src", "crontab", "ssl-cert", "lpadmin", "bluetooth", "scanner", "systemd-journal",
    "systemd-network", "systemd-resolve", "systemd-timesync", "systemd-coredump", "admin",
];

/// A username made from the first word of the person's name: lowercased, with anything a
/// login name cannot carry dropped. `Ada Lovelace` becomes `ada`, `Jean-Luc Picard` becomes
/// `jean-luc`, `José` becomes `jos`. Empty when there is nothing usable, which the screen
/// shows as an empty field to fill in rather than a name nobody chose.
pub fn derive_username(full_name: &str) -> String {
    let first = full_name.split_whitespace().next().unwrap_or("");
    let mut out = String::new();
    for c in first.chars().flat_map(char::to_lowercase) {
        let fits = c.is_ascii_lowercase() || c == '_' || (!out.is_empty() && (c.is_ascii_digit() || c == '-'));
        if fits {
            out.push(c);
        }
        if out.len() == MAX_USERNAME {
            break;
        }
    }
    // A trailing hyphen is legal and looks like a typo.
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// What is wrong with a login name, or `None` when `useradd` will take it and it is not
/// already somebody else's.
pub fn username_problem(username: &str) -> Option<String> {
    if username.is_empty() {
        return Some("Choose a username".into());
    }
    if username.len() > MAX_USERNAME {
        return Some(format!("A username can be at most {MAX_USERNAME} characters"));
    }
    let mut chars = username.chars();
    let first = chars.next().unwrap_or('_');
    if !(first.is_ascii_lowercase() || first == '_') {
        return Some("A username starts with a lowercase letter".into());
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
        return Some("Use lowercase letters, digits, - and _ only".into());
    }
    if RESERVED_NAMES.contains(&username) || username.starts_with("systemd-") {
        return Some(format!("`{username}` is taken by the system; choose another"));
    }
    None
}

/// The hostname a machine gets when nobody names it: `<username>-yantrik`, so two people's
/// machines on one network do not both answer to `yantrik`.
pub fn hostname_for(username: &str) -> String {
    // Underscores are fine in a login and not in a hostname.
    let base: String = username
        .chars()
        .map(|c| if c == '_' { '-' } else { c })
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
        .collect();
    let base = base.trim_matches('-');
    if base.is_empty() {
        return "yantrik".into();
    }
    let suffix = "-yantrik";
    let room = MAX_HOSTNAME - suffix.len();
    let base = &base[..base.len().min(room)];
    format!("{}{suffix}", base.trim_end_matches('-'))
}

/// What is wrong with a hostname, or `None` when it is one RFC 1123 label.
pub fn hostname_problem(hostname: &str) -> Option<String> {
    if hostname.is_empty() {
        return Some("The computer needs a name".into());
    }
    if hostname.len() > MAX_HOSTNAME {
        return Some(format!("A computer name can be at most {MAX_HOSTNAME} characters"));
    }
    if !hostname.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Some("Use lowercase letters, digits and - only".into());
    }
    if hostname.starts_with('-') || hostname.ends_with('-') {
        return Some("A computer name cannot start or end with -".into());
    }
    None
}

/// What is wrong with the full name. It is optional; it only cannot carry the characters that
/// separate fields in /etc/passwd, which `chfn` refuses and the install used to ignore.
pub fn full_name_problem(full_name: &str) -> Option<String> {
    if full_name.chars().any(|c| matches!(c, ':' | ',' | '=') || c.is_control()) {
        return Some("A name cannot contain : , or =".into());
    }
    None
}

/// What is wrong with the password pair, or `None` when there is one and both boxes agree.
///
/// No strength rule: the password is the person's to choose, and the installer is not the
/// place to argue about it. An empty one is refused because it would leave an account that
/// the lock screen at boot (#415) cannot ask anything of. So is one with a control character
/// (a newline, a tab, an escape), which the control surface can set and no keyboard types: it
/// also unlocks the encrypted disk, whose prompt ends a line at the first newline.
pub fn password_problem(password: &str, confirm: &str) -> Option<String> {
    if password.is_empty() {
        return Some("Choose a password".into());
    }
    if password.chars().any(char::is_control) {
        return Some("A password cannot contain a line break, tab or other control character".into());
    }
    if confirm.is_empty() {
        return Some("Type the password again to confirm it".into());
    }
    if password != confirm {
        return Some("The passwords don't match".into());
    }
    None
}

/// The shape of an IANA zone name: `America/Chicago`, `Etc/UTC`, `Asia/Kolkata`, `UTC`.
///
/// The value may come from the network and ends up as a path under /usr/share/zoneinfo and an
/// argument to a system command, so anything that could leave that directory is refused before
/// it is looked up at all.
pub fn is_plausible_timezone(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.starts_with('/')
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '+' | '-'))
}

/// What is wrong with a timezone, checked against the zone database under `zoneinfo`.
pub fn timezone_problem(timezone: &str, zoneinfo: &std::path::Path) -> Option<String> {
    if timezone.is_empty() {
        return Some("Choose a timezone, e.g. Europe/Berlin".into());
    }
    if !is_plausible_timezone(timezone) || !zoneinfo.join(timezone).is_file() {
        return Some(format!("`{timezone}` is not a timezone this system knows, e.g. Europe/Berlin"));
    }
    None
}

/// A layout name XKB could have: `us`, `de`, `latam`, `brai`. Checked before it is written into
/// a file the session sources, so a stray quote or newline cannot become a second line there.
pub fn is_plausible_layout(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && s.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// The layout named in the text of `/etc/default/keyboard`, the first one when several are
/// listed (`XKBLAYOUT="us,de"` means US with German as the switch).
pub fn parse_keyboard_file(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let value = line.trim().strip_prefix("XKBLAYOUT=")?;
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
        let first = value.split(',').next()?.trim();
        is_plausible_layout(first).then(|| first.to_string())
    })
}

/// The layout in `localectl status` output: the `X11 Layout:` line.
pub fn parse_localectl_layout(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let value = line.trim().strip_prefix("X11 Layout:")?;
        let first = value.split(',').next()?.trim();
        is_plausible_layout(first).then(|| first.to_string())
    })
}

/// The installed system's `/etc/default/keyboard`, in the shape `keyboard-configuration`
/// writes it, so `setupcon` and systemd-localed both read it.
pub fn keyboard_file(layout: &str) -> String {
    format!(
        "# Written by the Yantrik OS installer.\nXKBMODEL=\"pc105\"\nXKBLAYOUT=\"{layout}\"\nXKBVARIANT=\"\"\nXKBOPTIONS=\"\"\nBACKSPACE=\"guess\"\n"
    )
}

/// A labwc `environment` file with its keyboard layout set to `layout`, the rest kept.
///
/// labwc reads `XKB_DEFAULT_LAYOUT` from this file, which is what actually decides the keys in
/// the desktop session — /etc/default/keyboard is the console's, and Wayland does not read it.
pub fn environment_with_layout(existing: &str, layout: &str) -> String {
    let mut out: String = existing
        .lines()
        .filter(|line| !line.trim_start().starts_with("XKB_DEFAULT_LAYOUT="))
        .map(|line| format!("{line}\n"))
        .collect();
    out.push_str(&format!("XKB_DEFAULT_LAYOUT={layout}\n"));
    out
}

/// The layouts to offer: the common ones, with the detected one first when it is not already
/// among them. Each is `(code, label)`.
pub fn layout_choices(detected: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if is_plausible_layout(detected) && !COMMON_LAYOUTS.iter().any(|(c, _)| *c == detected) {
        out.push((detected.to_string(), format!("Detected layout ({detected})")));
    }
    out.extend(COMMON_LAYOUTS.iter().map(|(c, l)| (c.to_string(), l.to_string())));
    out
}

/// The label a layout code is shown with.
pub fn layout_label(code: &str) -> String {
    COMMON_LAYOUTS
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, l)| l.to_string())
        .unwrap_or_else(|| format!("Layout {code}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_username_comes_from_the_first_name() {
        assert_eq!(derive_username("Ada Lovelace"), "ada");
        assert_eq!(derive_username("  Jean-Luc Picard"), "jean-luc");
        assert_eq!(derive_username("José"), "jos");
        assert_eq!(derive_username("O'Brien"), "obrien");
        // Digits and hyphens cannot lead.
        assert_eq!(derive_username("2Pac"), "pac");
        assert_eq!(derive_username("-x-"), "x");
        assert_eq!(derive_username(""), "");
        assert_eq!(derive_username("李雷"), "");
        let long = "a".repeat(50);
        assert_eq!(derive_username(&long).len(), 32);
        // Whatever is derived is something the rules accept, or nothing at all.
        for name in ["Ada Lovelace", "Jean-Luc Picard", "José", "Zoë", "_x"] {
            let u = derive_username(name);
            assert!(u.is_empty() || username_problem(&u).is_none(), "{name} -> {u}");
        }
    }

    #[test]
    fn usernames_useradd_would_refuse_are_refused_here() {
        assert!(username_problem("ada").is_none());
        assert!(username_problem("_svc").is_none());
        assert!(username_problem("ada-l_2").is_none());
        assert!(username_problem("yantrik").is_none(), "the live user's name is fine to keep");
        for bad in ["", "Ada", "ada lovelace", "1ada", "-ada", "ada.l", "ada@home"] {
            assert!(username_problem(bad).is_some(), "{bad:?} should be refused");
        }
        assert!(username_problem(&"a".repeat(33)).is_some());
        assert!(username_problem(&"a".repeat(32)).is_none());
    }

    #[test]
    fn names_the_system_owns_are_refused() {
        for taken in ["root", "sudo", "audio", "video", "input", "nobody", "systemd-foo"] {
            let why = username_problem(taken).expect("reserved");
            assert!(why.contains("taken"), "{taken}: {why}");
        }
    }

    #[test]
    fn a_hostname_is_derived_from_the_username() {
        assert_eq!(hostname_for("ada"), "ada-yantrik");
        assert_eq!(hostname_for("_ada_l"), "ada-l-yantrik");
        assert_eq!(hostname_for(""), "yantrik");
        let long = hostname_for(&"a".repeat(32));
        assert!(long.len() <= 63 && hostname_problem(&long).is_none(), "{long}");
        for u in ["ada", "_x", "a-b_c", "z9"] {
            assert!(hostname_problem(&hostname_for(u)).is_none(), "{u}");
        }
    }

    #[test]
    fn hostnames_are_one_rfc_1123_label() {
        assert!(hostname_problem("ada-yantrik").is_none());
        assert!(hostname_problem("box1").is_none());
        for bad in ["", "-ada", "ada-", "ada_box", "Ada", "ada.local", "ada box"] {
            assert!(hostname_problem(bad).is_some(), "{bad:?} should be refused");
        }
        assert!(hostname_problem(&"a".repeat(64)).is_some());
    }

    #[test]
    fn a_full_name_may_be_empty_but_not_break_passwd() {
        assert!(full_name_problem("").is_none());
        assert!(full_name_problem("Ada Lovelace").is_none());
        assert!(full_name_problem("Zoë O'Brien-Śmith").is_none());
        for bad in ["Ada:x", "Ada, Countess", "a=b", "Ada\nroot"] {
            assert!(full_name_problem(bad).is_some(), "{bad:?}");
        }
    }

    #[test]
    fn the_password_pair_is_caught_before_install() {
        assert_eq!(password_problem("", "").as_deref(), Some("Choose a password"));
        assert!(password_problem("pw", "").unwrap().contains("again"));
        assert!(password_problem("pw", "pW").unwrap().contains("match"));
        assert!(password_problem("correct horse", "correct horse").is_none());
        assert!(password_problem("über-Straße ключ", "über-Straße ключ").is_none(), "any layout's letters");
        for typed_by_no_keyboard in ["line\nbreak", "tab\there", "esc\u{1b}[1m"] {
            assert!(
                password_problem(typed_by_no_keyboard, typed_by_no_keyboard).unwrap().contains("control"),
                "{typed_by_no_keyboard:?}"
            );
        }
    }

    #[test]
    fn timezones_must_exist_in_the_zone_database() {
        let dir = std::env::temp_dir().join(format!("yos-zoneinfo-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("America")).unwrap();
        std::fs::write(dir.join("America/Chicago"), b"TZif").unwrap();
        std::fs::write(dir.join("UTC"), b"TZif").unwrap();
        assert!(timezone_problem("America/Chicago", &dir).is_none());
        assert!(timezone_problem("UTC", &dir).is_none());
        assert!(timezone_problem("", &dir).is_some());
        assert!(timezone_problem("Mars/Olympus", &dir).is_some());
        // A directory is not a zone.
        assert!(timezone_problem("America", &dir).is_some());
        for bad in ["../../etc/passwd", "/etc/passwd", "America/Chicago; rm -rf /", "$(id)"] {
            assert!(timezone_problem(bad, &dir).is_some(), "{bad:?}");
            assert!(!is_plausible_timezone(bad), "{bad:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_keyboard_file_is_read_the_way_debian_writes_it() {
        let debian = "# KEYBOARD CONFIGURATION FILE\n\nXKBMODEL=\"pc105\"\nXKBLAYOUT=\"de\"\nXKBVARIANT=\"\"\nXKBOPTIONS=\"\"\n\nBACKSPACE=\"guess\"\n";
        assert_eq!(parse_keyboard_file(debian).as_deref(), Some("de"));
        assert_eq!(parse_keyboard_file("XKBLAYOUT=gb\n").as_deref(), Some("gb"));
        assert_eq!(parse_keyboard_file("XKBLAYOUT=\"us,ru\"\n").as_deref(), Some("us"));
        assert_eq!(parse_keyboard_file("XKBLAYOUT='fr'").as_deref(), Some("fr"));
        assert_eq!(parse_keyboard_file("XKBMODEL=\"pc105\"\n"), None);
        assert_eq!(parse_keyboard_file("XKBLAYOUT=\"\"\n"), None);
        assert_eq!(parse_keyboard_file("XKBLAYOUT=\"us;reboot\"\n"), None);
        // What the installer writes, it reads back.
        assert_eq!(parse_keyboard_file(&keyboard_file("jp")).as_deref(), Some("jp"));
    }

    #[test]
    fn localectl_names_the_x11_layout() {
        let out = "   System Locale: LANG=en_US.UTF-8\n       VC Keymap: us\n      X11 Layout: gb\n       X11 Model: pc105\n";
        assert_eq!(parse_localectl_layout(out).as_deref(), Some("gb"));
        assert_eq!(parse_localectl_layout("      X11 Layout: n/a\n"), None);
        assert_eq!(parse_localectl_layout("   VC Keymap: us\n"), None);
    }

    #[test]
    fn layouts_that_could_break_a_sourced_file_are_refused() {
        for good in ["us", "de", "latam", "brai", "custom_1"] {
            assert!(is_plausible_layout(good), "{good}");
        }
        for bad in ["", "US", "us\"", "us\nXKB", "1us", "us de", &"a".repeat(33)] {
            assert!(!is_plausible_layout(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_session_environment_gets_exactly_one_layout_line() {
        let env = "WLR_RENDERER=pixman\nXKB_DEFAULT_LAYOUT=us\nSLINT_BACKEND=winit\n";
        let out = environment_with_layout(env, "de");
        assert_eq!(out.matches("XKB_DEFAULT_LAYOUT=").count(), 1, "{out}");
        assert!(out.contains("XKB_DEFAULT_LAYOUT=de\n"));
        assert!(out.contains("WLR_RENDERER=pixman\n") && out.contains("SLINT_BACKEND=winit\n"));
        assert_eq!(environment_with_layout("", "fr"), "XKB_DEFAULT_LAYOUT=fr\n");
    }

    #[test]
    fn the_detected_layout_is_offered_first_and_only_once() {
        let common = layout_choices("de");
        assert_eq!(common.len(), COMMON_LAYOUTS.len(), "de is already listed");
        let odd = layout_choices("latam");
        assert_eq!(odd[0].0, "latam");
        assert_eq!(odd.len(), COMMON_LAYOUTS.len() + 1);
        assert_eq!(layout_choices("bad\"").len(), COMMON_LAYOUTS.len());
        assert!(layout_label("us").contains("India"));
        assert_eq!(layout_label("latam"), "Layout latam");
    }
}
