//! What a theme writes for the programs the shell does not draw: labwc's `themerc`, foot's
//! colours and the GTK colour scheme, plus the colour arithmetic they share. Split from
//! `theme.rs`, which keeps the catalogue, the choice and the wiring.
//!
//! The person's own files are edited, not replaced: foot's `[colors]` section and the one GTK
//! key are swapped inside whatever else the file says, and a block this wrote is recognised by
//! its marker lines and replaced in place on the next choice.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::theme::Theme;

/// The compositor's theme as shipped. Sizes and button images stay as they are; only the colours
/// below are replaced, so the structure of the frame is written in one place.
pub(crate) const BASE_THEMERC: &str = include_str!("../../../../config/labwc/themerc");

// ── Colours ────────────────────────────────────────────────────────────

/// `#rrggbb` as three bytes.
pub(crate) fn rgb(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

pub(crate) fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// `from` moved `t` of the way (0 to 1) toward `to`: how an edge is derived from its surface, so
/// a theme states its palette and gets frame lines that belong to it.
pub(crate) fn mix(from: &str, to: &str, t: f32) -> String {
    let (a, b) = (rgb(from).unwrap_or([0; 3]), rgb(to).unwrap_or([255; 3]));
    let c = |i: usize| (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t).round() as u8;
    hex([c(0), c(1), c(2)])
}

/// The colours labwc is given, by themerc key. The close button's red is a token of its own
/// (`color-close-hover`) and every theme keeps it, so it is not here.
pub(crate) fn frame_colours(t: &Theme) -> Vec<(&'static str, String)> {
    let p = &t.palette;
    let edge = mix(&p.bg_surface, &p.text_primary, 0.14);
    let quiet_edge = mix(&p.bg_deep, &p.text_primary, 0.08);
    vec![
        ("window.active.title.bg.color", p.bg_surface.clone()),
        ("window.active.label.text.color", p.text_primary.clone()),
        ("window.active.border.color", edge.clone()),
        ("window.active.button.unpressed.image.color", p.text_primary.clone()),
        ("window.active.button.hover.bg.color", p.bg_card.clone()),
        ("window.inactive.title.bg.color", p.bg_deep.clone()),
        ("window.inactive.label.text.color", p.text_dim.clone()),
        ("window.inactive.border.color", quiet_edge),
        ("window.inactive.button.unpressed.image.color", p.text_dim.clone()),
        ("window.inactive.button.hover.bg.color", p.bg_surface.clone()),
        ("menu.items.bg.color", p.bg_surface.clone()),
        ("menu.items.text.color", p.text_primary.clone()),
        // The accent, not teal: teal marks minds, and a menu highlight is not one.
        ("menu.items.active.bg.color", p.accent.clone()),
        ("menu.items.active.text.color", p.bg_deep.clone()),
        ("menu.border.color", edge.clone()),
        // The Alt+Tab list, so the switcher is the same card as the rest.
        ("osd.bg.color", p.bg_surface.clone()),
        ("osd.border.color", edge),
        ("osd.label.text.color", p.text_primary.clone()),
        ("osd.window-switcher.item.active.bg.color", p.bg_card.clone()),
        ("osd.window-switcher.item.active.border.color", p.text_primary.clone()),
    ]
}

/// The compositor's whole themerc for this theme: the shipped file with its colours replaced.
pub fn themerc(t: &Theme) -> String {
    let colours: HashMap<&str, String> = frame_colours(t).into_iter().collect();
    let mut seen = Vec::new();
    let mut out: String = BASE_THEMERC
        .lines()
        .map(|line| {
            let key = line.split_once(':').map(|(k, _)| k.trim()).filter(|k| !line.trim_start().starts_with('#'));
            match key.and_then(|k| colours.get(k).map(|v| (k, v))) {
                Some((k, value)) => {
                    seen.push(k.to_string());
                    format!("{k}: {value}")
                }
                None => line.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    out.push('\n');
    // A key the shipped file lacks is added, so a theme never silently loses a colour.
    for (key, value) in frame_colours(t) {
        if !seen.iter().any(|s| s == key) {
            out.push_str(&format!("{key}: {value}\n"));
        }
    }
    format!("# Written by Yantrik OS for the {} theme. Choosing another theme replaces this file.\n{out}", t.name)
}

// ── foot ───────────────────────────────────────────────────────────────

pub(crate) const FOOT_BEGIN: &str = "# >>> yantrik theme (written by Yantrik OS; replaced when a theme is chosen)";
pub(crate) const FOOT_END: &str = "# <<< yantrik theme";

/// The sixteen terminal colours. The same set under every theme: what changes is the ground and
/// the text, because a person reads `ls --color` by its hues.
pub(crate) const ANSI: [(&str, &str); 16] = [
    ("regular0", "1a1e2e"),
    ("regular1", "e86b6b"),
    ("regular2", "5ac8a0"),
    ("regular3", "d4a04a"),
    ("regular4", "6fa0e0"),
    ("regular5", "a87bd4"),
    ("regular6", "5ac8d4"),
    ("regular7", "c8ccd6"),
    ("bright0", "3a4058"),
    ("bright1", "f09090"),
    ("bright2", "7ee0c0"),
    ("bright3", "e0c070"),
    ("bright4", "8fb4e3"),
    ("bright5", "c0a0e0"),
    ("bright6", "80d8e8"),
    ("bright7", "f0f2f6"),
];

/// foot's `[colors]` section for this theme, between marker lines.
pub fn foot_block(t: &Theme) -> String {
    let bare = |c: &str| c.trim_start_matches('#').to_string();
    let mut s = format!("{FOOT_BEGIN}\n[colors]\nbackground={}\nforeground={}\n", bare(&t.palette.bg_deep), bare(&t.palette.text_secondary));
    for (name, value) in ANSI {
        s.push_str(&format!("{name}={value}\n"));
    }
    s.push_str(FOOT_END);
    s.push('\n');
    s
}

/// `existing` (the person's foot.ini) with the theme's colours in place of whatever colours it
/// had: a block this wrote before is replaced where it stands; failing that a `[colors]` section
/// is replaced; failing that the block is added at the end. Everything else is kept as written.
pub fn merge_foot(existing: &str, block: &str) -> String {
    let lines: Vec<&str> = existing.lines().collect();
    let begin = lines.iter().position(|l| l.trim() == FOOT_BEGIN);
    let end = begin.and_then(|b| lines[b..].iter().position(|l| l.trim() == FOOT_END).map(|e| b + e));
    let (from, to) = match (begin, end) {
        (Some(b), Some(e)) => (b, e + 1),
        _ => match lines.iter().position(|l| l.trim() == "[colors]") {
            Some(c) => {
                // To the next section header, or the end.
                let next = lines[c + 1..].iter().position(|l| l.trim_start().starts_with('[')).map_or(lines.len(), |n| c + 1 + n);
                (c, next)
            }
            None => (lines.len(), lines.len()),
        },
    };
    let mut out: Vec<&str> = lines[..from].to_vec();
    // A blank line before a block added after other content.
    if from == lines.len() && out.last().is_some_and(|l| !l.trim().is_empty()) {
        out.push("");
    }
    let mut text = out.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(block);
    // What followed, set off by exactly one blank line however many there were, so choosing the
    // same theme twice writes the same file.
    let rest = &lines[to.min(lines.len())..];
    let rest = &rest[rest.iter().take_while(|l| l.trim().is_empty()).count()..];
    if !rest.is_empty() {
        text.push('\n');
        text.push_str(&rest.join("\n"));
        text.push('\n');
    }
    text
}

// ── GTK ────────────────────────────────────────────────────────────────

/// GTK's `color-scheme` value for the theme.
pub fn gtk_scheme(t: &Theme) -> &'static str {
    if t.dark {
        "prefer-dark"
    } else {
        "default"
    }
}

/// `existing` (a GTK `settings.ini`) with `key=value` under `[Settings]`, the other lines kept.
pub fn merge_ini_key(existing: &str, key: &str, value: &str) -> String {
    let mut lines: Vec<String> = existing.lines().map(str::to_string).collect();
    let section = lines.iter().position(|l| l.trim() == "[Settings]");
    let Some(section) = section else {
        if !lines.is_empty() && !lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push("[Settings]".into());
        lines.push(format!("{key}={value}"));
        return lines.join("\n") + "\n";
    };
    let end = lines[section + 1..].iter().position(|l| l.trim_start().starts_with('[')).map_or(lines.len(), |n| section + 1 + n);
    let at = (section + 1..end).find(|&i| lines[i].split_once('=').is_some_and(|(k, _)| k.trim() == key));
    match at {
        Some(i) => lines[i] = format!("{key}={value}"),
        None => lines.insert(section + 1, format!("{key}={value}")),
    }
    lines.join("\n") + "\n"
}

// ── Writing it out ─────────────────────────────────────────────────────

/// Where the files go: the person's config and data folders.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub config: PathBuf,
    pub data: PathBuf,
}

impl Dirs {
    pub fn from_env() -> Dirs {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/root".into()));
        let pick = |var: &str, fallback: &str| {
            std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(fallback))
        };
        Dirs { config: pick("XDG_CONFIG_HOME", ".config"), data: pick("XDG_DATA_HOME", ".local/share") }
    }

    /// Where labwc reads the theme `rc.xml` names (`Yantrik`).
    pub fn themerc(&self) -> PathBuf {
        self.data.join("themes/Yantrik/labwc/themerc")
    }
}

/// What writing changed, so only a change is announced to the compositor.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Written {
    pub themerc_changed: bool,
}

/// Write `contents` to `path` if it differs, through a temporary file and a rename, creating the
/// folder. `true` when something was written.
pub(crate) fn write_if_changed(path: &Path, contents: &str) -> Result<bool, String> {
    if std::fs::read_to_string(path).ok().as_deref() == Some(contents) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("yantrik-tmp");
    std::fs::write(&tmp, contents).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

/// Write every file the theme reaches. A file that cannot be written is reported and the others
/// are still written: a read-only foot.ini must not leave the window frames on the old theme.
pub fn write_files(t: &Theme, dirs: &Dirs) -> Result<Written, String> {
    let mut problems = Vec::new();
    let mut written = Written::default();

    match write_if_changed(&dirs.themerc(), &themerc(t)) {
        Ok(changed) => written.themerc_changed = changed,
        Err(e) => problems.push(e),
    }

    let foot = dirs.config.join("foot/foot.ini");
    let existing = std::fs::read_to_string(&foot).unwrap_or_default();
    // The first time a theme touches the person's foot.ini, their own copy is kept beside it.
    let backup = foot.with_file_name("foot.ini.before-yantrik-theme");
    if !existing.is_empty() && !existing.contains(FOOT_BEGIN) && !backup.exists() {
        if let Err(e) = std::fs::write(&backup, &existing) {
            problems.push(format!("{}: {e}", backup.display()));
        }
    }
    if let Err(e) = write_if_changed(&foot, &merge_foot(&existing, &foot_block(t))) {
        problems.push(e);
    }

    let prefer_dark = if t.dark { "1" } else { "0" };
    for gtk in ["gtk-3.0", "gtk-4.0"] {
        let path = dirs.config.join(gtk).join("settings.ini");
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        if let Err(e) = write_if_changed(&path, &merge_ini_key(&existing, "gtk-application-prefer-dark-theme", prefer_dark)) {
            problems.push(e);
        }
    }

    if problems.is_empty() {
        Ok(written)
    } else {
        Err(problems.join("; "))
    }
}
