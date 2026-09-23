//! MIME dispatch — route file opens to internal viewers or external apps.
//!
//! Classifies files by extension and returns a `FileAction` telling the
//! caller which app opens them. The person's own defaults, from the
//! `[Default Applications]` section of their `mimeapps.list`, come first:
//! where they name an app the shell knows how to open, that app opens the
//! file, whatever the built-in table would have said (#233).

use std::path::{Path, PathBuf};

/// What to do when a file is opened.
#[derive(Debug, Clone, PartialEq)]
pub enum FileAction {
    /// Open in internal Image Viewer (screen 11).
    ImageViewer,
    /// Open in internal Text Editor (screen 12).
    TextEditor,
    /// Open in internal Audio Player (screen 13).
    AudioPlayer,
    /// Open in the machine's web browser, whichever `wire::dock::find_browser` finds.
    Browser,
    /// Launch via external command (e.g. mpv for video).
    External(String),
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
}

/// Classify a filename into a `FileAction`, following the person's own
/// defaults where they have set any.
pub fn classify(filename: &str) -> FileAction {
    classify_with(filename, &MimeDefaults::read())
}

/// The pure half of [`classify`]: the answer given the filename and the
/// parsed contents of the person's `mimeapps.list`. All the deciding is here
/// so the tests can drive it; the only I/O is `MimeDefaults::read`.
pub fn classify_with(filename: &str, defaults: &MimeDefaults) -> FileAction {
    if let Some(mime) = mime_for(filename) {
        if let Some(id) = defaults.default_for(mime) {
            // A default that names an app the shell has no route to is not followed:
            // nothing here reads .desktop entries to find their Exec lines, so the
            // built-in table answers instead of a guess.
            if let Some(action) = action_for_desktop(id) {
                return action;
            }
        }
    }
    builtin(filename)
}

/// The desktop entry ids the shell knows how to open, and what each means.
///
/// The browser ids are the distributions' spellings of the same browsers
/// `wire::dock::find_browser` looks for. Whichever of them the person chose,
/// the shell opens the browser this machine has, with the file as its
/// argument — the Browser pin's own behaviour, not a second launcher.
fn action_for_desktop(id: &str) -> Option<FileAction> {
    match id.trim().to_lowercase().as_str() {
        "yantrik-text-editor.desktop" => Some(FileAction::TextEditor),
        "yantrik-image-viewer.desktop" => Some(FileAction::ImageViewer),
        "yantrik-music-player.desktop" => Some(FileAction::AudioPlayer),
        "mpv.desktop" => Some(FileAction::External("mpv".to_string())),
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
fn mime_for(filename: &str) -> Option<&'static str> {
    let lower = filename.to_lowercase();
    MIME_BY_EXT
        .iter()
        .find(|(ext, _)| lower.ends_with(*ext))
        .map(|(_, mime)| *mime)
}

/// Extension to MIME type for the extensions the built-in table knows. The
/// two lists are kept in step by hand: an extension added to `builtin` with no
/// row here still opens, it just cannot be given a default of its own.
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
];

/// The shell's own associations, for a file the person has set no default for.
fn builtin(filename: &str) -> FileAction {
    let lower = filename.to_lowercase();

    // Web pages, their vector graphics and PDFs — the Browser. An .html in the
    // Text Editor put tags on the screen where a person wanted a page (#233),
    // and the browsers this desktop installs render all three.
    if matches_ext(&lower, &[".html", ".htm", ".svg", ".pdf"]) {
        return FileAction::Browser;
    }

    // Images
    if matches_ext(&lower, &[".jpg", ".jpeg", ".png", ".gif", ".bmp", ".webp", ".ico", ".tiff", ".tif"]) {
        return FileAction::ImageViewer;
    }

    // Audio
    if matches_ext(&lower, &[".mp3", ".ogg", ".flac", ".wav", ".m4a", ".aac", ".opus", ".wma"]) {
        return FileAction::AudioPlayer;
    }

    // Video — launch mpv externally
    if matches_ext(&lower, &[".mp4", ".mkv", ".avi", ".webm", ".mov", ".wmv", ".flv"]) {
        return FileAction::External("mpv".to_string());
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
        return FileAction::TextEditor;
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
        return FileAction::TextEditor;
    }

    // Unknown — try text editor as fallback for small files
    FileAction::TextEditor
}

/// List sibling image files in the same directory, sorted alphabetically.
/// Returns (image_paths, index_of_current).
pub fn sibling_images(file_path: &Path) -> (Vec<PathBuf>, usize) {
    let dir = file_path.parent().unwrap_or(Path::new("/"));
    let mut images: Vec<PathBuf> = Vec::new();

    // The defaults are read once for the directory, not once per entry.
    let defaults = MimeDefaults::read();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                if classify_with(&name, &defaults) == FileAction::ImageViewer {
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
            ("song.mp3", FileAction::AudioPlayer),
            ("track.FLAC", FileAction::AudioPlayer),
            // Video
            ("movie.mp4", FileAction::External("mpv".to_string())),
            // Text and code
            ("main.rs", FileAction::TextEditor),
            ("readme.md", FileAction::TextEditor),
            ("config.yaml", FileAction::TextEditor),
            ("app.js", FileAction::TextEditor),
            ("Makefile", FileAction::TextEditor),
        ];
        for (name, want) in cases {
            assert_eq!(&classify_with(name, &no_defaults), want, "{name} opens in the wrong app");
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
        assert_eq!(classify_with("page.html", &defaults), FileAction::TextEditor);
        // A browser set as the default for a type opens the Browser, whatever
        // the built-in table says the type is for — here an image, and a PDF
        // spelled with a capital letter in the desktop id.
        assert_eq!(classify_with("photo.png", &defaults), FileAction::Browser);
        assert_eq!(classify_with("manual.pdf", &defaults), FileAction::Browser);
        // vlc.desktop is an app the shell has no route to, so its line is not
        // followed and the built-in table still answers for video.
        assert_eq!(
            classify_with("movie.mp4", &defaults),
            FileAction::External("mpv".to_string())
        );
        // A type with no default line is untouched.
        assert_eq!(classify_with("song.mp3", &defaults), FileAction::AudioPlayer);
    }

    /// A mimeapps.list that is missing, empty or malformed simply leaves no
    /// defaults, and the built-in table answers everything.
    #[test]
    fn mimeapps_parse_tolerates_rubbish() {
        for contents in ["", "text/html=firefox.desktop\n", "[Default Applications]\nnoequals\n"] {
            let defaults = MimeDefaults::parse(contents);
            assert_eq!(classify_with("page.html", &defaults), FileAction::Browser);
        }
    }
}
