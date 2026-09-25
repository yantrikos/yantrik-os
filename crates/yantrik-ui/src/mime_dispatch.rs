//! MIME dispatch — which app window opens a file.
//!
//! Classifies files by extension and returns a `FileAction` telling the
//! caller which app opens them. The person's own defaults, from the
//! `[Default Applications]` section of their `mimeapps.list`, come first:
//! where they name an app the shell knows how to open, that app opens the
//! file, whatever the built-in table would have said (#233). A default that
//! names any other installed app is followed too, as `DesktopApp`, because
//! "Always use this app" in Files writes exactly such a line and a promise
//! the shell then ignores is worse than no promise. For a type the built-in
//! table has no app for, an installed `.desktop` entry that declares the type
//! in its `MimeType=` line opens it.
//!
//! The same module feeds Files' "Open with" list ([`open_with`]) and the
//! writing half of "Always use this app" ([`with_default`]), so the list, the
//! default it writes and the double-click that follows the default cannot
//! disagree.

use std::path::{Path, PathBuf};

use crate::apps::DesktopEntry;

/// What to do when a file is opened.
#[derive(Debug, Clone, PartialEq)]
pub enum FileAction {
    /// Open in the Images app, `yantrik-image-viewer`.
    ImageViewer,
    /// Open in the Editor app, `yantrik-text-editor`.
    TextEditor,
    /// Play in mpv's own window, sound and video alike. Sound used to play in a screen of the
    /// shell's own, which was not a window and could not be moved, closed or put behind
    /// anything (#253).
    MediaPlayer,
    /// Open in the machine's web browser, whichever `wire::dock::find_browser` finds.
    Browser,
    /// Open in an installed app of the person's own choosing, named by its desktop entry id
    /// (`yantrik-libreoffice.desktop`). The entry's `Exec` line is what runs, with the file as
    /// its last argument — the `wire::open_with` launcher reads the catalogue for it (#233).
    DesktopApp(String),
}

/// The app a `FileAction` names, in the words the control surface answers `files_open` with:
/// the shell's own routes by the id `open_app` opens them under, and a desktop entry by its id
/// without the `.desktop`. One rule picks the app, so one rule names it (#233).
pub fn app_name(action: &FileAction) -> String {
    match action {
        FileAction::ImageViewer => "images".to_string(),
        FileAction::TextEditor => "editor".to_string(),
        FileAction::MediaPlayer => "media-player".to_string(),
        FileAction::Browser => "browser".to_string(),
        FileAction::DesktopApp(id) => id.strip_suffix(".desktop").unwrap_or(id).to_string(),
    }
}

/// The person's default apps, as the `[Default Applications]` section of a
/// `mimeapps.list` states them: MIME type, and the desktop entry id that
/// opens it.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct MimeDefaults(Vec<(String, String)>);

impl MimeDefaults {
    /// Parse the contents of a `mimeapps.list`. Only `[Default Applications]`
    /// counts: `[Added Associations]` lists apps that CAN open a type, not the
    /// one that should. Each value is a `;`-separated list of desktop ids in
    /// order of preference; the first is the default.
    pub fn parse(contents: &str) -> Self {
        let mut defaults = Vec::new();
        let mut in_defaults = false;
        for line in contents.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_defaults = line == "[Default Applications]";
                continue;
            }
            if !in_defaults || line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((mime, value)) = line.split_once('=') else { continue };
            if let Some(id) = value.split(';').map(str::trim).find(|v| !v.is_empty()) {
                defaults.push((mime.trim().to_string(), id.to_string()));
            }
        }
        MimeDefaults(defaults)
    }

    /// Read the person's own defaults, `$XDG_CONFIG_HOME/mimeapps.list` or
    /// `~/.config/mimeapps.list`. A machine where they have never set one has
    /// no file and no defaults; that is ordinary, not an error.
    pub fn read() -> Self {
        std::fs::read_to_string(Self::path())
            .map(|contents| Self::parse(&contents))
            .unwrap_or_default()
    }

    fn path() -> PathBuf {
        if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
            if !dir.trim().is_empty() {
                return PathBuf::from(dir).join("mimeapps.list");
            }
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        PathBuf::from(home).join(".config/mimeapps.list")
    }

    /// The desktop id the person set as the default for this MIME type, if any.
    fn default_for(&self, mime: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(m, _)| m.eq_ignore_ascii_case(mime))
            .map(|(_, id)| id.as_str())
    }

    /// "Always use this app": make `desktop_id` the default for `mime` in the person's own
    /// `mimeapps.list`, the file [`MimeDefaults::read`] reads, so the next double-click follows
    /// it. A file that does not exist yet is created with just this default.
    ///
    /// The file is the person's, and other tools write it too, so it is never lost to this: a
    /// file that exists but cannot be read is an error, not an empty file to write over, and the
    /// new contents go to a temporary file beside it that is renamed into place, so a crash
    /// mid-write leaves the old file whole.
    pub fn set_default(mime: &str, desktop_id: &str) -> std::io::Result<()> {
        let path = Self::path();
        let contents = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let staged = path.with_extension("list.yantrik-tmp");
        std::fs::write(&staged, with_default(&contents, mime, desktop_id))?;
        std::fs::rename(&staged, &path)
    }
}

/// The contents of a `mimeapps.list` with `mime=desktop_id;` set in its `[Default Applications]`
/// section: an existing line for the type replaced where it stands, a new line added under the
/// section head, and the section itself appended when the file has none. Everything else in the
/// file — comments, other types, `[Added Associations]` — is carried over untouched, because the
/// file is the person's and other tools write it too. The pure half of
/// [`MimeDefaults::set_default`], split out so the tests can drive it (#233).
pub fn with_default(contents: &str, mime: &str, desktop_id: &str) -> String {
    let new_line = format!("{mime}={desktop_id};");
    let mut lines: Vec<String> = contents.lines().map(str::to_string).collect();

    let mut section: Option<usize> = None;
    let mut existing: Option<usize> = None;
    let mut in_defaults = false;
    for (i, raw) in lines.iter().enumerate() {
        let trimmed = raw.trim();
        if trimmed.starts_with('[') {
            in_defaults = trimmed == "[Default Applications]";
            if in_defaults && section.is_none() {
                section = Some(i);
            }
            continue;
        }
        if in_defaults && existing.is_none() && !trimmed.is_empty() && !trimmed.starts_with('#') {
            if let Some((key, _)) = trimmed.split_once('=') {
                if key.trim().eq_ignore_ascii_case(mime) {
                    existing = Some(i);
                }
            }
        }
    }

    match (section, existing) {
        (_, Some(i)) => lines[i] = new_line,
        (Some(i), None) => lines.insert(i + 1, new_line),
        (None, None) => {
            if lines.iter().any(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[Default Applications]".to_string());
            lines.push(new_line);
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Classify a filename into a `FileAction`, following the person's own
/// defaults where they have set any.
pub fn classify(filename: &str) -> FileAction {
    classify_with(
        filename,
        &MimeDefaults::read(),
        &crate::apps::Catalogue::shared().get(),
    )
}

/// The pure half of [`classify`]: the answer given the filename, the parsed
/// contents of the person's `mimeapps.list` and the installed apps' desktop
/// entries. All the deciding is here so the tests can drive it; the only I/O
/// is `MimeDefaults::read` and the catalogue snapshot.
pub fn classify_with(
    filename: &str,
    defaults: &MimeDefaults,
    installed: &[DesktopEntry],
) -> FileAction {
    let mime = mime_for(filename);

    if let Some(mime) = mime {
        if let Some(id) = defaults.default_for(mime) {
            // An app the shell has a route of its own to opens through that route:
            // whichever browser the person chose, the shell opens the browser this
            // machine has.
            if let Some(action) = action_for_desktop(id) {
                return action;
            }
            // Any other default is followed as long as its entry is installed —
            // "Always use this app" writes exactly such a line, and a promise the
            // shell then ignores is worse than no promise. An entry that is gone
            // cannot be launched, so for one the built-in table answers instead.
            if installed
                .iter()
                .any(|e| format!("{}.desktop", e.app_id).eq_ignore_ascii_case(id))
            {
                return FileAction::DesktopApp(id.trim().to_string());
            }
        }
    }

    // The shell's own table, for the types it knows an app for.
    if let Some(action) = builtin(filename) {
        return action;
    }

    // A type the table has no app for — an .odt, say — opens in the first installed
    // entry that declares it in its `MimeType=` line (#233).
    if let Some(mime) = mime {
        if let Some(entry) = installed
            .iter()
            .find(|e| e.mime_types.iter().any(|m| m.eq_ignore_ascii_case(mime)))
        {
            return FileAction::DesktopApp(format!("{}.desktop", entry.app_id));
        }
    }

    // Unknown — try text editor as fallback for small files.
    FileAction::TextEditor
}

/// The desktop entry ids the shell knows how to open, and what each means.
///
/// The browser ids are the distributions' spellings of the same browsers
/// `wire::dock::find_browser` looks for. Whichever of them the person chose,
/// the shell opens the browser this machine has, with the file as its
/// argument — the Browser pin's own behaviour, not a second launcher.
/// `wire::open_with` reads this too, so an "Open with" row naming one of these
/// ids launches the same way a double-click does (#233).
pub fn action_for_desktop(id: &str) -> Option<FileAction> {
    match id.trim().to_lowercase().as_str() {
        "yantrik-text-editor.desktop" => Some(FileAction::TextEditor),
        "yantrik-image-viewer.desktop" => Some(FileAction::ImageViewer),
        // The Music app is not built yet (see `wire::dock`'s shelf), so a default naming it
        // plays where it would have played before it was chosen.
        "yantrik-music-player.desktop" | "mpv.desktop" => Some(FileAction::MediaPlayer),
        "firefox.desktop"
        | "firefox-esr.desktop"
        | "librewolf.desktop"
        | "org.mozilla.firefox.desktop"
        | "chromium.desktop"
        | "chromium-browser.desktop"
        | "google-chrome.desktop"
        | "google-chrome-stable.desktop"
        | "com.google.chrome.desktop"
        | "epiphany.desktop"
        | "epiphany-browser.desktop"
        | "org.gnome.epiphany.desktop"
        | "brave-browser.desktop"
        | "brave.desktop" => Some(FileAction::Browser),
        _ => None,
    }
}

/// The freedesktop MIME type of a filename, by its extension — the key the
/// person's `mimeapps.list` is written in. An extension the shell has no type
/// for has no default to look up, and the built-in table answers it.
/// `wire::files` reads it for "Always use this app" (#233).
pub fn mime_for(filename: &str) -> Option<&'static str> {
    let lower = filename.to_lowercase();
    MIME_BY_EXT
        .iter()
        .find(|(ext, _)| lower.ends_with(*ext))
        .map(|(_, mime)| *mime)
}

/// Extension to MIME type for the extensions the shell knows a type for. The
/// two lists are kept in step by hand: an extension added to `builtin` with no
/// row here still opens, it just cannot be given a default of its own. The
/// office rows at the end are types `builtin` deliberately has no app for —
/// they exist so the installed apps' `MimeType=` lines can answer them (#233).
const MIME_BY_EXT: &[(&str, &str)] = &[
    (".html", "text/html"),
    (".htm", "text/html"),
    (".svg", "image/svg+xml"),
    (".pdf", "application/pdf"),
    (".jpg", "image/jpeg"),
    (".jpeg", "image/jpeg"),
    (".png", "image/png"),
    (".gif", "image/gif"),
    (".bmp", "image/bmp"),
    (".webp", "image/webp"),
    (".ico", "image/vnd.microsoft.icon"),
    (".tiff", "image/tiff"),
    (".tif", "image/tiff"),
    (".mp3", "audio/mpeg"),
    (".ogg", "audio/ogg"),
    (".flac", "audio/flac"),
    (".wav", "audio/x-wav"),
    (".m4a", "audio/mp4"),
    (".aac", "audio/aac"),
    (".opus", "audio/opus"),
    (".wma", "audio/x-ms-wma"),
    (".mp4", "video/mp4"),
    (".mkv", "video/x-matroska"),
    (".avi", "video/x-msvideo"),
    (".webm", "video/webm"),
    (".mov", "video/quicktime"),
    (".wmv", "video/x-ms-wmv"),
    (".flv", "video/x-flv"),
    (".txt", "text/plain"),
    (".md", "text/markdown"),
    (".csv", "text/csv"),
    (".json", "application/json"),
    (".yaml", "application/yaml"),
    (".yml", "application/yaml"),
    (".xml", "application/xml"),
    (".css", "text/css"),
    (".js", "text/javascript"),
    (".py", "text/x-python"),
    (".sh", "application/x-shellscript"),
    (".c", "text/x-csrc"),
    (".h", "text/x-chdr"),
    (".cpp", "text/x-c++src"),
    (".go", "text/x-go"),
    (".java", "text/x-java"),
    (".rb", "text/x-ruby"),
    (".rs", "text/rust"),
    // Office documents. The built-in table has no app of the shell's own for these —
    // the rows are here so the type has a name an installed app's `MimeType=` line
    // and a `mimeapps.list` default can be matched against (#233).
    (".odt", "application/vnd.oasis.opendocument.text"),
    (".ods", "application/vnd.oasis.opendocument.spreadsheet"),
    (".odp", "application/vnd.oasis.opendocument.presentation"),
    (".doc", "application/msword"),
    (".xls", "application/vnd.ms-excel"),
    (".ppt", "application/vnd.ms-powerpoint"),
    (".docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
    (".xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
    (".pptx", "application/vnd.openxmlformats-officedocument.presentationml.presentation"),
    (".rtf", "application/rtf"),
];

/// The shell's own associations, for a file the person has set no default for.
/// `None` means the table has no opinion — the caller then asks the installed
/// apps' `MimeType=` lines before falling back to the Text Editor (#233).
fn builtin(filename: &str) -> Option<FileAction> {
    let lower = filename.to_lowercase();

    // Web pages, their vector graphics and PDFs — the Browser. An .html in the
    // Text Editor put tags on the screen where a person wanted a page (#233),
    // and the browsers this desktop installs render all three.
    if matches_ext(&lower, &[".html", ".htm", ".svg", ".pdf"]) {
        return Some(FileAction::Browser);
    }

    // Images
    if matches_ext(&lower, &[".jpg", ".jpeg", ".png", ".gif", ".bmp", ".webp", ".ico", ".tiff", ".tif"]) {
        return Some(FileAction::ImageViewer);
    }

    // Sound and video, both in mpv's window
    if matches_ext(&lower, &[
        ".mp3", ".ogg", ".flac", ".wav", ".m4a", ".aac", ".opus", ".wma",
        ".mp4", ".mkv", ".avi", ".webm", ".mov", ".wmv", ".flv",
    ]) {
        return Some(FileAction::MediaPlayer);
    }

    // Text / code / config — open in editor
    if matches_ext(&lower, &[
        ".txt", ".md", ".rs", ".py", ".sh", ".bash", ".zsh",
        ".js", ".ts", ".jsx", ".tsx", ".css", ".scss",
        ".json", ".yaml", ".yml", ".toml", ".xml", ".csv",
        ".c", ".h", ".cpp", ".hpp", ".go", ".java", ".rb",
        ".lua", ".vim", ".conf", ".cfg", ".ini", ".env",
        ".log", ".diff", ".patch", ".sql", ".dockerfile",
        ".makefile", ".cmake",
    ]) {
        return Some(FileAction::TextEditor);
    }

    // Files without extension or named like Makefile, Dockerfile, etc.
    let name_lower = Path::new(&lower)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if matches!(name_lower,
        "makefile" | "dockerfile" | "readme" | "license" | "changelog"
        | "todo" | "authors" | "contributing" | ".gitignore" | ".gitattributes"
        | ".editorconfig" | ".env" | ".env.local"
    ) {
        return Some(FileAction::TextEditor);
    }

    // Unknown: an office document, say. The installed apps get asked first.
    None
}

/// One row of Files' "Open with" list: an app that can open the file, by its
/// desktop entry id, with a name to show and whether it is the default — the
/// app a plain double-click uses (#233).
#[derive(Debug, Clone, PartialEq)]
pub struct OpenWith {
    /// Desktop entry id, e.g. `chromium.desktop`.
    pub id: String,
    /// Display name, e.g. `Chromium Web Browser`.
    pub name: String,
    /// Whether this is what a double-click currently does.
    pub is_default: bool,
}

/// Build the "Open with" list for a filename: the current default first, then
/// every installed app whose `MimeType=` line declares the file's type, then
/// the Text Editor as a last resort for anything readable. `browser` is the
/// desktop id of the machine's browser as `wire::dock::find_browser` names it,
/// `None` when there is none. Pure, like the rest of the deciding here (#233).
pub fn open_with(
    filename: &str,
    defaults: &MimeDefaults,
    installed: &[DesktopEntry],
    browser: Option<&str>,
) -> Vec<OpenWith> {
    let mime = mime_for(filename);
    let mut rows: Vec<OpenWith> = Vec::new();

    // The default the double-click actually follows, first and marked. A
    // Browser answer with no browser on the machine gets no row: there is
    // nothing to open it with, and an entry that cannot launch is a lie.
    // The fallback names are what the routes are called on screen; an
    // installed entry's own name wins over them.
    let default_row: Option<(String, String)> = match classify_with(filename, defaults, installed) {
        FileAction::Browser => {
            browser.map(|id| (id.to_string(), name_for(id, "Browser", installed)))
        }
        FileAction::ImageViewer => Some((
            "yantrik-image-viewer.desktop".to_string(),
            name_for("yantrik-image-viewer.desktop", "Images", installed),
        )),
        FileAction::TextEditor => Some((
            "yantrik-text-editor.desktop".to_string(),
            name_for("yantrik-text-editor.desktop", "Editor", installed),
        )),
        FileAction::MediaPlayer => {
            Some(("mpv.desktop".to_string(), name_for("mpv.desktop", "Media Player", installed)))
        }
        FileAction::DesktopApp(id) => {
            let stem = id.strip_suffix(".desktop").unwrap_or(&id).to_string();
            let name = name_for(&id, &stem, installed);
            Some((id, name))
        }
    };
    if let Some((id, name)) = default_row {
        rows.push(OpenWith { id, name, is_default: true });
    }

    // Every installed app that declares the type, in catalogue order, skipping
    // the default already listed.
    if let Some(mime) = mime {
        for entry in installed {
            let id = format!("{}.desktop", entry.app_id);
            let declares = entry.mime_types.iter().any(|m| m.eq_ignore_ascii_case(mime));
            if declares && !rows.iter().any(|r| r.id.eq_ignore_ascii_case(&id)) {
                rows.push(OpenWith { id, name: entry.name.clone(), is_default: false });
            }
        }
    }

    // The Text Editor last: any file can be read as text, but offering it
    // first is what made #233 look broken.
    let editor = "yantrik-text-editor.desktop";
    if !rows.iter().any(|r| r.id.eq_ignore_ascii_case(editor)) {
        rows.push(OpenWith {
            id: editor.to_string(),
            name: name_for(editor, "Editor", installed),
            is_default: false,
        });
    }

    rows
}

/// The display name for a desktop id: the installed entry's own name where
/// there is one, `fallback` otherwise.
fn name_for(id: &str, fallback: &str, installed: &[DesktopEntry]) -> String {
    installed
        .iter()
        .find(|e| format!("{}.desktop", e.app_id).eq_ignore_ascii_case(id))
        .map(|e| e.name.clone())
        .unwrap_or_else(|| fallback.to_string())
}

/// List sibling image files in the same directory, sorted alphabetically.
/// Returns (image_paths, index_of_current).
pub fn sibling_images(file_path: &Path) -> (Vec<PathBuf>, usize) {
    let dir = file_path.parent().unwrap_or(Path::new("/"));
    let mut images: Vec<PathBuf> = Vec::new();

    // The defaults and the app list are read once for the directory, not once per entry.
    let defaults = MimeDefaults::read();
    let installed = crate::apps::Catalogue::shared().get();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                if classify_with(&name, &defaults, &installed) == FileAction::ImageViewer {
                    images.push(path);
                }
            }
        }
    }

    images.sort_by(|a, b| {
        a.file_name()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .cmp(&b.file_name().unwrap_or_default().to_ascii_lowercase())
    });

    let current_idx = images
        .iter()
        .position(|p| p == file_path)
        .unwrap_or(0);

    (images, current_idx)
}

/// Check if a lowercase filename ends with any of the given extensions.
fn matches_ext(lower: &str, exts: &[&str]) -> bool {
    exts.iter().any(|ext| lower.ends_with(ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An installed app, as little of it as the deciding here reads.
    fn entry(app_id: &str, name: &str, mime_types: &[&str]) -> DesktopEntry {
        DesktopEntry {
            app_id: app_id.to_string(),
            name: name.to_string(),
            exec: format!("yantrik-{app_id}"),
            mime_types: mime_types.iter().map(|m| m.to_string()).collect(),
            ..Default::default()
        }
    }

    /// The built-in table: extension -> the app that opens it. `.html` opening
    /// in the Browser is the case #233 was reported for — it used to open in
    /// the Text Editor and show the tags.
    #[test]
    fn classify_by_extension() {
        let no_defaults = MimeDefaults::default();
        let cases: &[(&str, FileAction)] = &[
            // Web pages, their vector graphics and PDFs
            ("page.html", FileAction::Browser),
            ("PAGE.HTM", FileAction::Browser),
            ("logo.svg", FileAction::Browser),
            ("manual.pdf", FileAction::Browser),
            // Images
            ("photo.jpg", FileAction::ImageViewer),
            ("icon.PNG", FileAction::ImageViewer),
            ("art.webp", FileAction::ImageViewer),
            // Audio
            ("song.mp3", FileAction::MediaPlayer),
            ("track.FLAC", FileAction::MediaPlayer),
            // Video
            ("movie.mp4", FileAction::MediaPlayer),
            // Text and code
            ("main.rs", FileAction::TextEditor),
            ("readme.md", FileAction::TextEditor),
            ("config.yaml", FileAction::TextEditor),
            ("app.js", FileAction::TextEditor),
            ("Makefile", FileAction::TextEditor),
        ];
        for (name, want) in cases {
            assert_eq!(&classify_with(name, &no_defaults, &[]), want, "{name} opens in the wrong app");
        }
    }

    /// The person's `mimeapps.list` wins over the built-in table, for every
    /// type whose default names an app the shell knows how to open.
    #[test]
    fn mimeapps_default_overrides_the_table() {
        let defaults = MimeDefaults::parse(
            "# what the person chose\n\
             [Added Associations]\n\
             text/html=chromium.desktop;\n\
             \n\
             [Default Applications]\n\
             text/html=yantrik-text-editor.desktop;firefox.desktop;\n\
             image/png=firefox.desktop\n\
             application/pdf=Firefox.desktop\n\
             video/mp4=vlc.desktop\n",
        );
        // The default for text/html names the Text Editor, so the Text Editor
        // opens it — and the [Added Associations] entry above did not decide
        // that, because a preference is not a default.
        assert_eq!(classify_with("page.html", &defaults, &[]), FileAction::TextEditor);
        // A browser set as the default for a type opens the Browser, whatever
        // the built-in table says the type is for — here an image, and a PDF
        // spelled with a capital letter in the desktop id.
        assert_eq!(classify_with("photo.png", &defaults, &[]), FileAction::Browser);
        assert_eq!(classify_with("manual.pdf", &defaults, &[]), FileAction::Browser);
        // vlc.desktop names no app the shell routes to and no installed entry
        // in this test, so its line is not followed and the built-in table
        // still answers for video.
        assert_eq!(classify_with("movie.mp4", &defaults, &[]), FileAction::MediaPlayer);
        // A type with no default line is untouched.
        assert_eq!(classify_with("song.mp3", &defaults, &[]), FileAction::MediaPlayer);
    }

    /// A mimeapps.list that is missing, empty or malformed simply leaves no
    /// defaults, and the built-in table answers everything.
    #[test]
    fn mimeapps_parse_tolerates_rubbish() {
        for contents in ["", "text/html=firefox.desktop\n", "[Default Applications]\nnoequals\n"] {
            let defaults = MimeDefaults::parse(contents);
            assert_eq!(classify_with("page.html", &defaults, &[]), FileAction::Browser);
        }
    }

    /// "Always use this app" writes a default naming an app the shell has no
    /// route of its own to. The next double-click must follow it — a promise
    /// the shell then ignores is worse than no promise (#233).
    #[test]
    fn a_default_naming_an_installed_app_is_followed() {
        let defaults = MimeDefaults::parse(
            "[Default Applications]\n\
             application/vnd.oasis.opendocument.text=yantrik-libreoffice.desktop;\n",
        );
        let installed = [entry("yantrik-libreoffice", "LibreOffice", &[])];
        assert_eq!(
            classify_with("notes.odt", &defaults, &installed),
            FileAction::DesktopApp("yantrik-libreoffice.desktop".to_string())
        );
        // The same line with the app uninstalled (or gone since) cannot be
        // followed: nothing would run. The fallback answers instead.
        assert_eq!(classify_with("notes.odt", &defaults, &[]), FileAction::TextEditor);
    }

    /// A type the built-in table has no app for opens in an installed app that
    /// declares it in its `MimeType=` line — the freedesktop rule, and the
    /// second half of #233: an .odt with LibreOffice installed opens there,
    /// not in the Text Editor.
    #[test]
    fn an_installed_app_that_declares_the_type_opens_it() {
        let installed = [entry(
            "yantrik-libreoffice",
            "LibreOffice",
            &["application/vnd.oasis.opendocument.text"],
        )];
        assert_eq!(
            classify_with("notes.odt", &MimeDefaults::default(), &installed),
            FileAction::DesktopApp("yantrik-libreoffice.desktop".to_string())
        );
        // No app declares the type: the Text Editor fallback still answers.
        assert_eq!(
            classify_with("notes.odt", &MimeDefaults::default(), &[]),
            FileAction::TextEditor
        );
    }

    /// Where the shell's own table has an app for the type, it wins over a
    /// catalogue entry that also declares it: a browser the person chose opens
    /// html through the shell's browser route, not a second launcher (#233).
    #[test]
    fn the_builtin_table_beats_a_catalogue_entry() {
        let installed = [entry("yantrik-libreoffice", "LibreOffice", &["text/html"])];
        assert_eq!(classify_with("page.html", &MimeDefaults::default(), &installed), FileAction::Browser);
    }

    /// The "Open with" list: the default the double-click follows comes first
    /// and is marked, apps declaring the type follow, the Text Editor is the
    /// last resort, and no id appears twice (#233).
    #[test]
    fn open_with_lists_the_default_first_and_the_editor_last() {
        let installed = [
            entry("chromium", "Chromium Web Browser", &["text/html"]),
            entry("yantrik-libreoffice", "LibreOffice", &["text/html"]),
        ];
        let rows = open_with("page.html", &MimeDefaults::default(), &installed, Some("chromium.desktop"));
        assert_eq!(
            rows,
            vec![
                OpenWith { id: "chromium.desktop".into(), name: "Chromium Web Browser".into(), is_default: true },
                OpenWith { id: "yantrik-libreoffice.desktop".into(), name: "LibreOffice".into(), is_default: false },
                OpenWith { id: "yantrik-text-editor.desktop".into(), name: "Editor".into(), is_default: false },
            ],
            "the default once and first, the other html app next, the editor last"
        );

        // A machine with no browser offers no Browser row: an entry that
        // cannot launch is a lie.
        let rows = open_with("page.html", &MimeDefaults::default(), &[], None);
        assert!(rows.iter().all(|r| r.id != "chromium.desktop"));
        assert_eq!(rows.last().unwrap().id, "yantrik-text-editor.desktop");

        // A type nothing declares still offers the editor.
        let rows = open_with("notes.odt", &MimeDefaults::default(), &[], Some("chromium.desktop"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "yantrik-text-editor.desktop");
    }

    /// "Always use this app" writes the person's `mimeapps.list`: creating the
    /// section when the file has none, replacing the type's line when it has
    /// one, and leaving everything else — other types, comments,
    /// `[Added Associations]` — untouched (#233).
    #[test]
    fn with_default_writes_the_defaults_section() {
        // No file yet: the section is created.
        assert_eq!(
            with_default("", "text/html", "chromium.desktop"),
            "[Default Applications]\ntext/html=chromium.desktop;\n"
        );
        // An existing line for the type is replaced where it stands.
        let contents = "[Default Applications]\ntext/html=firefox.desktop;\nimage/png=gimp.desktop;\n";
        let written = with_default(contents, "text/html", "chromium.desktop");
        assert_eq!(
            written,
            "[Default Applications]\ntext/html=chromium.desktop;\nimage/png=gimp.desktop;\n"
        );
        // A section with no line for the type: the line is added under the head,
        // and the other sections and comments survive.
        let contents = "# mine\n[Default Applications]\nimage/png=gimp.desktop;\n\n[Added Associations]\ntext/html=firefox.desktop;\n";
        let written = with_default(contents, "text/html", "chromium.desktop");
        assert_eq!(
            written,
            "# mine\n[Default Applications]\ntext/html=chromium.desktop;\nimage/png=gimp.desktop;\n\n[Added Associations]\ntext/html=firefox.desktop;\n"
        );
        // What the writer writes, the reader reads back as the new default.
        let defaults = MimeDefaults::parse(&with_default(contents, "text/html", "chromium.desktop"));
        assert_eq!(classify_with("page.html", &defaults, &[]), FileAction::Browser);
    }

    /// The control surface's answer names the app the one rule picked, so
    /// `files_open` says what opened the file (#233).
    #[test]
    fn app_name_names_the_app_the_rule_picked() {
        assert_eq!(app_name(&FileAction::Browser), "browser");
        assert_eq!(app_name(&FileAction::TextEditor), "editor");
        assert_eq!(app_name(&FileAction::ImageViewer), "images");
        assert_eq!(app_name(&FileAction::MediaPlayer), "media-player");
        assert_eq!(
            app_name(&FileAction::DesktopApp("yantrik-libreoffice.desktop".to_string())),
            "yantrik-libreoffice"
        );
    }
}
