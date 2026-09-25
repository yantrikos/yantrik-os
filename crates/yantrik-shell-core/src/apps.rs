//! App registry — .desktop file scanner + built-in app definitions.
//!
//! Scans /usr/share/applications/ and ~/.local/share/applications/ for .desktop files.
//! Parses Name, Exec, Icon, Categories, Comment, and visibility flags.
//! Provides fuzzy search for Intent Lens integration.
//!
//! ## The keys a surface declares itself with
//!
//! An app that publishes a control surface says so in its own `.desktop` file, which every Debian
//! app already ships (design/surface-sdk-2026-09-23.md, section 4):
//!
//! ```text
//! X-Yantrik-Surface=libreoffice
//! X-Yantrik-Purpose=Documents, spreadsheets and slides: open, read, edit and export them
//! X-Yantrik-Aliases=writer;calc;impress
//! X-Yantrik-Adapter=/usr/lib/yantrik/adapters/libreoffice
//! ```
//!
//! That is the whole registration. The shell reads these keys, so the app is listed in
//! `describe shell` → `apps` while it is closed, opens by its id or any alias, answers to every
//! alias on the socket bus, and has an adapter started beside it when it cannot host a surface
//! itself. This OS's own apps declare themselves with the same keys and get nothing more.

use std::path::{Path, PathBuf};

/// The `.desktop` key naming the control surface an app publishes.
pub const KEY_SURFACE: &str = "X-Yantrik-Surface";
/// What the app is FOR, in one line, for a reader choosing between apps it cannot see.
pub const KEY_PURPOSE: &str = "X-Yantrik-Purpose";
/// Other names the surface answers to, `;`-separated like every freedesktop list.
pub const KEY_ALIASES: &str = "X-Yantrik-Aliases";
/// A separate program that provides the surface for an app that cannot host one itself.
pub const KEY_ADAPTER: &str = "X-Yantrik-Adapter";

/// One spelling of a surface name, so the separator a caller arrived with is not part of it.
///
/// The protocol's fold (docs/surface-protocol.md, "Resolving a name"): trim, lowercase, and `_`
/// and space become `-`. `Download Manager`, `download_manager` and `download-manager` are one
/// question.
pub fn fold_name(name: &str) -> String {
    name.trim().to_lowercase().replace(['_', ' '], "-")
}

/// Whether `name` can be a surface's id or alias: lowercase words of letters and digits joined by
/// single `-`, as the protocol names them.
///
/// Stricter than "anything a file name can hold" on purpose. Every name here becomes a file in
/// the socket directory (`app-<name>.sock`), so a `/`, a `..` or a dot-separated reverse-DNS id
/// would be a path, not a name — refused rather than sanitised, because a name that was quietly
/// changed is a name nobody will ask for.
pub fn is_surface_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.split('-').all(|word| {
            !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

/// A parsed .desktop entry.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DesktopEntry {
    /// Display name (Name= field).
    pub name: String,
    /// Executable command (Exec= field), kept as the entry wrote it — quoting, escapes and
    /// field codes included. A launch path reads it through [`exec_argv`], which answers the
    /// field codes and tokenises per the Desktop Entry spec (#304).
    pub exec: String,
    /// `TryExec`: a program that must be on this machine for the entry to be listed at all —
    /// the standard freedesktop rule, carried as written; whoever lists entries asks it
    /// (`yantrik-ui`'s `entry_is_launchable`). `None` when the file names none. An adapter's
    /// entry names the app it wraps here, because its `Exec` runs the wrapper, which is
    /// always installed (#214).
    pub try_exec: Option<String>,
    /// Icon name or path (Icon= field).
    pub icon: String,
    /// Semicolon-separated categories (Categories= field).
    pub categories: String,
    /// Short description (Comment= field).
    pub comment: String,
    /// Desktop file basename without .desktop extension (used as app_id).
    pub app_id: String,
    /// Single-char icon for Lens display (derived from categories/name).
    pub icon_char: String,
    /// `X-Yantrik-Surface`: the id of the control surface this app publishes, folded. `None` when
    /// the file declares none, or declares one that is not a surface name.
    pub surface: Option<String>,
    /// `X-Yantrik-Purpose`: what the app is for, in one line. Empty when not said.
    pub purpose: String,
    /// `X-Yantrik-Aliases`: the other names the surface answers to, folded, without the id itself,
    /// without duplicates and without anything that is not a surface name.
    pub aliases: Vec<String>,
    /// `X-Yantrik-Adapter`: the command that provides the surface for this app, when the app
    /// cannot host one. Only kept beside a declared surface: an adapter for nothing is nothing.
    pub adapter: Option<String>,
    /// `MimeType=`: the file types the app says it opens, as the freedesktop `;`-separated list
    /// states them. Files' "Open with" list reads these, and so does the answer for a file type
    /// the shell's own table has no app for (#233). Empty when the file declares none.
    pub mime_types: Vec<String>,
}

/// Built-in Yantrik apps that appear in the app grid alongside system apps.
pub fn builtin_apps() -> Vec<DesktopEntry> {
    vec![
DesktopEntry {
            name: "Files".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "System;FileManager;".into(), comment: "Browse files".into(),
            app_id: "files".into(), icon_char: "F".into(), ..Default::default()
        },
// "Editor", "Images" and "Media Player" are not listed here. The Editor and Images are
        // applications with windows of their own and ship .desktop files; the shell's copies of
        // all three are gone (#253). Sound and video open from Files in mpv's own window.
        DesktopEntry {
            name: "Bond".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Utility;".into(), comment: "Companion bond tracker".into(),
            app_id: "bond".into(), icon_char: "\u{2665}".into(), ..Default::default()
        },
        DesktopEntry {
            name: "Personality".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Utility;".into(), comment: "Companion personality evolution".into(),
            app_id: "personality".into(), icon_char: "\u{2727}".into(), ..Default::default()
        },
        DesktopEntry {
            name: "Memory".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Utility;".into(), comment: "Browse companion memories".into(),
            app_id: "memory".into(), icon_char: "\u{25C8}".into(), ..Default::default()
        },
        DesktopEntry {
            name: "Notifications".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Utility;".into(), comment: "Notification center".into(),
            app_id: "notifications".into(), icon_char: "N".into(), ..Default::default()
        },
        DesktopEntry {
            name: "System".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "System;Monitor;".into(), comment: "System dashboard".into(),
            app_id: "system".into(), icon_char: "\u{25C9}".into(), ..Default::default()
        },
        DesktopEntry {
            name: "Settings".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Settings;".into(), comment: "Yantrik settings".into(),
            app_id: "settings".into(), icon_char: "\u{2699}".into(), ..Default::default()
        },
        DesktopEntry {
            name: "About".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Utility;".into(), comment: "System info".into(),
            app_id: "about".into(), icon_char: "\u{2139}".into(), ..Default::default()
        },
        DesktopEntry {
            name: "Packages".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "System;PackageManager;".into(), comment: "Install and manage packages".into(),
            app_id: "packages".into(), icon_char: "P".into(), ..Default::default()
        },
DesktopEntry {
            name: "Devices".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "System;HardwareSettings;".into(), comment: "Hardware device dashboard".into(),
            app_id: "devices".into(), icon_char: "\u{2699}".into(), ..Default::default()
        },
        DesktopEntry {
            name: "Permissions".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "System;Security;".into(), comment: "File & system permissions".into(),
            app_id: "permissions".into(), icon_char: "\u{2318}".into(), ..Default::default()
        },
        // One pane per agent, with its work inside it: every mind's conversation, each call it
        // made as a card. design/agents-workspace-2026-09-23.md.
        DesktopEntry {
            name: "Agents".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Utility;".into(), comment: "Your agents and what each one is doing".into(),
            app_id: "agents".into(), icon_char: "A".into(), ..Default::default()
        },
        // Every recipe the companion holds, drawn as its stages as it runs.
        // design/desk-and-mind-2026-09-23.md, section 4.
        DesktopEntry {
            name: "Recipes".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "Utility;".into(), comment: "Recipes and how each one is flowing".into(),
            app_id: "recipes".into(), icon_char: "R".into(), ..Default::default()
        },
DesktopEntry {
            name: "Skills".into(), exec: "__builtin__".into(), icon: String::new(),
            categories: "System;".into(), comment: "Install companion skills".into(),
            app_id: "skills".into(), icon_char: "\u{2605}".into(), ..Default::default()
        },
    ]
}

/// System apps that duplicate Yantrik built-ins or are noise in the launcher.
const HIDDEN_APP_IDS: &[&str] = &[
    "xfce4-about",
    "xfce4-settings-manager",
    "thunar",
    "thunar-settings",
    "thunar-bulk-rename",
    "Thunar-bulk-rename",
    "xfce4-file-manager",
    "foot-client",
    "footclient",
    "foot-server",
    "foot",
    "xfce4-terminal",
    "org.freedesktop.Xwayland",
    "mpv",
];

/// Scan all XDG application directories for .desktop files.
/// Returns built-in Yantrik apps first, then system apps sorted by name.
pub fn scan() -> Vec<DesktopEntry> {
    scan_in(&app_dirs())
}

/// [`scan`], over the directories given rather than the session's.
///
/// The directories come first-wins, the way freedesktop orders them: an entry in the person's own
/// directory shadows one of the same name further down the list. Split out so a test can scan a
/// directory of its own without changing the environment every other test reads.
pub fn scan_in(dirs: &[PathBuf]) -> Vec<DesktopEntry> {
    let mut entries = builtin_apps();
    let mut seen_ids: std::collections::HashSet<String> = entries.iter().map(|e| e.app_id.clone()).collect();

    for id in HIDDEN_APP_IDS {
        seen_ids.insert(id.to_string());
    }

    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        let read_dir = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };

        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }

            if let Some(de) = parse_desktop_file(&path) {
                if !seen_ids.contains(&de.app_id) {
                    seen_ids.insert(de.app_id.clone());
                    entries.push(de);
                }
            }
        }
    }

    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    tracing::info!(count = entries.len(), "Scanned .desktop files");
    entries
}

/// Search entries by query. Returns matches sorted by relevance.
pub fn search<'a>(query: &str, entries: &'a [DesktopEntry]) -> Vec<&'a DesktopEntry> {
    let lower = query.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();

    let mut scored: Vec<(&DesktopEntry, u32)> = entries
        .iter()
        .filter_map(|e| {
            let score = match_score(e, &lower, &words);
            if score > 0 { Some((e, score)) } else { None }
        })
        .collect();

    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored.into_iter().take(6).map(|(e, _)| e).collect()
}

fn match_score(entry: &DesktopEntry, query: &str, words: &[&str]) -> u32 {
    let name_lower = entry.name.to_lowercase();
    let app_id_lower = entry.app_id.to_lowercase();
    let exec_lower = entry.exec.to_lowercase();
    let cats_lower = entry.categories.to_lowercase();
    let comment_lower = entry.comment.to_lowercase();

    let mut score = 0u32;

    if name_lower == query {
        score += 100;
    } else if name_lower.starts_with(query) {
        score += 80;
    } else if name_lower.contains(query) {
        score += 60;
    }

    if app_id_lower.contains(query) {
        score += 50;
    }

    let exec_basename = exec_lower.split('/').last().unwrap_or(&exec_lower);
    if exec_basename.contains(query) {
        score += 40;
    }

    if cats_lower.contains(query) {
        score += 20;
    }

    if comment_lower.contains(query) {
        score += 10;
    }

    if words.len() > 1 && score == 0 {
        let all_text = format!("{} {} {} {} {}", name_lower, app_id_lower, exec_lower, cats_lower, comment_lower);
        if words.iter().all(|w| all_text.contains(w)) {
            score += 30;
        }
    }

    score
}

/// Where this OS installs its own .desktop entries.
///
/// The session puts `/opt/yantrik/share` on `XDG_DATA_DIRS`, and that is how the entries are
/// normally found. It is also appended here, last, because this OS's own apps are now found ONLY
/// through their .desktop files — the shell's hardcoded launch table is gone — and a shell started
/// by anything that did not set the session's environment (a developer's restart, an older
/// install script) would otherwise have no apps at all. Last, so any directory the session named
/// still wins.
pub const YANTRIK_APPLICATIONS: &str = "/opt/yantrik/share/applications";

/// The application directories, in the order freedesktop says to search them.
pub fn app_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/share/applications"));
    }
    if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(data_home).join("applications"));
    }

    if let Ok(data_dirs) = std::env::var("XDG_DATA_DIRS") {
        for dir in data_dirs.split(':').filter(|d| !d.trim().is_empty()) {
            dirs.push(PathBuf::from(dir).join("applications"));
        }
    } else {
        dirs.push(PathBuf::from("/usr/share/applications"));
        dirs.push(PathBuf::from("/usr/local/share/applications"));
    }
    dirs.push(PathBuf::from(YANTRIK_APPLICATIONS));

    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs
}

/// A number that changes whenever anything a scan would read changes: an entry added, removed,
/// renamed or rewritten in any of `dirs`, or a directory appearing or going away.
///
/// The scan ran once at startup, then again whenever the launcher opened — so an app installed
/// while the shell ran existed for the launcher and for nobody else: not for `open_app`, not in
/// `describe shell`, not on the socket bus under its aliases. Asking this every few seconds costs
/// one `stat` per entry, and a scan runs only when the answer moves.
///
/// Every entry's name, size and modification time rather than the directory's own mtime, because
/// editing an installed entry in place — adding `X-Yantrik-Surface` to it — changes the file and
/// not the directory, and a file that is not an entry changes the directory and nothing a scan
/// reads.
pub fn fingerprint(dirs: &[PathBuf]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for dir in dirs {
        dir.hash(&mut hasher);
        let Ok(read_dir) = std::fs::read_dir(dir) else {
            false.hash(&mut hasher);
            continue;
        };
        true.hash(&mut hasher);
        let mut files: Vec<(std::ffi::OsString, Option<std::time::SystemTime>, u64)> = read_dir
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("desktop"))
            .map(|e| {
                let meta = e.metadata().ok();
                (
                    e.file_name(),
                    meta.as_ref().and_then(|m| m.modified().ok()),
                    meta.map(|m| m.len()).unwrap_or(0),
                )
            })
            .collect();
        files.sort();
        files.hash(&mut hasher);
    }
    hasher.finish()
}

fn parse_desktop_file(path: &Path) -> Option<DesktopEntry> {
    let content = std::fs::read_to_string(path).ok()?;
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    parse_desktop_text(stem, &content)
}

/// One .desktop file's `[Desktop Entry]` group, as the launcher and the surface catalogue read it.
///
/// `stem` is the file's name without `.desktop`, which is the entry's id. `None` for anything the
/// launcher does not list: not an application, hidden, or without a name or a command.
pub fn parse_desktop_text(stem: &str, content: &str) -> Option<DesktopEntry> {
    let mut name = String::new();
    let mut exec = String::new();
    let mut try_exec = String::new();
    let mut icon = String::new();
    let mut categories = String::new();
    let mut comment = String::new();
    let mut entry_type = String::new();
    let mut no_display = false;
    let mut hidden = false;
    let mut in_desktop_entry = false;
    let mut surface = String::new();
    let mut purpose = String::new();
    let mut aliases = String::new();
    let mut adapter = String::new();
    let mut mime_types = String::new();

    for line in content.lines() {
        let line = line.trim();

        if line.starts_with('[') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }

        if !in_desktop_entry {
            continue;
        }

        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();

            match key {
                "Name" => name = value.to_string(),
                // Kept as written: where the entry put its field code is what a launch needs
                // to know, and that is lost if the codes are removed here (#304).
                "Exec" => exec = value.to_string(),
                "TryExec" => try_exec = value.to_string(),
                "Icon" => icon = value.to_string(),
                "Categories" => categories = value.to_string(),
                "Comment" => comment = value.to_string(),
                "Type" => entry_type = value.to_string(),
                "NoDisplay" => no_display = value == "true",
                "Hidden" => hidden = value == "true",
                KEY_SURFACE => surface = value.to_string(),
                KEY_PURPOSE => purpose = value.to_string(),
                KEY_ALIASES => aliases = value.to_string(),
                KEY_ADAPTER => adapter = value.to_string(),
                "MimeType" => mime_types = value.to_string(),
                _ => {}
            }
        }
    }

    if entry_type != "Application" || no_display || hidden || name.is_empty() || exec.is_empty() {
        return None;
    }

    let app_id = stem.to_string();
    let icon_char = derive_icon_char(&categories, &name);
    let (surface, aliases, adapter) = surface_keys(&app_id, &surface, &aliases, &adapter);

    // The freedesktop list, split the way every `;`-separated key splits: trimmed, empty items
    // dropped, the order the file wrote kept.
    let mime_types = mime_types
        .split(';')
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .collect();

    Some(DesktopEntry {
        name,
        exec,
        try_exec: (!try_exec.is_empty()).then_some(try_exec),
        icon,
        categories,
        comment,
        app_id,
        icon_char,
        surface,
        purpose,
        aliases,
        adapter,
        mime_types,
    })
}

/// The surface keys of one entry, as far as they can be trusted.
///
/// A name that is not a surface name is dropped and said once in the log, with the file it came
/// from — rather than folded into something the author never wrote. An alias equal to the id, or
/// repeated, is dropped without comment: it names nothing new. An adapter with no surface is
/// dropped, because the shell would be starting a process to provide nothing.
fn surface_keys(
    app_id: &str,
    surface: &str,
    aliases: &str,
    adapter: &str,
) -> (Option<String>, Vec<String>, Option<String>) {
    if surface.trim().is_empty() {
        return (None, Vec::new(), None);
    }
    let id = fold_name(surface);
    if !is_surface_name(&id) {
        tracing::warn!(
            entry = app_id,
            surface,
            "{KEY_SURFACE} is not a surface name (lowercase words joined by `-`); the entry is \
             listed as an app without a surface"
        );
        return (None, Vec::new(), None);
    }
    let mut names: Vec<String> = Vec::new();
    for alias in aliases.split(';').map(fold_name).filter(|a| !a.is_empty()) {
        if !is_surface_name(&alias) {
            tracing::warn!(entry = app_id, alias, "{KEY_ALIASES} names something that is not a surface name; left out");
            continue;
        }
        if alias != id && !names.contains(&alias) {
            names.push(alias);
        }
    }
    let adapter = Some(adapter.trim()).filter(|a| !a.is_empty()).map(str::to_string);
    (Some(id), names, adapter)
}

/// The argv one desktop entry's `Exec=` line stands for, read the way the Desktop Entry spec
/// says: arguments split on unquoted whitespace, `"`-quoted sections kept as one argument with
/// `\"` and `\\` unescaped, a backslash outside quotes escaping the next character, and the
/// field codes answered where they stand (#304).
///
/// `file` is what `%f`, `%F`, `%u` and `%U` expand to — the one file being opened. A launch
/// with no file passes `None` and those codes are removed, as the spec says for them; a code
/// in the middle of the line puts the file there (flatpak's `--file-forwarding … @@u %U @@`
/// needs exactly that), and only a line with no file code at all gets the file appended as
/// the last argument. The deprecated codes (`%d %D %n %N %i %c %k
/// %v %m`) are dropped, `%%` is a literal percent, and a field code inside a quoted argument
/// is removed rather than expanded — also per the spec.
///
/// The result is argv, never a shell command line: no launcher should join it back into a
/// string for a shell to re-split.
pub fn exec_argv(exec: &str, file: Option<&str>) -> Vec<String> {
    const FILE_CODES: &[char] = &['f', 'F', 'u', 'U'];
    const DEAD_CODES: &str = "dDnNickvm";

    let mut argv: Vec<String> = Vec::new();
    let mut arg = String::new();
    // Whether anything besides an empty-expanding file code went into `arg`: an argument that
    // was only `%f` disappears with no file, while a deliberately quoted `""` stays the empty
    // argument the entry asked for.
    let mut literal = false;
    // Whether the line asked for the file anywhere a code counts (outside quotes): only a
    // line that did not gets the file appended.
    let mut saw_file_code = false;
    let mut chars = exec.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' => {
                if literal || !arg.is_empty() {
                    argv.push(std::mem::take(&mut arg));
                }
                literal = false;
            }
            '"' => {
                literal = true;
                loop {
                    match chars.next() {
                        // An unterminated quote swallows the rest of the line, as a shell's
                        // would not, but a broken entry should not lose its last argument.
                        None | Some('"') => break,
                        Some('\\') => match chars.peek() {
                            // Inside quotes only `"` and `\` are escapes; a backslash before
                            // anything else stays the backslash the entry wrote.
                            Some('"') | Some('\\') => arg.push(chars.next().unwrap()),
                            _ => arg.push('\\'),
                        },
                        Some('%') => match chars.peek() {
                            // The spec: a field code inside a quoted argument is removed,
                            // never expanded.
                            Some(c) if FILE_CODES.contains(c) || DEAD_CODES.contains(*c) => {
                                chars.next();
                            }
                            _ => arg.push('%'),
                        },
                        Some(c) => arg.push(c),
                    }
                }
            }
            '\\' => {
                // The general escape rule: outside quotes a backslash makes the next
                // character literal, whatever it is.
                literal = true;
                if let Some(next) = chars.next() {
                    arg.push(next);
                }
            }
            '%' => match chars.next() {
                Some('%') => {
                    arg.push('%');
                    literal = true;
                }
                Some(code) if FILE_CODES.contains(&code) => {
                    saw_file_code = true;
                    if let Some(file) = file {
                        arg.push_str(file);
                    }
                }
                Some(code) if DEAD_CODES.contains(code) => {}
                // An unknown code is not a code: the percent and its character stay as the
                // entry wrote them, as `strip_field_codes` left them before.
                Some(other) => {
                    arg.push('%');
                    arg.push(other);
                    literal = true;
                }
                None => {
                    arg.push('%');
                    literal = true;
                }
            },
            _ => {
                literal = true;
                arg.push(c);
            }
        }
    }
    if literal || !arg.is_empty() {
        argv.push(arg);
    }

    // Only a line with no file code at all gets the file appended, as the spec's rule for an
    // application that takes a file but wrote no code.
    if let Some(file) = file {
        if !saw_file_code {
            argv.push(file.to_string());
        }
    }
    argv
}

fn derive_icon_char(categories: &str, name: &str) -> String {
    let cats = categories.to_lowercase();
    let name_lower = name.to_lowercase();

    if cats.contains("terminal") || cats.contains("system") {
        ">_".to_string()
    } else if cats.contains("webbrowser") || cats.contains("browser") {
        if name_lower.contains("firefox") {
            "\u{2740}".to_string()
        } else {
            "W".to_string()
        }
    } else if cats.contains("filemanager") || cats.contains("filesystem") {
        "F".to_string()
    } else if cats.contains("texteditor") || cats.contains("editor") {
        "E".to_string()
    } else if cats.contains("game") {
        "G".to_string()
    } else if cats.contains("audio") || cats.contains("music") || cats.contains("player") {
        "M".to_string()
    } else if cats.contains("video") {
        "V".to_string()
    } else if cats.contains("graphics") || cats.contains("image") {
        "I".to_string()
    } else if cats.contains("office") || cats.contains("document") {
        "D".to_string()
    } else if cats.contains("network") || cats.contains("email") || cats.contains("chat") {
        "N".to_string()
    } else if cats.contains("settings") || cats.contains("preferences") {
        "*".to_string()
    } else if cats.contains("development") || cats.contains("ide") {
        "<>".to_string()
    } else {
        name.chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".to_string())
    }
}

#[cfg(test)]
mod name_collision_tests {
    use super::builtin_apps;

    /// The names of the applications this OS ships, from APP_NAMES in yantrik-ui.
    ///
    /// Duplicated here rather than imported because yantrik-shell-core sits BELOW yantrik-ui
    /// and must not depend on it. The list is small and changes rarely; the test failing with
    /// a clear message is worth more than the coupling would be.
    const SHIPPED_APP_NAMES: &[&str] = &[
        "Calendar", "Containers", "yDoc", "Downloads", "Email", "Images", "Music", "Network",
        "Notes", "yPresent", "Snippets", "ySheets", "System Monitor", "Terminal", "Editor",
        "Weather",
    ];

    /// No shell screen may share a name with an application.
    ///
    /// The launcher lists both, one after the other, so a collision reads as the same thing
    /// listed twice — and the two behave differently: one opens a window you can alt-tab to,
    /// the other changes the shell's screen. "Editor" was exactly this, and it took a
    /// photograph of the launcher to notice.
    #[test]
    fn no_builtin_screen_shares_a_name_with_an_app() {
        let clashes: Vec<String> = builtin_apps()
            .iter()
            .filter(|e| SHIPPED_APP_NAMES.contains(&e.name.as_str()))
            .map(|e| format!("builtin `{}` is also the name of a shipped app", e.name))
            .collect();

        assert!(
            clashes.is_empty(),
            "{}\n\nThe launcher shows both, side by side, and nothing tells them apart. \
             Either rename the screen or drop it from builtin_apps() and let the application \
             be the one that answers to the name.",
            clashes.join("\n")
        );
    }
}

#[cfg(test)]
mod surface_key_tests {
    use super::*;

    /// A directory of its own under the system temp dir, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "yantrik-apps-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const LIBREOFFICE: &str = "[Desktop Entry]
Type=Application
Name=LibreOffice
Exec=libreoffice %U
X-Yantrik-Surface=libreoffice
X-Yantrik-Purpose=Documents, spreadsheets and slides: open, read, edit and export them
X-Yantrik-Aliases=writer;Calc; impress ;;libreoffice;writer
X-Yantrik-Adapter=/usr/lib/yantrik/adapters/libreoffice --uno

[Desktop Action new]
X-Yantrik-Surface=not-this-one
";

    /// The four keys the design names, read the way the design writes them.
    #[test]
    fn the_four_keys_are_read() {
        let entry = parse_desktop_text("libreoffice-startcenter", LIBREOFFICE).expect("an app");
        assert_eq!(entry.surface.as_deref(), Some("libreoffice"));
        assert_eq!(entry.purpose, "Documents, spreadsheets and slides: open, read, edit and export them");
        // Folded, deduplicated, the id itself and empty items left out, order kept.
        assert_eq!(entry.aliases, vec!["writer", "calc", "impress"]);
        assert_eq!(entry.adapter.as_deref(), Some("/usr/lib/yantrik/adapters/libreoffice --uno"));
        // A key in another group is not the entry's.
        assert_ne!(entry.surface.as_deref(), Some("not-this-one"));
        // And the ordinary keys are still what they were — Exec kept as the file wrote it,
        // field code included: the launcher answers the code when it runs the command (#304).
        assert_eq!(entry.exec, "libreoffice %U");
        assert_eq!(entry.app_id, "libreoffice-startcenter");
    }

    /// An app that says nothing is an app without a surface, exactly as before.
    #[test]
    fn an_entry_without_the_keys_declares_nothing() {
        let entry = parse_desktop_text("vim", "[Desktop Entry]\nType=Application\nName=Vim\nExec=vim %F\n")
            .expect("an app");
        assert_eq!(entry.surface, None);
        assert!(entry.purpose.is_empty() && entry.aliases.is_empty() && entry.adapter.is_none());
    }

    /// `TryExec` is read as written: the program the entry is FOR, which for an adapter's entry
    /// is the app it wraps and not the wrapper its `Exec` runs (#214). Whether the program is on
    /// this machine is asked by whoever lists entries (`yantrik-ui`'s `entry_is_launchable`); the
    /// parser only carries the name.
    #[test]
    fn try_exec_names_the_program_the_entry_is_for() {
        let entry = parse_desktop_text(
            "yantrik-libreoffice",
            "[Desktop Entry]\nType=Application\nName=LibreOffice\n\
             Exec=yantrik-libreoffice %U\nTryExec=soffice\n",
        )
        .expect("an app");
        assert_eq!(entry.exec, "yantrik-libreoffice %U");
        assert_eq!(entry.try_exec.as_deref(), Some("soffice"));
        // A file that names none carries none, and entries are listed exactly as before.
        let plain =
            parse_desktop_text("vim", "[Desktop Entry]\nType=Application\nName=Vim\nExec=vim %F\n")
                .expect("an app");
        assert_eq!(plain.try_exec, None);
        // An empty value names nothing.
        let empty = parse_desktop_text(
            "x",
            "[Desktop Entry]\nType=Application\nName=X\nExec=x\nTryExec=\n",
        )
        .expect("an app");
        assert_eq!(empty.try_exec, None);
        // Like every other key, it is the entry's group alone that is read.
        let other_group = parse_desktop_text(
            "y",
            "[Desktop Entry]\nType=Application\nName=Y\nExec=y\n\n\
             [Desktop Action new]\nTryExec=not-this-one\n",
        )
        .expect("an app");
        assert_eq!(other_group.try_exec, None);
    }

    /// A surface id is folded the way every client folds a name, and one that is not a surface
    /// name at all declares nothing — its aliases and adapter with it.
    #[test]
    fn a_name_that_is_not_a_surface_name_is_refused_not_rewritten() {
        let entry = |surface: &str, aliases: &str| {
            parse_desktop_text(
                "x",
                &format!(
                    "[Desktop Entry]\nType=Application\nName=X\nExec=x\nX-Yantrik-Surface={surface}\n\
                     X-Yantrik-Aliases={aliases}\nX-Yantrik-Adapter=/bin/x-adapter\n"
                ),
            )
            .unwrap()
        };
        assert_eq!(entry("Download_Manager", "").surface.as_deref(), Some("download-manager"));
        for bad in ["../../etc/passwd", "org.libreoffice.Writer", "a/b", "-x", "x--y", "caf\u{e9}"] {
            let e = entry(bad, "fine");
            assert_eq!(e.surface, None, "`{bad}` must not become a socket name");
            assert!(e.aliases.is_empty() && e.adapter.is_none(), "{bad}");
        }
        // One bad alias is dropped; the good ones stay.
        assert_eq!(entry("thing", "ok;../up;also ok;a.b").aliases, vec!["ok", "also-ok"]);
        // An adapter beside no surface is nothing to start.
        let lone = parse_desktop_text(
            "y",
            "[Desktop Entry]\nType=Application\nName=Y\nExec=y\nX-Yantrik-Adapter=/bin/y-adapter\n",
        )
        .unwrap();
        assert_eq!(lone.adapter, None);
    }

    #[test]
    fn surface_names_are_the_protocols() {
        for good in ["notes", "download-manager", "image-viewer", "a1", "x-2-y"] {
            assert!(is_surface_name(good), "{good}");
        }
        for bad in ["", "Notes", "notes.sock", "a_b", "a b", "-a", "a-", "a--b", ".", ".."] {
            assert!(!is_surface_name(bad), "{bad:?}");
        }
        assert_eq!(fold_name("  Container Manager "), "container-manager");
        assert_eq!(fold_name("container_manager"), "container-manager");
    }

    /// Scanning a directory finds a declared surface, and a directory the person owns shadows the
    /// same entry further down the list.
    #[test]
    fn a_scan_of_given_directories_reads_the_keys_and_keeps_the_first() {
        let mine = Scratch::new("mine");
        let system = Scratch::new("system");
        std::fs::write(system.0.join("libreoffice-startcenter.desktop"), LIBREOFFICE).unwrap();
        std::fs::write(
            mine.0.join("libreoffice-startcenter.desktop"),
            LIBREOFFICE.replace("X-Yantrik-Aliases=writer;Calc; impress ;;libreoffice;writer", "X-Yantrik-Aliases=lo"),
        )
        .unwrap();
        let found = scan_in(&[mine.0.clone(), system.0.clone()]);
        let lo: Vec<&DesktopEntry> = found.iter().filter(|e| e.app_id == "libreoffice-startcenter").collect();
        assert_eq!(lo.len(), 1, "one entry per id");
        assert_eq!(lo[0].aliases, vec!["lo"], "the person's own copy wins");
        // The shell's own screens are still there, and declare nothing.
        assert!(found.iter().any(|e| e.app_id == "files" && e.surface.is_none()));
    }

    /// The fingerprint moves when an entry is added, rewritten in place, or removed — and does not
    /// move when nothing changed, which is what keeps the shell from rescanning every tick.
    #[test]
    fn the_fingerprint_moves_exactly_when_the_directories_do() {
        let dir = Scratch::new("fp");
        let missing = dir.0.join("not-there");
        let dirs = vec![dir.0.clone(), missing.clone()];
        let before = fingerprint(&dirs);
        assert_eq!(fingerprint(&dirs), before, "nothing changed, nothing moved");

        let file = dir.0.join("thing.desktop");
        std::fs::write(&file, "[Desktop Entry]\nType=Application\nName=Thing\nExec=thing\n").unwrap();
        let added = fingerprint(&dirs);
        assert_ne!(added, before, "an entry was added");

        // Rewritten in place, as an update that adds the surface keys to an existing entry does.
        std::fs::write(
            &file,
            "[Desktop Entry]\nType=Application\nName=Thing\nExec=thing\nX-Yantrik-Surface=thing\n",
        )
        .unwrap();
        assert_ne!(fingerprint(&dirs), added, "an entry was rewritten");

        std::fs::write(dir.0.join("notes.txt"), "not an entry").unwrap();
        let with_other = fingerprint(&dirs);
        std::fs::remove_file(dir.0.join("notes.txt")).unwrap();
        assert_eq!(fingerprint(&dirs), with_other, "a file that is not an entry is not a change");

        std::fs::create_dir_all(&missing).unwrap();
        assert_ne!(fingerprint(&dirs), with_other, "a directory appeared");
    }

    /// Our own directory is searched even when the session forgot to name it, and after
    /// everything the session did name.
    #[test]
    fn this_oss_own_entries_are_always_searched_last() {
        let dirs = app_dirs();
        assert_eq!(dirs.last().map(PathBuf::as_path), Some(Path::new(YANTRIK_APPLICATIONS)));
        let unique: std::collections::HashSet<_> = dirs.iter().collect();
        assert_eq!(unique.len(), dirs.len(), "no directory is scanned twice");
    }
}

#[cfg(test)]
mod mime_type_tests {
    use super::*;

    /// `MimeType=` is read as the freedesktop list it is: trimmed, empties dropped, order kept,
    /// and only from the entry's own group. Files' "Open with" offers an app exactly for the
    /// types the app declared (#233).
    #[test]
    fn mime_types_are_read_as_a_list() {
        let entry = parse_desktop_text(
            "chromium",
            "[Desktop Entry]\nType=Application\nName=Chromium\nExec=chromium %U\n\
             MimeType=text/html;text/xml;; application/xhtml+xml ;\n",
        )
        .expect("an app");
        assert_eq!(entry.mime_types, ["text/html", "text/xml", "application/xhtml+xml"]);

        // A key in another group is not the entry's, like every other key.
        let other_group = parse_desktop_text(
            "y",
            "[Desktop Entry]\nType=Application\nName=Y\nExec=y\n\n\
             [Desktop Action new]\nMimeType=text/plain\n",
        )
        .expect("an app");
        assert!(other_group.mime_types.is_empty());

        // An entry that declares none opens nothing by type, and an empty value declares none.
        let plain =
            parse_desktop_text("vim", "[Desktop Entry]\nType=Application\nName=Vim\nExec=vim %F\n")
                .expect("an app");
        assert!(plain.mime_types.is_empty());
        let empty = parse_desktop_text(
            "x",
            "[Desktop Entry]\nType=Application\nName=X\nExec=x\nMimeType=;;\n",
        )
        .expect("an app");
        assert!(empty.mime_types.is_empty());
    }
}

#[cfg(test)]
mod exec_argv_tests {
    use super::exec_argv;

    /// The table #304 lists: plain, quoted path, `%f` mid-line, flatpak's `@@u %U @@`, and no
    /// field code at all.
    #[test]
    fn the_file_goes_where_the_entry_put_its_field_code() {
        let file = "/home/me/My Documents/report.odt";
        // No field code: the file is appended, as it always was.
        assert_eq!(exec_argv("gedit", Some(file)), ["gedit", file]);
        assert_eq!(exec_argv("app --flag", Some(file)), ["app", "--flag", file]);
        // A quoted program path stays one argument, and `%f` mid-line is where the file goes —
        // not after everything else.
        assert_eq!(
            exec_argv("\"/opt/My App/bin/app\" --flag %f", Some(file)),
            ["/opt/My App/bin/app", "--flag", file]
        );
        assert_eq!(
            exec_argv("app --open %f --new-window", Some(file)),
            ["app", "--open", file, "--new-window"]
        );
        // flatpak's exported entries: the file belongs between `@@u` and `@@`, which appending
        // last put after them, where flatpak reads it as a file name of its own.
        assert_eq!(
            exec_argv("/usr/bin/flatpak run --file-forwarding org.x.Y @@u %U @@", Some(file)),
            ["/usr/bin/flatpak", "run", "--file-forwarding", "org.x.Y", "@@u", file, "@@"]
        );
        // All four file codes take the one file, wherever in the line they stand.
        assert_eq!(exec_argv("app %F", Some(file)), ["app", file]);
        assert_eq!(exec_argv("app %u", Some(file)), ["app", file]);
        // A code inside quotes is removed, not expanded — and does not stop the append: the
        // line still asked for no file.
        assert_eq!(exec_argv("app \"%f\"", Some(file)), ["app", "", file]);
    }

    /// A launch with no file removes the file codes, as the spec says, and the deprecated
    /// codes go whatever the launch carries.
    #[test]
    fn codes_expand_to_nothing_when_there_is_no_file() {
        assert_eq!(exec_argv("libreoffice %U", None), ["libreoffice"]);
        assert_eq!(exec_argv("app %f", None), ["app"]);
        // An argument that was only the code disappears; one that carried more keeps its rest.
        assert_eq!(exec_argv("app --file=%f", None), ["app", "--file="]);
        assert_eq!(exec_argv("app %d %D %n %N %i %c %k %v %m end", None), ["app", "end"]);
        // `%%` is a literal percent, and a `%` before anything else stays as written.
        assert_eq!(exec_argv("app 100%% %z", None), ["app", "100%", "%z"]);
    }

    /// Quoting and escapes, per the spec's rules rather than a shell's.
    #[test]
    fn quoting_and_escapes_follow_the_spec() {
        // A quoted section can start mid-argument.
        assert_eq!(exec_argv("app\"two words\"tail", None), ["apptwo wordstail"]);
        // Inside quotes `\"` and `\\` unescape; a backslash before anything else stays.
        assert_eq!(exec_argv("app \"a\\\"b\\\\c\\d\"", None), ["app", "a\"b\\c\\d"]);
        // Outside quotes a backslash makes the next character literal, including a space.
        assert_eq!(exec_argv("app a\\ b \\<x\\>", None), ["app", "a b", "<x>"]);
        // A quoted empty string is the empty argument the entry asked for.
        assert_eq!(exec_argv("app \"\" x", None), ["app", "", "x"]);
        // An unterminated quote keeps the rest of the line as one argument.
        assert_eq!(exec_argv("app \"two three", None), ["app", "two three"]);
    }
}
