//! The library: characters and games as plain files under one directory.
//!
//! `characters/<slug>.json` holds a validated CharacterSpec. `games/<slug>/` holds
//! `spec.json`, the compiled `index.html`, and — once the verifier has run —
//! `verify.json` and `screenshot.png`. Plain files in the ordinary data directory
//! because a built game is a deliverable a person can copy anywhere and open by
//! double-clicking; nothing here needs a database, and everything survives a
//! reinstall of the app.
//!
//! Deleting a game moves it to the freedesktop Trash, where the desktop's Files
//! app can bring it back. That is why `delete` is graded recoverable-standard
//! rather than dangerous on the control surface.
//!
//! Updating a spec in place keeps the one it replaces (`spec.previous.json`
//! beside a game, `characters/previous/<slug>.json` for a character) and drops
//! whatever was compiled from the old one, so the list never shows a build as
//! current when the spec under it has moved on.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Local;
use serde_json::Value;

use crate::compile;
use crate::spec::{parse_character, parse_game, CharacterRef, CharacterSpec, GameSpec, ResolvedGame};
use crate::verify;

#[cfg(test)]
mod tests;
mod trash;
mod write;

pub use write::{private_folder, stamp_of, write_in_place, Stamp};
use write::write_private;

/// Where the library lives: `YANTRIK_ARCADE_DIR` when it is set and not empty, otherwise
/// `$HOME/.local/share/yantrik/arcade`, where it has always been. Not `$XDG_DATA_HOME`: the
/// window and the command line read this one rule, so a game built from a terminal is the game
/// the window lists, and a library the old build made is found where it was.
pub fn resolve_root(dir_override: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> PathBuf {
    match dir_override.filter(|d| !d.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(home.unwrap_or_else(|| "/tmp".into())).join(".local/share/yantrik/arcade"),
    }
}

/// A character as the list shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct CharacterEntry {
    pub slug: String,
    pub name: String,
    pub archetype: String,
}

/// A game as the list shows it: what exists on disk, not what the spec promises.
#[derive(Debug, Clone, PartialEq)]
pub struct GameEntry {
    pub slug: String,
    pub title: String,
    pub built: bool,
    pub last_build: Option<String>,
    /// One line out of verify.json, e.g. "passed all 7 gates" or the first failure.
    pub verification: Option<String>,
}

#[derive(Clone)]
pub struct Library {
    root: PathBuf,
    /// Tests point this at a pretend Trash; in the app it is None and the real
    /// freedesktop directories are resolved at delete time. An injected field
    /// rather than an env var because unit tests share one process and mutating
    /// XDG_DATA_HOME would race.
    trash_override: Option<PathBuf>,
}

impl Library {
    /// The real library, honouring `YANTRIK_ARCADE_DIR` so tests (and the CLI)
    /// can point the app somewhere disposable.
    pub fn open() -> Library {
        let root = resolve_root(std::env::var_os("YANTRIK_ARCADE_DIR"), std::env::var_os("HOME"));
        Library { root, trash_override: None }
    }

    /// The library an app started at `paths` keeps: the one [`Library::open`] finds, so the
    /// window and the command line cannot read different libraries. Under a test it is the app's
    /// data folder, and the Trash is a folder of the test's own.
    pub fn for_app(paths: &yantrik_app::Paths) -> Library {
        #[cfg(test)]
        return Library { root: paths.data_dir().to_path_buf(), trash_override: Some(paths.state_dir().join("trash")) };
        #[cfg(not(test))]
        {
            let _ = paths;
            Library::open()
        }
    }

    /// A library rooted at an explicit directory — what every test here uses.
    /// Test-only on purpose: the shipping paths all go through `open()` so there
    /// is exactly one way to find the real library.
    #[cfg(test)]
    pub fn at(root: impl Into<PathBuf>) -> Library {
        Library { root: root.into(), trash_override: None }
    }

    /// Send deletes to a pretend Trash data directory instead of the real one.
    #[cfg(test)]
    pub fn with_trash_at(mut self, data_dir: impl Into<PathBuf>) -> Library {
        self.trash_override = Some(data_dir.into());
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn characters_dir(&self) -> PathBuf {
        self.root.join("characters")
    }

    pub fn game_dir(&self, slug: &str) -> PathBuf {
        self.root.join("games").join(slug)
    }

    pub fn game_html(&self, slug: &str) -> PathBuf {
        self.game_dir(slug).join("index.html")
    }

    pub fn game_spec_path(&self, slug: &str) -> PathBuf {
        self.game_dir(slug).join("spec.json")
    }

    pub fn verify_path(&self, slug: &str) -> PathBuf {
        self.game_dir(slug).join("verify.json")
    }

    pub fn screenshot_path(&self, slug: &str) -> PathBuf {
        self.game_dir(slug).join("screenshot.png")
    }

    // ── Characters ────────────────────────────────────────────────

    /// Validate and save a character spec. Refusals are the validator's sentences.
    pub fn save_character(&self, text: &str) -> Result<CharacterEntry, String> {
        let spec = parse_character(text)?;
        let slug = slugify(&spec.name);
        if slug.is_empty() {
            return Err(format!(
                "`name` ({:?}) leaves nothing usable as a filename; use letters, digits or spaces.",
                spec.name
            ));
        }
        fs::create_dir_all(self.characters_dir()).map_err(|e| format!("cannot create the characters directory: {e}"))?;
        let path = self.characters_dir().join(format!("{slug}.json"));
        if path.exists() {
            return Err(format!(
                "A character called {:?} is already saved; delete it first or pick another name.",
                spec.name
            ));
        }
        let json = serde_json::to_string_pretty(&spec).expect("a parsed spec re-serializes");
        write_private(&path, json.as_bytes())?;
        Ok(CharacterEntry {
            slug,
            name: spec.name.clone(),
            archetype: spec.archetype.as_str().to_string(),
        })
    }

    /// Replace a saved character's spec. The name is the key: a character whose
    /// name is not yet in the library is refused, because that is `new_character`'s
    /// job and a typo in the name should not quietly make a second creature.
    ///
    /// Games that cast this character by name were compiled with the old one, so
    /// their builds are dropped along with their verdicts and screenshots — `build`
    /// is milliseconds and the specs are untouched — and the slugs of the games
    /// that need it come back so the caller can say so. A game that inlined a copy
    /// of the character is not affected: it has its own.
    pub fn update_character(&self, text: &str) -> Result<(CharacterEntry, Vec<String>), String> {
        let spec = parse_character(text)?;
        let slug = slugify(&spec.name);
        let path = self.characters_dir().join(format!("{slug}.json"));
        if slug.is_empty() || !path.exists() {
            return Err(format!(
                "No character called {:?} is saved to update; `new_character` saves a new one, and `describe` lists the ones that exist.",
                spec.name
            ));
        }
        let previous_dir = self.characters_dir().join("previous");
        fs::create_dir_all(&previous_dir).map_err(|e| format!("cannot create {}: {e}", previous_dir.display()))?;
        fs::copy(&path, previous_dir.join(format!("{slug}.json")))
            .map_err(|e| format!("cannot keep the previous spec: {e}"))?;
        let json = serde_json::to_string_pretty(&spec).expect("a parsed spec re-serializes");
        write_private(&path, json.as_bytes())?;

        let mut stale = Vec::new();
        for game in self.list_games() {
            if !game.built {
                continue;
            }
            let Ok((_, game_spec)) = self.load_game_spec(&game.slug) else { continue };
            let casts_by_name = matches!(
                game_spec.player.as_ref().map(|p| &p.character),
                Some(CharacterRef::Name(name)) if slugify(name) == slug || name.to_lowercase() == spec.name.to_lowercase()
            );
            if casts_by_name {
                self.drop_build(&game.slug);
                stale.push(game.slug);
            }
        }
        Ok((
            CharacterEntry { slug, name: spec.name.clone(), archetype: spec.archetype.as_str().to_string() },
            stale,
        ))
    }

    /// Find a saved character by its exact name or by slug, case-insensitively.
    pub fn load_character(&self, name_or_slug: &str) -> Result<CharacterSpec, String> {
        let want = name_or_slug.trim().to_lowercase();
        let dir = self.characters_dir();
        let entries = fs::read_dir(&dir).map_err(|_| {
            format!("No character called {:?} is saved: the character library is empty.", name_or_slug)
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let slug = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let Ok(text) = fs::read_to_string(&path) else { continue };
            let Ok(spec) = serde_json::from_str::<CharacterSpec>(&text) else { continue };
            if slug.eq_ignore_ascii_case(&want) || spec.name.to_lowercase() == want {
                return Ok(spec);
            }
        }
        Err(format!(
            "No character called {:?} is saved; `describe` lists the ones that are.",
            name_or_slug
        ))
    }

    pub fn list_characters(&self) -> Vec<CharacterEntry> {
        let mut out = Vec::new();
        if let Ok(entries) = fs::read_dir(self.characters_dir()) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let slug = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
                if let Ok(text) = fs::read_to_string(&path) {
                    if let Ok(spec) = serde_json::from_str::<CharacterSpec>(&text) {
                        out.push(CharacterEntry {
                            slug,
                            name: spec.name,
                            archetype: spec.archetype.as_str().to_string(),
                        });
                    }
                }
            }
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out
    }

    // ── Games ─────────────────────────────────────────────────────

    /// Validate a game spec and file it. Building is a separate step so a spec can
    /// be saved, looked at, and edited before anything is compiled.
    pub fn save_game(&self, text: &str) -> Result<GameEntry, String> {
        let spec = parse_game(text)?;
        let slug = slugify(&spec.title);
        if slug.is_empty() {
            return Err(format!(
                "`title` ({:?}) leaves nothing usable as a filename; use letters, digits or spaces.",
                spec.title
            ));
        }
        let dir = self.game_dir(&slug);
        if dir.exists() {
            return Err(format!(
                "A game called {:?} already exists; delete it first or pick another title.",
                spec.title
            ));
        }
        fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let json = serde_json::to_string_pretty(&spec).expect("a parsed spec re-serializes");
        write_private(&self.game_spec_path(&slug), json.as_bytes())
            .map_err(|e| format!("cannot write the spec: {e}"))?;
        Ok(self.describe_game(&slug, &spec))
    }

    /// Replace a saved game's spec. Design is iteration, and the only way to change
    /// a game used to be `delete` — graded sensitive, so an approval card per try
    /// (#112). This is graded standard because nothing is lost: the spec it replaces
    /// stays beside it as `spec.previous.json`, one step back.
    ///
    /// `existing` names the game to replace when the new spec carries a different
    /// title (a rename); otherwise the spec's own title says which game it is. The
    /// compiled HTML, the verdict and the screenshot go with the old spec, because
    /// they were made from it: the list says "not built" until `build` runs again.
    pub fn update_game(&self, text: &str, existing: Option<&str>) -> Result<GameEntry, String> {
        let spec = parse_game(text)?;
        let new_slug = slugify(&spec.title);
        if new_slug.is_empty() {
            return Err(format!(
                "`title` ({:?}) leaves nothing usable as a filename; use letters, digits or spaces.",
                spec.title
            ));
        }
        let key = existing.map(str::trim).filter(|s| !s.is_empty()).unwrap_or(&spec.title);
        let (old_slug, _) = self.load_game_spec(key).map_err(|_| {
            format!(
                "No game called {key:?} exists to update; `new_game` saves a new one, and `describe` lists the ones that exist."
            )
        })?;
        if new_slug != old_slug {
            if self.game_dir(&new_slug).exists() {
                return Err(format!(
                    "Retitling {old_slug:?} to {:?} would land on a game that already exists; delete that one first or pick another title.",
                    spec.title
                ));
            }
            fs::rename(self.game_dir(&old_slug), self.game_dir(&new_slug))
                .map_err(|e| format!("cannot retitle {old_slug} to {new_slug}: {e}"))?;
        }
        let dir = self.game_dir(&new_slug);
        fs::rename(self.game_spec_path(&new_slug), dir.join("spec.previous.json"))
            .map_err(|e| format!("cannot keep the previous spec: {e}"))?;
        let json = serde_json::to_string_pretty(&spec).expect("a parsed spec re-serializes");
        write_private(&self.game_spec_path(&new_slug), json.as_bytes())
            .map_err(|e| format!("cannot write the spec: {e}"))?;
        self.drop_build(&new_slug);
        Ok(self.describe_game(&new_slug, &spec))
    }

    /// Everything compiled or measured from a spec that is no longer current.
    fn drop_build(&self, slug: &str) {
        let _ = fs::remove_file(self.game_html(slug));
        let _ = fs::remove_file(self.verify_path(slug));
        let _ = fs::remove_file(self.screenshot_path(slug));
    }

    /// Load a filed spec by title or slug.
    pub fn load_game_spec(&self, title_or_slug: &str) -> Result<(String, GameSpec), String> {
        let want = title_or_slug.trim().to_lowercase();
        let want_slug = slugify(title_or_slug);
        let games_dir = self.root.join("games");
        let entries = fs::read_dir(&games_dir).map_err(|_| {
            format!("No game called {:?} exists: the library is empty.", title_or_slug)
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let slug = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
            let spec_path = path.join("spec.json");
            let Ok(text) = fs::read_to_string(&spec_path) else { continue };
            let Ok(spec) = serde_json::from_str::<GameSpec>(&text) else { continue };
            if slug.eq_ignore_ascii_case(&want)
                || slug == want_slug
                || spec.title.to_lowercase() == want
            {
                return Ok((slug, spec));
            }
        }
        Err(format!(
            "No game called {:?} exists; `describe` lists the ones that do.",
            title_or_slug
        ))
    }

    /// Compile a filed game into its index.html. Resolves a by-name character
    /// against the character library at build time.
    pub fn build_game(&self, title_or_slug: &str) -> Result<(String, PathBuf), String> {
        let (slug, spec) = self.load_game_spec(title_or_slug)?;
        let lib = self;
        let resolved = ResolvedGame::resolve(spec, &|name| lib.load_character(name))?;
        let html = compile::compile(&resolved);
        let out = self.game_html(&slug);
        write_private(&out, html.as_bytes())?;
        // A fresh build invalidates the old verdict: it was about the previous file.
        let _ = fs::remove_file(self.verify_path(&slug));
        Ok((slug, out))
    }

    pub fn list_games(&self) -> Vec<GameEntry> {
        let mut out = Vec::new();
        if let Ok(entries) = fs::read_dir(self.root.join("games")) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let Some(slug) = path.file_name().and_then(|s| s.to_str()).map(String::from) else {
                    continue;
                };
                let Ok(text) = fs::read_to_string(path.join("spec.json")) else { continue };
                let Ok(spec) = serde_json::from_str::<GameSpec>(&text) else { continue };
                out.push(self.describe_game(&slug, &spec));
            }
        }
        out.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        out
    }

    /// One game's row: existence facts straight off the disk.
    fn describe_game(&self, slug: &str, spec: &GameSpec) -> GameEntry {
        let html = self.game_html(slug);
        let built = html.exists();
        let last_build = html
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .map(|t| {
                let dt: chrono::DateTime<Local> = t.into();
                dt.format("%Y-%m-%d %H:%M").to_string()
            });
        let verification = fs::read_to_string(self.verify_path(slug))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .map(|v| verify_summary_line(&v));
        GameEntry {
            slug: slug.to_string(),
            title: spec.title.clone(),
            built,
            last_build,
            verification,
        }
    }

    /// Record a verification result beside the game. Refused when the game is gone.
    pub fn write_verify(&self, slug: &str, report: &Value) -> Result<(), String> {
        let path = self.verify_path(slug);
        let json = serde_json::to_string_pretty(report).map_err(|e| e.to_string())?;
        write_in_place(&path, json.as_bytes())
    }

    /// Record the verdict on the build whose stamp is `judged`. A verdict is about one file: if
    /// the game was built again, or removed, while it was being judged, it is dropped, and the
    /// new build is not shown as verified on the strength of the old one.
    /// What a finished run keeps: its screenshot and its verdict, each one atomic write, and only
    /// if the build is still the one it judged. Anything else (the build changed, the game is
    /// gone) keeps nothing, and the earlier files stand untouched.
    pub fn commit(&self, slug: &str, judged: &Stamp, shot: Option<Vec<u8>>, verdict: Option<&Value>) -> Result<(), String> {
        if &stamp_of(&self.game_html(slug)) != judged {
            return Err(format!(
                "{slug} was built again or removed while it was being checked, so that result was dropped; run it again"
            ));
        }
        if let Some(bytes) = shot {
            write_in_place(&self.screenshot_path(slug), &bytes)?;
        }
        match verdict {
            Some(report) => self.write_verify(slug, report),
            None => Ok(()),
        }
    }

    pub fn write_verdict(&self, slug: &str, report: &Value, judged: &Stamp) -> Result<(), String> {
        if &stamp_of(&self.game_html(slug)) != judged {
            return Err(format!(
                "{slug} was built again or removed while it was being checked, so that verdict is about a file that is gone; run it again"
            ));
        }
        self.write_verify(slug, report)
    }

}

/// One line out of a verify.json report, for the games list and `describe`. The
/// words are the verifier's own (`verify::summary_line`), so the row a person
/// reads days later says the same thing the job said when it landed — including
/// whether it was the game or the machine that decided it.
pub fn verify_summary_line(report: &Value) -> String {
    let when = report
        .get("when")
        .and_then(|v| v.as_str())
        .unwrap_or("at an unknown time");
    let gates: Vec<verify::Gate> = report
        .get("gates")
        .and_then(|v| v.as_array())
        .map(|g| g.iter().filter_map(verify::Gate::from_value).collect())
        .unwrap_or_default();
    if gates.is_empty() {
        return format!("verified {when}");
    }
    format!("verified {when}: {}", verify::summary_line(&gates))
}

/// A filename-safe slug from a title or name: lowercase, runs of anything else
/// collapsed to single dashes.
pub fn slugify(text: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in text.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

