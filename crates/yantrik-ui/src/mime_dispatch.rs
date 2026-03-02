//! MIME dispatch — route file opens to internal viewers or external apps.
//!
//! Classifies files by extension and returns a `FileAction` telling the
//! caller which internal screen to open (or whether to launch externally).

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
    /// Launch via external command (e.g. mpv for video).
    External(String),
}

/// Classify a filename into a `FileAction` based on its extension.
pub fn classify(filename: &str) -> FileAction {
    let lower = filename.to_lowercase();

    // Images
    if matches_ext(&lower, &[".jpg", ".jpeg", ".png", ".gif", ".bmp", ".webp", ".svg", ".ico", ".tiff", ".tif"]) {
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

    // PDF — launch externally
    if lower.ends_with(".pdf") {
        return FileAction::External("xdg-open".to_string());
    }

    // Text / code / config — open in editor
    if matches_ext(&lower, &[
        ".txt", ".md", ".rs", ".py", ".sh", ".bash", ".zsh",
        ".js", ".ts", ".jsx", ".tsx", ".html", ".css", ".scss",
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

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                if classify(&name) == FileAction::ImageViewer {
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

    #[test]
    fn test_classify_image() {
        assert_eq!(classify("photo.jpg"), FileAction::ImageViewer);
        assert_eq!(classify("icon.PNG"), FileAction::ImageViewer);
        assert_eq!(classify("art.webp"), FileAction::ImageViewer);
    }

    #[test]
    fn test_classify_audio() {
        assert_eq!(classify("song.mp3"), FileAction::AudioPlayer);
        assert_eq!(classify("track.FLAC"), FileAction::AudioPlayer);
    }

    #[test]
    fn test_classify_text() {
        assert_eq!(classify("main.rs"), FileAction::TextEditor);
        assert_eq!(classify("readme.md"), FileAction::TextEditor);
        assert_eq!(classify("config.yaml"), FileAction::TextEditor);
    }

    #[test]
    fn test_classify_video() {
        assert_eq!(classify("movie.mp4"), FileAction::External("mpv".to_string()));
    }
}
