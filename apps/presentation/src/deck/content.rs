//! What a deck is, as it is stored: its slides, its theme, and the `.ydeck` format they are
//! written in. No window and no disk: the arithmetic of reading and writing a deck is here so it
//! can be tested without either.

use serde::{Deserialize, Serialize};

/// The shape this build writes and the highest it will read.
///
/// It is the first field of the file for the same reason Download Manager's is: a reader that
/// does not understand the rest can still find out that it does not, and say so, instead of
/// guessing at fields it has never heard of.
pub const FORMAT_VERSION: u32 = 1;

/// The extension the format is named by, used when a deck is given a home of its own.
pub const EXTENSION: &str = "ydeck";

/// A deck this size is not a deck; it is a file that happens to parse. Bounded so a malformed
/// or hostile file cannot be loaded into an unbounded model.
pub const MAX_SLIDES: usize = 500;

/// Bytes. A deck of 500 short slides is well under a megabyte; four is room for long notes.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// The layouts `presentation.slint` draws, by index. Anything outside this is clamped on load
/// rather than drawn as a blank slide nobody chose.
pub const LAYOUT_COUNT: i32 = 6;

// ── Themes ───────────────────────────────────────────────────────────

/// A slide theme as plain numbers, so the table can live beside the deck rather than inside the
/// window. The view turns one of these into the screen's `PresentTheme`.
#[derive(Clone, Copy, Debug)]
pub struct ThemeSpec {
    pub name: &'static str,
    pub bg: (u8, u8, u8),
    pub text: (u8, u8, u8),
    pub accent: (u8, u8, u8),
}

/// The themes the screen can draw.
///
/// The screen reads `themes[0]` and only `themes[0]` for the canvas, the presenter view and the
/// thumbnails, so the app hands it the one that is chosen rather than a list to pick from. The
/// chosen theme is part of the deck and is saved with it.
pub const THEMES: [ThemeSpec; 5] = [
    ThemeSpec { name: "Slate", bg: (0x1B, 0x1E, 0x24), text: (0xE8, 0xEA, 0xED), accent: (0x4E, 0x79, 0xA7) },
    ThemeSpec { name: "Paper", bg: (0xFA, 0xF8, 0xF3), text: (0x22, 0x24, 0x28), accent: (0xC2, 0x5E, 0x1E) },
    ThemeSpec { name: "Ink", bg: (0x0E, 0x0F, 0x12), text: (0xF2, 0xF3, 0xF5), accent: (0x76, 0xB7, 0xB2) },
    ThemeSpec { name: "Sand", bg: (0xEC, 0xE3, 0xD2), text: (0x33, 0x2C, 0x22), accent: (0x59, 0x6E, 0x3A) },
    ThemeSpec { name: "Plum", bg: (0x24, 0x1B, 0x2A), text: (0xEE, 0xE6, 0xF2), accent: (0xE1, 0x57, 0x59) },
];

/// The chosen theme, whatever number was stored. A file written by a build with more themes
/// than this one has must not leave the window with no colours at all.
pub fn theme_spec(index: usize) -> &'static ThemeSpec {
    &THEMES[index % THEMES.len()]
}

// ── The content ──────────────────────────────────────────────────────

/// One slide, as it is stored.
///
/// The slide's number is deliberately not here: it is a position in a vector, and the screen is
/// given the index at render time, so there is no second source of truth to keep in step.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slide {
    pub title: String,
    pub body: String,
    pub notes: String,
    /// Index into the screen's layout list, 0..LAYOUT_COUNT.
    #[serde(default)]
    pub layout: i32,
}

impl Slide {
    pub fn new(title: impl Into<String>, body: impl Into<String>, layout: i32) -> Self {
        Self { title: title.into(), body: body.into(), notes: String::new(), layout: layout.clamp(0, LAYOUT_COUNT - 1) }
    }

    /// Does any of this slide's text contain `needle`, case-insensitively?
    pub(super) fn matches(&self, needle: &str) -> bool {
        let needle = needle.to_lowercase();
        self.title.to_lowercase().contains(&needle)
            || self.body.to_lowercase().contains(&needle)
            || self.notes.to_lowercase().contains(&needle)
    }
}

/// Everything a `.ydeck` holds, and everything `dirty()` compares.
///
/// The selected slide is not in here on purpose: moving the selection is not an edit, and a deck
/// that reports unsaved changes because someone looked at slide three would train a person to
/// ignore the word.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Content {
    pub title: String,
    #[serde(default)]
    pub theme: usize,
    pub slides: Vec<Slide>,
}

/// The file, with its version in front of its content.
#[derive(Serialize, Deserialize)]
struct DeckFile {
    version: u32,
    title: String,
    #[serde(default)]
    theme: usize,
    slides: Vec<Slide>,
}

/// What is on the canvas right now.
///
/// The screen binds `text <=> current-title` and friends two-way, so these four properties are
/// the live text. The window hands them to every command that moves the selection or changes
/// the deck, and the deck commits them before it does anything else.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edits {
    pub title: String,
    pub body: String,
    pub notes: String,
    pub layout: i32,
}

impl From<&Slide> for Edits {
    fn from(s: &Slide) -> Self {
        Self { title: s.title.clone(), body: s.body.clone(), notes: s.notes.clone(), layout: s.layout }
    }
}

// ── Failures with names ──────────────────────────────────────────────

/// Why a file could not be loaded. Named rather than stringly, because "never half-load" means
/// the caller has to be able to keep the deck it already has and say which of these happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The file could not be read at all.
    Unreadable(String),
    /// Larger than [`MAX_FILE_BYTES`].
    TooLarge(u64),
    /// Not JSON.
    NotJson(String),
    /// JSON, but with no `version` field, so there is no way to know what it is.
    VersionMissing,
    /// Written by a newer build. Read as far as the version and no further.
    VersionAhead(u32),
    /// The right version, the wrong shape.
    Malformed(String),
    /// A deck with no slides has nothing to show and cannot be edited into one.
    NoSlides,
    /// More slides than this build will hold.
    TooManySlides(usize),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Unreadable(e) => write!(f, "could not read it: {e}"),
            LoadError::TooLarge(n) => {
                write!(f, "it is {n} bytes, over the {MAX_FILE_BYTES}-byte limit; nothing was loaded")
            }
            LoadError::NotJson(e) => write!(f, "it is not a yPresent deck: {e}"),
            LoadError::VersionMissing => write!(f, "it has no `version`, so there is no way to tell what it is"),
            LoadError::VersionAhead(v) => write!(
                f,
                "it is version {v} and this build reads version {FORMAT_VERSION}; the deck you had is untouched"
            ),
            LoadError::Malformed(e) => write!(f, "version {FORMAT_VERSION}, but not that shape: {e}"),
            LoadError::NoSlides => write!(f, "it holds no slides"),
            LoadError::TooManySlides(n) => write!(f, "it holds {n} slides, over the {MAX_SLIDES} this build will open"),
        }
    }
}

// ── Reading and writing the format ───────────────────────────────────

/// Turn a `.ydeck` file's text into content, or say exactly why not.
///
/// The version is read off a `serde_json::Value` before anything is deserialised into the
/// struct. A future version is free to rename every other field, and a `from_str::<DeckFile>`
/// would then report a missing `slides` where the truth is "this was written by a newer build".
pub fn parse(text: &str) -> Result<Content, LoadError> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| LoadError::NotJson(e.to_string()))?;
    check_version(&value)?;
    let file: DeckFile = serde_json::from_value(value).map_err(|e| LoadError::Malformed(e.to_string()))?;
    bounded(file.slides.len())?;
    Ok(Content {
        title: file.title,
        theme: file.theme % THEMES.len(),
        slides: file
            .slides
            .into_iter()
            .map(|mut s| {
                s.layout = s.layout.clamp(0, LAYOUT_COUNT - 1);
                s
            })
            .collect(),
    })
}

/// The version of a parsed file: present, and not newer than this build reads.
pub(crate) fn check_version(value: &serde_json::Value) -> Result<(), LoadError> {
    match value.get("version").and_then(|v| v.as_u64()) {
        None => Err(LoadError::VersionMissing),
        Some(v) if v > FORMAT_VERSION as u64 => Err(LoadError::VersionAhead(v as u32)),
        Some(_) => Ok(()),
    }
}

/// A deck holds at least one slide and at most [`MAX_SLIDES`].
pub(crate) fn bounded(slides: usize) -> Result<(), LoadError> {
    match slides {
        0 => Err(LoadError::NoSlides),
        n if n > MAX_SLIDES => Err(LoadError::TooManySlides(n)),
        _ => Ok(()),
    }
}

/// The text a `.ydeck` file holds. Pretty-printed: a deck is small, and a file a person can read
/// in an editor and a `diff` can show line by line is worth the extra bytes.
pub fn encode(content: &Content) -> String {
    let file = DeckFile {
        version: FORMAT_VERSION,
        title: content.title.clone(),
        theme: content.theme,
        slides: content.slides.clone(),
    };
    // The shape is fixed and every field is serialisable, so this cannot fail; the fallback
    // exists so a serialiser change can never take the window down with it.
    serde_json::to_string_pretty(&file)
        .unwrap_or_else(|_| format!("{{\"version\":{FORMAT_VERSION},\"title\":\"\",\"slides\":[]}}"))
}
