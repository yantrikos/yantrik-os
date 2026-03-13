//! Presentation app wire module — screen 31.
//!
//! Full-featured slide deck editor with layouts, themes, speaker notes,
//! text formatting, file save/load, images, presenter timer, slide transitions,
//! templates, export, undo/redo, search, and keyboard shortcuts.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::bridge::CompanionBridge;
use crate::{AiProposalState, App, PresentTheme, SlideData};

// ─── Data Structures ────────────────────────────────────────────────

/// Text formatting state per slide.
#[derive(Clone, Debug, Default)]
struct SlideFormatting {
    bold: bool,
    italic: bool,
    font_size: i32,
    /// Index into TEXT_COLOR_PRESETS.
    text_color_idx: i32,
}

/// Text color presets.
const TEXT_COLOR_PRESETS: &[&str] = &[
    "#ffffff", // 0 = white
    "#000000", // 1 = black
    "#ff4444", // 2 = red
    "#44ff44", // 3 = green
    "#4488ff", // 4 = blue
    "#ffcc00", // 5 = yellow
    "#ff88ff", // 6 = magenta
    "#00cccc", // 7 = cyan
];

/// Transition types.
const TRANSITION_NAMES: &[&str] = &[
    "None",        // 0
    "Fade",        // 1
    "Slide Left",  // 2
    "Slide Right", // 3
    "Zoom",        // 4
];

/// A single slide.
#[derive(Clone, Debug)]
struct Slide {
    title: String,
    body: String,
    notes: String,
    layout: i32,
    image_path: Option<String>,
    formatting: SlideFormatting,
    transition: i32,
}

impl Slide {
    fn new(title: impl Into<String>, body: impl Into<String>, layout: i32) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            notes: String::new(),
            layout,
            image_path: None,
            formatting: SlideFormatting {
                font_size: 24,
                ..Default::default()
            },
            transition: 0,
        }
    }
}

// ── AI Proposal Types ──

/// A structured slide proposal from the AI.
#[derive(Clone, Debug, serde::Deserialize)]
struct AiSlideProposal {
    title: String,
    #[serde(default)]
    bullets: Vec<String>,
    #[serde(default)]
    speaker_notes: String,
    #[serde(default)]
    slide_type: String,
}

/// A full deck proposal from the AI.
#[derive(Clone, Debug, serde::Deserialize)]
struct AiDeckProposal {
    #[serde(default)]
    title: String,
    slides: Vec<AiSlideProposal>,
}

/// A split proposal from the AI.
#[derive(Clone, Debug, serde::Deserialize)]
struct AiSplitProposal {
    slides: Vec<AiSlideProposal>,
}

/// Pending proposal waiting for user to preview and selectively apply.
#[derive(Clone, Debug)]
struct PendingProposal {
    action_name: String,
    /// For deck proposals, all proposed slides. For single-slide, just one.
    proposed_slides: Vec<AiSlideProposal>,
    /// Original slide data for diff display (only for single-slide actions).
    original_slide: Option<Slide>,
    /// Which slide in proposed_slides is currently being previewed.
    preview_index: usize,
    /// Whether this is a deck-level proposal (replaces all slides).
    is_deck_proposal: bool,
}

/// Convert bullet points to body string (joined with newline, prefixed with bullet).
fn bullets_to_body(bullets: &[String]) -> String {
    if bullets.is_empty() {
        return String::new();
    }
    bullets.iter()
        .map(|b| {
            let trimmed = b.trim();
            if trimmed.starts_with('\u{2022}') || trimmed.starts_with('-') || trimmed.starts_with('*') {
                trimmed.to_string()
            } else {
                format!("\u{2022} {}", trimmed)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Convert body string to bullet points (split on newlines, strip prefixes).
fn body_to_bullets(body: &str) -> Vec<String> {
    body.lines()
        .map(|l| {
            let trimmed = l.trim();
            let stripped = trimmed.trim_start_matches('\u{2022}')
                .trim_start_matches('-')
                .trim_start_matches('*')
                .trim();
            stripped.to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// Normalize and validate an AI slide proposal.
fn normalize_proposal(p: &mut AiSlideProposal) {
    // Trim
    p.title = p.title.trim().to_string();
    p.speaker_notes = p.speaker_notes.trim().to_string();

    // Cap title
    if p.title.len() > 80 {
        p.title.truncate(80);
    }

    // Normalize bullets
    p.bullets = p.bullets.iter()
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty())
        .collect();

    // Cap bullets to 7
    p.bullets.truncate(7);

    // Cap each bullet to 120 chars
    for b in &mut p.bullets {
        if b.len() > 120 {
            b.truncate(120);
        }
    }

    // Normalize slide_type
    if p.slide_type.is_empty() {
        p.slide_type = "title_and_content".to_string();
    }
}

/// Parse a JSON response from the AI, extracting JSON from markdown code blocks if needed.
fn parse_ai_json<T: serde::de::DeserializeOwned>(response: &str) -> Option<T> {
    let trimmed = response.trim();

    // Try to extract JSON from ```json ... ``` blocks
    let json_str = if let Some(start) = trimmed.find("```json") {
        let after = &trimmed[start + 7..];
        if let Some(end) = after.find("```") {
            after[..end].trim()
        } else {
            after.trim()
        }
    } else if let Some(start) = trimmed.find("```") {
        let after = &trimmed[start + 3..];
        // Skip optional language tag on same line
        let after = if let Some(nl) = after.find('\n') {
            &after[nl + 1..]
        } else {
            after
        };
        if let Some(end) = after.find("```") {
            after[..end].trim()
        } else {
            after.trim()
        }
    } else if trimmed.starts_with('{') || trimmed.starts_with('[') {
        trimmed
    } else {
        // Try to find JSON object in the response
        if let Some(start) = trimmed.find('{') {
            if let Some(end) = trimmed.rfind('}') {
                &trimmed[start..=end]
            } else {
                return None;
            }
        } else {
            return None;
        }
    };

    serde_json::from_str(json_str).ok()
}

/// Map AI slide_type string to layout integer.
fn slide_type_to_layout(slide_type: &str) -> i32 {
    match slide_type {
        "title_slide" => 0,
        "title_and_content" => 1,
        "two_column" => 2,
        "section_header" => 3,
        "blank" => 4,
        "image_and_text" | "image_text" => 5,
        _ => 1,
    }
}

/// A snapshot of the slides vec for undo/redo.
#[derive(Clone, Debug)]
struct UndoSnapshot {
    slides: Vec<Slide>,
    current_index: usize,
}

/// Search state.
#[derive(Clone, Debug, Default)]
struct SearchState {
    query: String,
    /// Indices of slides that match, in order.
    matches: Vec<usize>,
    /// Position within `matches`.
    match_cursor: usize,
}

/// Full presentation state.
struct PresentationState {
    slides: Vec<Slide>,
    current_index: usize,
    current_theme: usize,
    is_presenting: bool,

    // File
    file_path: Option<PathBuf>,
    is_modified: bool,
    created_date: String,
    modified_date: String,

    // Presenter timer
    timer_running: bool,
    timer_elapsed_secs: u64,

    // Default layout for new slides
    default_layout: i32,

    // Custom themes appended after presets
    custom_themes: Vec<(String, String, String, String)>,

    // Undo / redo
    undo_stack: Vec<UndoSnapshot>,
    redo_stack: Vec<UndoSnapshot>,

    // Search
    search: SearchState,

    // Speaker notes per slide index (separate from slide notes for the panel)
    speaker_notes: HashMap<usize, String>,

    // AI proposal state
    pending_proposal: Option<PendingProposal>,
}

/// Layout names (indexed by layout id).
const LAYOUT_NAMES: &[&str] = &[
    "Title Slide",
    "Title + Content",
    "Two Column",
    "Section Header",
    "Blank",
    "Image + Text",
];

/// Theme presets: (name, bg, text, accent).
const THEME_PRESETS: &[(&str, &str, &str, &str)] = &[
    ("Dark", "#1a1a2e", "#ffffff", "#6c63ff"),
    ("Light", "#ffffff", "#1a1a2e", "#4a90d9"),
    ("Blue", "#0d1b2a", "#e0e0e0", "#00b4d8"),
    ("Green", "#0a2e0a", "#e0e0e0", "#2e9b47"),
];

const MAX_UNDO: usize = 30;

impl PresentationState {
    fn new() -> Self {
        let now = current_timestamp();
        Self {
            slides: vec![Slide::new("Title Slide", "Click to add subtitle", 0)],
            current_index: 0,
            current_theme: 0,
            is_presenting: false,
            file_path: None,
            is_modified: false,
            created_date: now.clone(),
            modified_date: now,
            timer_running: false,
            timer_elapsed_secs: 0,
            default_layout: 1,
            custom_themes: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            search: SearchState::default(),
            speaker_notes: HashMap::new(),
            pending_proposal: None,
        }
    }

    fn current_slide(&self) -> &Slide {
        &self.slides[self.current_index]
    }

    fn current_slide_mut(&mut self) -> &mut Slide {
        &mut self.slides[self.current_index]
    }

    /// Push an undo snapshot (call BEFORE mutation).
    fn push_undo(&mut self) {
        let snap = UndoSnapshot {
            slides: self.slides.clone(),
            current_index: self.current_index,
        };
        self.undo_stack.push(snap);
        if self.undo_stack.len() > MAX_UNDO {
            self.undo_stack.remove(0);
        }
        // Any new edit clears the redo stack.
        self.redo_stack.clear();
    }

    /// Mark as modified and update timestamp.
    fn mark_modified(&mut self) {
        self.is_modified = true;
        self.modified_date = current_timestamp();
    }

    fn presentation_title(&self) -> String {
        if let Some(first) = self.slides.first() {
            first.title.clone()
        } else {
            "Untitled".to_string()
        }
    }

    fn slide_progress(&self) -> String {
        format!("Slide {} of {}", self.current_index + 1, self.slides.len())
    }

    fn timer_text(&self) -> String {
        let mins = self.timer_elapsed_secs / 60;
        let secs = self.timer_elapsed_secs % 60;
        format!("{:02}:{:02}", mins, secs)
    }

    fn next_slide_title(&self) -> String {
        if self.current_index + 1 < self.slides.len() {
            self.slides[self.current_index + 1].title.clone()
        } else {
            String::new()
        }
    }

    fn next_slide_body(&self) -> String {
        if self.current_index + 1 < self.slides.len() {
            self.slides[self.current_index + 1].body.clone()
        } else {
            String::new()
        }
    }
}

// ─── Helpers ────────────────────────────────────────────────────────

fn current_timestamp() -> String {
    // Simple ISO-ish timestamp without chrono dependency.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Convert to rough date string (good enough for metadata).
    let days = secs / 86400;
    let years = 1970 + days / 365;
    let remaining_days = days % 365;
    let month = remaining_days / 30 + 1;
    let day = remaining_days % 30 + 1;
    format!("{}-{:02}-{:02}", years, month, day)
}

/// Parse hex color string to slint Color.
fn hex_to_color(hex: &str) -> slint::Color {
    let hex = hex.trim_start_matches('#');
    if hex.len() < 6 {
        return slint::Color::from_rgb_u8(0, 0, 0);
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
    slint::Color::from_rgb_u8(r, g, b)
}

/// Presentations directory.
fn presentations_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".local/share/yantrik/presentations")
}

/// Exports directory.
fn exports_dir() -> PathBuf {
    presentations_dir().join("exports")
}

/// Build the themes model for the UI (active theme at index 0).
fn build_themes_model(
    active_index: usize,
    custom_themes: &[(String, String, String, String)],
) -> ModelRc<PresentTheme> {
    let all_themes: Vec<(&str, &str, &str, &str)> = THEME_PRESETS
        .iter()
        .copied()
        .chain(
            custom_themes
                .iter()
                .map(|(n, b, t, a)| (n.as_str(), b.as_str(), t.as_str(), a.as_str())),
        )
        .collect();

    let clamped = active_index.min(all_themes.len().saturating_sub(1));
    let (name, bg, text, accent) = all_themes[clamped];
    let active = PresentTheme {
        name: SharedString::from(name),
        bg_color: slint::Brush::SolidColor(hex_to_color(bg)),
        text_color: slint::Brush::SolidColor(hex_to_color(text)),
        accent_color: slint::Brush::SolidColor(hex_to_color(accent)),
        is_active: true,
    };
    let mut themes = vec![active];
    for (i, (name, bg, text, accent)) in all_themes.iter().enumerate() {
        if i != clamped {
            themes.push(PresentTheme {
                name: SharedString::from(*name),
                bg_color: slint::Brush::SolidColor(hex_to_color(bg)),
                text_color: slint::Brush::SolidColor(hex_to_color(text)),
                accent_color: slint::Brush::SolidColor(hex_to_color(accent)),
                is_active: false,
            });
        }
    }
    ModelRc::new(VecModel::from(themes))
}

/// Build slides model from state.
fn build_slides_model(state: &PresentationState) -> ModelRc<SlideData> {
    let slides: Vec<SlideData> = state
        .slides
        .iter()
        .enumerate()
        .map(|(i, s)| SlideData {
            title: SharedString::from(&s.title),
            body: SharedString::from(&s.body),
            notes: SharedString::from(&s.notes),
            layout: s.layout,
            slide_number: (i + 1) as i32,
        })
        .collect();
    ModelRc::new(VecModel::from(slides))
}

/// Sync all slide data + current slide info to the UI.
/// Sync proposal state to Slint UI for preview.
fn sync_proposal_to_ui(ui: &App, state: &PresentationState) {
    if let Some(ref prop) = state.pending_proposal {
        ui.set_pres_ai_has_proposal(true);

        let preview_idx = prop.preview_index;
        if let Some(proposed) = prop.proposed_slides.get(preview_idx) {
            let proposed_body = bullets_to_body(&proposed.bullets);

            // Get original for diff comparison
            let (orig_title, orig_body, orig_notes) = if let Some(ref orig) = prop.original_slide {
                (orig.title.clone(), orig.body.clone(), orig.notes.clone())
            } else {
                (String::new(), String::new(), String::new())
            };

            let title_changed = proposed.title != orig_title;
            let body_changed = proposed_body != orig_body;
            let notes_changed = proposed.speaker_notes != orig_notes;

            ui.set_pres_ai_proposal(AiProposalState {
                action_name: prop.action_name.clone().into(),
                is_deck_proposal: prop.is_deck_proposal,
                slide_count: prop.proposed_slides.len() as i32,
                current_preview_index: preview_idx as i32,
                original_title: orig_title.into(),
                original_body: orig_body.into(),
                original_notes: orig_notes.into(),
                proposed_title: proposed.title.clone().into(),
                proposed_body: proposed_body.into(),
                proposed_notes: proposed.speaker_notes.clone().into(),
                proposed_slide_type: proposed.slide_type.clone().into(),
                title_changed,
                body_changed,
                notes_changed,
                slide_type_changed: false,
            });
        }
    } else {
        ui.set_pres_ai_has_proposal(false);
    }
}

fn sync_to_ui(ui: &App, state: &PresentationState) {
    ui.set_pres_slides(build_slides_model(state));
    ui.set_pres_slide_count(state.slides.len() as i32);
    ui.set_pres_current_slide_index(state.current_index as i32);

    // Current slide fields
    let slide = state.current_slide();
    ui.set_pres_current_title(SharedString::from(&slide.title));
    ui.set_pres_current_body(SharedString::from(&slide.body));
    ui.set_pres_current_notes(SharedString::from(&slide.notes));
    ui.set_pres_current_layout(slide.layout);
    ui.set_pres_is_presenting(state.is_presenting);

    // Image
    let image_str = slide.image_path.as_deref().unwrap_or("");
    ui.set_pres_current_image(SharedString::from(image_str));

    // Formatting
    ui.set_pres_current_bold(slide.formatting.bold);
    ui.set_pres_current_italic(slide.formatting.italic);
    ui.set_pres_current_font_size(slide.formatting.font_size);
    ui.set_pres_current_text_color(slide.formatting.text_color_idx);

    // Transition
    ui.set_pres_current_transition(slide.transition);

    // Themes
    ui.set_pres_themes(build_themes_model(state.current_theme, &state.custom_themes));

    // Presenter info
    ui.set_pres_timer_text(SharedString::from(state.timer_text()));
    ui.set_pres_next_title(SharedString::from(state.next_slide_title()));
    ui.set_pres_next_body(SharedString::from(state.next_slide_body()));
    ui.set_pres_slide_progress(SharedString::from(state.slide_progress()));
    ui.set_pres_presentation_title(SharedString::from(state.presentation_title()));

    // Metadata
    ui.set_pres_is_modified(state.is_modified);
    let fp = state
        .file_path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    ui.set_pres_file_path(SharedString::from(fp));

    // Search
    ui.set_pres_search_count(state.search.matches.len() as i32);

    // Speaker notes panel
    let speaker_note = state
        .speaker_notes
        .get(&state.current_index)
        .cloned()
        .unwrap_or_else(|| slide.notes.clone());
    ui.set_pres_current_speaker_note(SharedString::from(speaker_note));
}

// ─── JSON save/load ─────────────────────────────────────────────────

/// Minimal JSON serialisation (no serde dependency).
fn slides_to_json(state: &PresentationState) -> String {
    let mut out = String::from("{\n");
    let title = json_escape(&state.presentation_title());
    out.push_str(&format!("  \"title\": \"{}\",\n", title));
    out.push_str(&format!("  \"theme\": {},\n", state.current_theme));
    out.push_str(&format!(
        "  \"created\": \"{}\",\n",
        json_escape(&state.created_date)
    ));
    out.push_str(&format!(
        "  \"modified\": \"{}\",\n",
        json_escape(&state.modified_date)
    ));
    out.push_str("  \"slides\": [\n");
    for (i, slide) in state.slides.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"title\": \"{}\",\n",
            json_escape(&slide.title)
        ));
        out.push_str(&format!(
            "      \"body\": \"{}\",\n",
            json_escape(&slide.body)
        ));
        out.push_str(&format!(
            "      \"notes\": \"{}\",\n",
            json_escape(&slide.notes)
        ));
        out.push_str(&format!("      \"layout\": {},\n", slide.layout));
        out.push_str(&format!("      \"transition\": {},\n", slide.transition));
        let img = slide.image_path.as_deref().unwrap_or("");
        out.push_str(&format!(
            "      \"image_path\": \"{}\",\n",
            json_escape(img)
        ));
        out.push_str("      \"formatting\": {\n");
        out.push_str(&format!(
            "        \"bold\": {},\n",
            slide.formatting.bold
        ));
        out.push_str(&format!(
            "        \"italic\": {},\n",
            slide.formatting.italic
        ));
        out.push_str(&format!(
            "        \"font_size\": {},\n",
            slide.formatting.font_size
        ));
        out.push_str(&format!(
            "        \"text_color_idx\": {}\n",
            slide.formatting.text_color_idx
        ));
        out.push_str("      }\n");
        if i + 1 < state.slides.len() {
            out.push_str("    },\n");
        } else {
            out.push_str("    }\n");
        }
    }
    out.push_str("  ]\n");
    out.push('}');
    out
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Minimal JSON parsing for loading presentations.
fn load_presentation_from_json(json: &str) -> Option<PresentationState> {
    // Parse using a basic approach — find slide objects between array brackets.
    let theme = extract_json_int(json, "\"theme\"").unwrap_or(0) as usize;
    let created = extract_json_string(json, "\"created\"").unwrap_or_default();
    let modified = extract_json_string(json, "\"modified\"").unwrap_or_default();

    // Find slides array
    let slides_start = json.find("\"slides\"")?;
    let arr_start = json[slides_start..].find('[')? + slides_start;
    let arr_end = find_matching_bracket(json, arr_start)?;
    let slides_json = &json[arr_start + 1..arr_end];

    let mut slides = Vec::new();
    let mut pos = 0;
    while pos < slides_json.len() {
        if let Some(obj_start) = slides_json[pos..].find('{') {
            let abs_start = pos + obj_start;
            if let Some(obj_end) = find_matching_brace(slides_json, abs_start) {
                let obj = &slides_json[abs_start..=obj_end];
                let title = extract_json_string(obj, "\"title\"").unwrap_or_default();
                let body = extract_json_string(obj, "\"body\"").unwrap_or_default();
                let notes = extract_json_string(obj, "\"notes\"").unwrap_or_default();
                let layout = extract_json_int(obj, "\"layout\"").unwrap_or(1);
                let transition = extract_json_int(obj, "\"transition\"").unwrap_or(0);
                let image_path = extract_json_string(obj, "\"image_path\"");
                let image_path = image_path.filter(|s| !s.is_empty());

                // Parse formatting sub-object
                let bold = extract_json_bool(obj, "\"bold\"").unwrap_or(false);
                let italic = extract_json_bool(obj, "\"italic\"").unwrap_or(false);
                let font_size = extract_json_int(obj, "\"font_size\"").unwrap_or(24);
                let text_color_idx = extract_json_int(obj, "\"text_color_idx\"").unwrap_or(0);

                slides.push(Slide {
                    title,
                    body,
                    notes,
                    layout: layout as i32,
                    image_path,
                    formatting: SlideFormatting {
                        bold,
                        italic,
                        font_size: font_size as i32,
                        text_color_idx: text_color_idx as i32,
                    },
                    transition: transition as i32,
                });
                pos = obj_end + 1;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    if slides.is_empty() {
        return None;
    }

    Some(PresentationState {
        slides,
        current_index: 0,
        current_theme: theme.min(THEME_PRESETS.len() - 1),
        is_presenting: false,
        file_path: None,
        is_modified: false,
        created_date: created,
        modified_date: modified,
        timer_running: false,
        timer_elapsed_secs: 0,
        default_layout: 1,
        custom_themes: Vec::new(),
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
        search: SearchState::default(),
        speaker_notes: HashMap::new(),
        pending_proposal: None,
    })
}

fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let key_pos = json.find(key)?;
    let after_key = &json[key_pos + key.len()..];
    let colon = after_key.find(':')?;
    let after_colon = after_key[colon + 1..].trim_start();
    if !after_colon.starts_with('"') {
        return None;
    }
    let str_start = 1; // skip opening quote
    let mut result = String::new();
    let bytes = after_colon.as_bytes();
    let mut i = str_start;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'"' => {
                    result.push('"');
                    i += 2;
                }
                b'\\' => {
                    result.push('\\');
                    i += 2;
                }
                b'n' => {
                    result.push('\n');
                    i += 2;
                }
                b'r' => {
                    result.push('\r');
                    i += 2;
                }
                b't' => {
                    result.push('\t');
                    i += 2;
                }
                _ => {
                    result.push(bytes[i] as char);
                    i += 1;
                }
            }
        } else if bytes[i] == b'"' {
            break;
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    Some(result)
}

fn extract_json_int(json: &str, key: &str) -> Option<i64> {
    let key_pos = json.find(key)?;
    let after_key = &json[key_pos + key.len()..];
    let colon = after_key.find(':')?;
    let after_colon = after_key[colon + 1..].trim_start();
    // Read digits (and optional minus)
    let num_str: String = after_colon
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    num_str.parse().ok()
}

fn extract_json_bool(json: &str, key: &str) -> Option<bool> {
    let key_pos = json.find(key)?;
    let after_key = &json[key_pos + key.len()..];
    let colon = after_key.find(':')?;
    let after_colon = after_key[colon + 1..].trim_start();
    if after_colon.starts_with("true") {
        Some(true)
    } else if after_colon.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn find_matching_bracket(json: &str, open_pos: usize) -> Option<usize> {
    let mut depth = 0;
    let bytes = json.as_bytes();
    for i in open_pos..bytes.len() {
        match bytes[i] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn find_matching_brace(json: &str, open_pos: usize) -> Option<usize> {
    let mut depth = 0;
    let bytes = json.as_bytes();
    for i in open_pos..bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

// ─── Save / Load helpers ────────────────────────────────────────────

fn do_save(state: &mut PresentationState, path: &PathBuf) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = slides_to_json(state);
    match std::fs::write(path, &json) {
        Ok(()) => {
            state.file_path = Some(path.clone());
            state.is_modified = false;
            tracing::info!(path = %path.display(), "Presentation saved");
        }
        Err(e) => {
            tracing::error!(path = %path.display(), error = %e, "Failed to save presentation");
        }
    }
}

fn do_load_most_recent() -> Option<(PresentationState, PathBuf)> {
    let dir = presentations_dir();
    let entries = std::fs::read_dir(&dir).ok()?;
    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "ypres")
                .unwrap_or(false)
        })
        .collect();
    // Sort by modified time, most recent first.
    files.sort_by(|a, b| {
        let ta = a.metadata().and_then(|m| m.modified()).ok();
        let tb = b.metadata().and_then(|m| m.modified()).ok();
        tb.cmp(&ta)
    });
    let entry = files.first()?;
    let path = entry.path();
    let content = std::fs::read_to_string(&path).ok()?;
    let mut pstate = load_presentation_from_json(&content)?;
    pstate.file_path = Some(path.clone());
    Some((pstate, path))
}

// ─── Export helpers ─────────────────────────────────────────────────

fn export_markdown(state: &PresentationState) {
    let dir = exports_dir();
    let _ = std::fs::create_dir_all(&dir);

    let title = sanitise_filename(&state.presentation_title());
    let path = dir.join(format!("{}.md", title));

    let mut md = String::new();
    for (i, slide) in state.slides.iter().enumerate() {
        if i == 0 {
            md.push_str(&format!("# {}\n\n", slide.title));
        } else {
            md.push_str(&format!("## {}\n\n", slide.title));
        }
        if !slide.body.is_empty() {
            md.push_str(&slide.body);
            md.push_str("\n\n");
        }
        if !slide.notes.is_empty() {
            md.push_str(&format!("> **Notes:** {}\n\n", slide.notes));
        }
        md.push_str("---\n\n");
    }

    match std::fs::write(&path, &md) {
        Ok(()) => tracing::info!(path = %path.display(), "Exported markdown"),
        Err(e) => tracing::error!(error = %e, "Failed to export markdown"),
    }
}

fn export_outline(state: &PresentationState) {
    let dir = exports_dir();
    let _ = std::fs::create_dir_all(&dir);

    let title = sanitise_filename(&state.presentation_title());
    let path = dir.join(format!("{}_outline.txt", title));

    let mut out = String::new();
    out.push_str(&format!(
        "{}\n{}\n\n",
        state.presentation_title(),
        "=".repeat(state.presentation_title().len())
    ));
    for (i, slide) in state.slides.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", i + 1, slide.title));
        // Extract bullet points from body
        for line in slide.body.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                out.push_str(&format!("   - {}\n", trimmed));
            }
        }
        out.push('\n');
    }

    match std::fs::write(&path, &out) {
        Ok(()) => tracing::info!(path = %path.display(), "Exported outline"),
        Err(e) => tracing::error!(error = %e, "Failed to export outline"),
    }
}

fn sanitise_filename(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

// ─── Templates ──────────────────────────────────────────────────────

fn template_blank() -> Vec<Slide> {
    vec![Slide::new("Title Slide", "Click to add subtitle", 0)]
}

fn template_status_update() -> Vec<Slide> {
    vec![
        Slide::new("Status Update", "Team / Project Name\nDate", 0),
        Slide::new("Agenda", "1. Progress overview\n2. Key accomplishments\n3. Blockers\n4. Next steps", 1),
        Slide::new("Progress Overview", "• Milestone 1: Complete\n• Milestone 2: In progress\n• Milestone 3: Planned", 1),
        Slide::new("Key Accomplishments", "• Achievement 1\n• Achievement 2\n• Achievement 3", 1),
        Slide::new("Blockers & Risks", "• Blocker 1\n• Risk 1\n• Mitigation plan", 1),
        Slide::new("Summary & Next Steps", "• Action item 1\n• Action item 2\n• Timeline", 1),
    ]
}

fn template_project_proposal() -> Vec<Slide> {
    vec![
        Slide::new("Project Proposal", "Project Name\nPrepared by", 0),
        Slide::new("Problem Statement", "• What problem are we solving?\n• Who is affected?\n• Current impact", 1),
        Slide::new("Proposed Solution", "• Approach overview\n• Key components\n• Expected outcomes", 1),
        Slide::new("Timeline", "Phase 1: Research (2 weeks)\nPhase 2: Development (4 weeks)\nPhase 3: Testing (2 weeks)\nPhase 4: Launch (1 week)", 1),
        Slide::new("Budget", "• Development costs\n• Infrastructure\n• Ongoing maintenance\n• Total estimate", 1),
        Slide::new("Next Steps", "• Approval needed\n• Key decisions\n• Immediate actions", 1),
    ]
}

fn template_team_introduction() -> Vec<Slide> {
    vec![
        Slide::new("Meet the Team", "Department / Project", 0),
        Slide::new("Team Member 1", "Name\nRole\nBackground\nKey skills", 5),
        Slide::new("Team Member 2", "Name\nRole\nBackground\nKey skills", 5),
        Slide::new("Team Member 3", "Name\nRole\nBackground\nKey skills", 5),
        Slide::new("Team Member 4", "Name\nRole\nBackground\nKey skills", 5),
        Slide::new("Contact Information", "Email:\nSlack:\nMeeting hours:", 1),
    ]
}

fn template_quarterly_review() -> Vec<Slide> {
    vec![
        Slide::new("Quarterly Review", "Q_ 20__\nTeam / Department", 0),
        Slide::new("Highlights", "• Top achievement 1\n• Top achievement 2\n• Top achievement 3", 1),
        Slide::new("Key Metrics", "• Metric 1: value (change%)\n• Metric 2: value (change%)\n• Metric 3: value (change%)", 2),
        Slide::new("Challenges", "• Challenge 1\n• Challenge 2\n• Lessons learned", 1),
        Slide::new("Plan for Next Quarter", "• Goal 1\n• Goal 2\n• Goal 3\n• Resource needs", 1),
    ]
}

// ─── Notes generation ───────────────────────────────────────────────

fn generate_notes_for_slide(slide: &Slide) -> String {
    let mut notes = format!("This slide covers {}.", slide.title);
    let bullets: Vec<&str> = slide
        .body
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    if !bullets.is_empty() {
        notes.push_str(" Key points: ");
        let points: Vec<String> = bullets
            .iter()
            .map(|b| {
                b.trim_start_matches('•')
                    .trim_start_matches('-')
                    .trim_start_matches('*')
                    .trim()
                    .to_string()
            })
            .filter(|b| !b.is_empty())
            .collect();
        notes.push_str(&points.join("; "));
        notes.push('.');
    }
    notes
}

// ─── Search ─────────────────────────────────────────────────────────

fn run_search(state: &mut PresentationState) {
    let query_lower = state.search.query.to_lowercase();
    if query_lower.is_empty() {
        state.search.matches.clear();
        state.search.match_cursor = 0;
        return;
    }
    state.search.matches = state
        .slides
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.title.to_lowercase().contains(&query_lower)
                || s.body.to_lowercase().contains(&query_lower)
                || s.notes.to_lowercase().contains(&query_lower)
        })
        .map(|(i, _)| i)
        .collect();
    state.search.match_cursor = 0;
    // Jump to first match if any
    if let Some(&idx) = state.search.matches.first() {
        state.current_index = idx;
    }
}

// ─── Wire ───────────────────────────────────────────────────────────

/// Wire all presentation callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    let state = Rc::new(RefCell::new(PresentationState::new()));

    // Initial sync
    sync_to_ui(ui, &state.borrow());

    // ── Auto-save timer (every 60 seconds) ──
    {
        let autosave_timer = Timer::default();
        let state = state.clone();
        let ui_weak = ui.as_weak();
        autosave_timer.start(TimerMode::Repeated, Duration::from_secs(60), move || {
            let mut s = state.borrow_mut();
            if s.is_modified {
                let path = if let Some(ref p) = s.file_path {
                    p.clone()
                } else {
                    let dir = presentations_dir();
                    let title = sanitise_filename(&s.presentation_title());
                    let name = if title.is_empty() {
                        "untitled".to_string()
                    } else {
                        title
                    };
                    dir.join(format!("{}.ypres", name))
                };
                do_save(&mut s, &path);
                if let Some(ui) = ui_weak.upgrade() {
                    sync_to_ui(&ui, &s);
                }
                tracing::info!("Presentation auto-saved");
            }
        });
        // Keep the timer alive by leaking it (standard pattern in this codebase).
        std::mem::forget(autosave_timer);
    }

    // ── Presenter timer (ticks every second when running) ──
    {
        let presenter_timer = Timer::default();
        let state = state.clone();
        let ui_weak = ui.as_weak();
        presenter_timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
            let mut s = state.borrow_mut();
            if s.timer_running {
                s.timer_elapsed_secs += 1;
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_pres_timer_text(SharedString::from(s.timer_text()));
                }
            }
        });
        std::mem::forget(presenter_timer);
    }

    // ── Add slide (inserts after current) ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_add_slide(move || {
            let mut s = state.borrow_mut();
            s.push_undo();
            let layout = s.default_layout;
            let new_slide = Slide::new(
                format!("Slide {}", s.slides.len() + 1),
                "Click to add content",
                layout,
            );
            let insert_at = s.current_index + 1;
            s.slides.insert(insert_at, new_slide);
            s.current_index = insert_at;
            s.mark_modified();
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Delete slide ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_delete_slide(move || {
            let mut s = state.borrow_mut();
            if s.slides.len() <= 1 {
                return;
            }
            s.push_undo();
            let idx = s.current_index;
            s.slides.remove(idx);
            if s.current_index >= s.slides.len() {
                s.current_index = s.slides.len() - 1;
            }
            s.mark_modified();
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Duplicate slide ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_duplicate_slide(move || {
            let mut s = state.borrow_mut();
            s.push_undo();
            let dup = s.slides[s.current_index].clone();
            let insert_at = s.current_index + 1;
            s.slides.insert(insert_at, dup);
            s.current_index = insert_at;
            s.mark_modified();
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Move slide up ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_move_slide_up(move || {
            let mut s = state.borrow_mut();
            if s.current_index == 0 {
                return;
            }
            s.push_undo();
            let idx = s.current_index;
            s.slides.swap(idx, idx - 1);
            s.current_index = idx - 1;
            s.mark_modified();
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Move slide down ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_move_slide_down(move || {
            let mut s = state.borrow_mut();
            if s.current_index >= s.slides.len() - 1 {
                return;
            }
            s.push_undo();
            let idx = s.current_index;
            s.slides.swap(idx, idx + 1);
            s.current_index = idx + 1;
            s.mark_modified();
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Select slide ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_select_slide(move |idx| {
            let mut s = state.borrow_mut();
            let idx = idx as usize;
            if idx < s.slides.len() {
                s.current_index = idx;
                if let Some(ui) = ui_weak.upgrade() {
                    sync_to_ui(&ui, &s);
                }
            }
        });
    }

    // ── Set layout ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_set_layout(move |layout| {
            let mut s = state.borrow_mut();
            let layout = layout.clamp(0, LAYOUT_NAMES.len() as i32 - 1);
            s.push_undo();
            s.current_slide_mut().layout = layout;
            s.mark_modified();
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Set theme (cycles to next) ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_set_theme(move |_idx| {
            let mut s = state.borrow_mut();
            let total = THEME_PRESETS.len() + s.custom_themes.len();
            s.current_theme = (s.current_theme + 1) % total;
            tracing::debug!(theme_index = s.current_theme, "Theme changed");
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Toggle present mode ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_toggle_present(move || {
            let mut s = state.borrow_mut();
            s.is_presenting = !s.is_presenting;
            tracing::info!(presenting = s.is_presenting, "Presenter mode toggled");
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Export PDF (stub) ──
    {
        ui.on_pres_export_pdf(move || {
            tracing::info!("Export PDF requested (stub — not yet implemented)");
        });
    }

    // ── Title edited ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_title_edited(move |text| {
            let mut s = state.borrow_mut();
            s.current_slide_mut().title = text.to_string();
            s.mark_modified();
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_pres_slides(build_slides_model(&s));
                ui.set_pres_presentation_title(SharedString::from(s.presentation_title()));
                ui.set_pres_slide_progress(SharedString::from(s.slide_progress()));
            }
        });
    }

    // ── Body edited ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_body_edited(move |text| {
            let mut s = state.borrow_mut();
            s.current_slide_mut().body = text.to_string();
            s.mark_modified();
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_pres_slides(build_slides_model(&s));
            }
        });
    }

    // ── Notes edited ──
    {
        let state = state.clone();
        ui.on_pres_notes_edited(move |text| {
            let mut s = state.borrow_mut();
            s.current_slide_mut().notes = text.to_string();
            s.mark_modified();
        });
    }

    // ── Next slide ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_next_slide(move || {
            let mut s = state.borrow_mut();
            if s.current_index < s.slides.len() - 1 {
                s.current_index += 1;
                if let Some(ui) = ui_weak.upgrade() {
                    sync_to_ui(&ui, &s);
                }
            }
        });
    }

    // ── Previous slide ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_prev_slide(move || {
            let mut s = state.borrow_mut();
            if s.current_index > 0 {
                s.current_index -= 1;
                if let Some(ui) = ui_weak.upgrade() {
                    sync_to_ui(&ui, &s);
                }
            }
        });
    }

    // ═══════════════════════════════════════════════════════════════
    // NEW FEATURES
    // ═══════════════════════════════════════════════════════════════

    // ── Text Formatting: Bold ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_format_bold(move || {
            let mut s = state.borrow_mut();
            s.push_undo();
            let slide = s.current_slide_mut();
            slide.formatting.bold = !slide.formatting.bold;
            s.mark_modified();
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Text Formatting: Italic ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_format_italic(move || {
            let mut s = state.borrow_mut();
            s.push_undo();
            let slide = s.current_slide_mut();
            slide.formatting.italic = !slide.formatting.italic;
            s.mark_modified();
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Text Formatting: Font Size ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_set_font_size(move |size| {
            let mut s = state.borrow_mut();
            s.push_undo();
            let clamped = size.clamp(8, 120);
            s.current_slide_mut().formatting.font_size = clamped;
            s.mark_modified();
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Text Formatting: Text Color ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_set_text_color(move |color_idx| {
            let mut s = state.borrow_mut();
            s.push_undo();
            let clamped = color_idx.clamp(0, TEXT_COLOR_PRESETS.len() as i32 - 1);
            s.current_slide_mut().formatting.text_color_idx = clamped;
            s.mark_modified();
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── File Save ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_save(move || {
            let mut s = state.borrow_mut();
            let path = if let Some(ref p) = s.file_path {
                p.clone()
            } else {
                let dir = presentations_dir();
                let title = sanitise_filename(&s.presentation_title());
                let name = if title.is_empty() {
                    "untitled".to_string()
                } else {
                    title
                };
                dir.join(format!("{}.ypres", name))
            };
            do_save(&mut s, &path);
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── File Load (most recent) ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_load(move || {
            if let Some((loaded, path)) = do_load_most_recent() {
                let mut s = state.borrow_mut();
                s.slides = loaded.slides;
                s.current_index = 0;
                s.current_theme = loaded.current_theme;
                s.is_presenting = false;
                s.file_path = Some(path.clone());
                s.is_modified = false;
                s.created_date = loaded.created_date;
                s.modified_date = loaded.modified_date;
                s.undo_stack.clear();
                s.redo_stack.clear();
                s.search = SearchState::default();
                tracing::info!(path = %path.display(), "Presentation loaded");
                if let Some(ui) = ui_weak.upgrade() {
                    sync_to_ui(&ui, &s);
                }
            } else {
                tracing::info!("No .ypres files found to load");
            }
        });
    }

    // ── File Save As ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_save_as(move |name| {
            let mut s = state.borrow_mut();
            let dir = presentations_dir();
            let sanitised = sanitise_filename(&name.to_string());
            let fname = if sanitised.is_empty() {
                "untitled".to_string()
            } else {
                sanitised
            };
            let path = dir.join(format!("{}.ypres", fname));
            do_save(&mut s, &path);
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Insert Image ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_insert_image(move |path| {
            let mut s = state.borrow_mut();
            s.push_undo();
            let path_str = path.to_string();
            s.current_slide_mut().image_path = if path_str.is_empty() {
                None
            } else {
                Some(path_str)
            };
            s.mark_modified();
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Presenter Timer: Toggle ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_toggle_timer(move || {
            let mut s = state.borrow_mut();
            s.timer_running = !s.timer_running;
            tracing::info!(running = s.timer_running, "Presenter timer toggled");
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Presenter Timer: Reset ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_reset_timer(move || {
            let mut s = state.borrow_mut();
            s.timer_elapsed_secs = 0;
            s.timer_running = false;
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Generate Notes ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_generate_notes(move || {
            let mut s = state.borrow_mut();
            s.push_undo();
            // Extract title and body before mutating notes.
            let title = s.current_slide().title.clone();
            let body = s.current_slide().body.clone();
            let tmp_slide = Slide {
                title,
                body,
                notes: String::new(),
                layout: 0,
                image_path: None,
                formatting: SlideFormatting::default(),
                transition: 0,
            };
            let notes = generate_notes_for_slide(&tmp_slide);
            s.current_slide_mut().notes = notes;
            s.mark_modified();
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Insert Object (placeholder — logs the object type) ──
    {
        ui.on_pres_insert_object(move |kind| {
            tracing::info!("Insert object requested: {}", kind);
            // TODO: implement object insertion for kind: text, image, shape, chart, table, icon
        });
    }

    // ── Keyboard Shortcuts ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_key_pressed(move |key| {
            let key_str = key.to_string();
            let mut s = state.borrow_mut();
            let is_presenting = s.is_presenting;

            match key_str.as_str() {
                "Right" | "Down" | " " | "Space" | "PageDown" => {
                    if is_presenting && s.current_index < s.slides.len() - 1 {
                        s.current_index += 1;
                        if let Some(ui) = ui_weak.upgrade() {
                            sync_to_ui(&ui, &s);
                        }
                    }
                }
                "Left" | "Up" | "PageUp" => {
                    if is_presenting && s.current_index > 0 {
                        s.current_index -= 1;
                        if let Some(ui) = ui_weak.upgrade() {
                            sync_to_ui(&ui, &s);
                        }
                    }
                }
                "Escape" => {
                    if is_presenting {
                        s.is_presenting = false;
                        tracing::info!("Exited presenter mode via Escape");
                        if let Some(ui) = ui_weak.upgrade() {
                            sync_to_ui(&ui, &s);
                        }
                    }
                }
                "Home" => {
                    if is_presenting && !s.slides.is_empty() {
                        s.current_index = 0;
                        if let Some(ui) = ui_weak.upgrade() {
                            sync_to_ui(&ui, &s);
                        }
                    }
                }
                "End" => {
                    if is_presenting && !s.slides.is_empty() {
                        s.current_index = s.slides.len() - 1;
                        if let Some(ui) = ui_weak.upgrade() {
                            sync_to_ui(&ui, &s);
                        }
                    }
                }
                "n" | "N" => {
                    if !is_presenting {
                        // New slide in edit mode
                        s.push_undo();
                        let layout = s.default_layout;
                        let new_slide = Slide::new(
                            format!("Slide {}", s.slides.len() + 1),
                            "Click to add content",
                            layout,
                        );
                        let insert_at = s.current_index + 1;
                        s.slides.insert(insert_at, new_slide);
                        s.current_index = insert_at;
                        s.mark_modified();
                        if let Some(ui) = ui_weak.upgrade() {
                            sync_to_ui(&ui, &s);
                        }
                    }
                }
                "Delete" => {
                    if !is_presenting && s.slides.len() > 1 {
                        s.push_undo();
                        let idx = s.current_index;
                        s.slides.remove(idx);
                        if s.current_index >= s.slides.len() {
                            s.current_index = s.slides.len() - 1;
                        }
                        s.mark_modified();
                        if let Some(ui) = ui_weak.upgrade() {
                            sync_to_ui(&ui, &s);
                        }
                    }
                }
                _ => {}
            }
        });
    }

    // ── Set Transition ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_set_transition(move |transition_type| {
            let mut s = state.borrow_mut();
            let clamped = transition_type.clamp(0, TRANSITION_NAMES.len() as i32 - 1);
            s.push_undo();
            s.current_slide_mut().transition = clamped;
            s.mark_modified();
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Use Template ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_use_template(move |template_idx| {
            let mut s = state.borrow_mut();
            s.push_undo();
            let slides = match template_idx {
                0 => template_blank(),
                1 => template_status_update(),
                2 => template_project_proposal(),
                3 => template_team_introduction(),
                4 => template_quarterly_review(),
                _ => template_blank(),
            };
            s.slides = slides;
            s.current_index = 0;
            s.mark_modified();
            tracing::info!(template = template_idx, "Applied deck template");
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Export Markdown ──
    {
        let state = state.clone();
        ui.on_pres_export_markdown(move || {
            let s = state.borrow();
            export_markdown(&s);
        });
    }

    // ── Export Outline ──
    {
        let state = state.clone();
        ui.on_pres_export_outline(move || {
            let s = state.borrow();
            export_outline(&s);
        });
    }

    // ── Undo ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_undo(move || {
            let mut s = state.borrow_mut();
            if let Some(snap) = s.undo_stack.pop() {
                // Save current state to redo stack.
                let redo_snap = UndoSnapshot {
                    slides: s.slides.clone(),
                    current_index: s.current_index,
                };
                s.redo_stack.push(redo_snap);
                s.slides = snap.slides;
                s.current_index = snap.current_index.min(s.slides.len().saturating_sub(1));
                s.mark_modified();
                if let Some(ui) = ui_weak.upgrade() {
                    sync_to_ui(&ui, &s);
                }
            }
        });
    }

    // ── Redo ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_redo(move || {
            let mut s = state.borrow_mut();
            if let Some(snap) = s.redo_stack.pop() {
                let undo_snap = UndoSnapshot {
                    slides: s.slides.clone(),
                    current_index: s.current_index,
                };
                s.undo_stack.push(undo_snap);
                s.slides = snap.slides;
                s.current_index = snap.current_index.min(s.slides.len().saturating_sub(1));
                s.mark_modified();
                if let Some(ui) = ui_weak.upgrade() {
                    sync_to_ui(&ui, &s);
                }
            }
        });
    }

    // ── Search ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_search(move |query| {
            let mut s = state.borrow_mut();
            s.search.query = query.to_string();
            run_search(&mut s);
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Search Next ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_search_next(move || {
            let mut s = state.borrow_mut();
            if s.search.matches.is_empty() {
                return;
            }
            s.search.match_cursor = (s.search.match_cursor + 1) % s.search.matches.len();
            let idx = s.search.matches[s.search.match_cursor];
            s.current_index = idx;
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Custom Theme ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_custom_theme(move |bg, text, accent| {
            let mut s = state.borrow_mut();
            let name = format!("Custom {}", s.custom_themes.len() + 1);
            s.custom_themes
                .push((name, bg.to_string(), text.to_string(), accent.to_string()));
            // Switch to the newly added custom theme.
            s.current_theme = THEME_PRESETS.len() + s.custom_themes.len() - 1;
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Set Default Layout ──
    {
        let state = state.clone();
        ui.on_pres_set_default_layout(move |layout| {
            let mut s = state.borrow_mut();
            s.default_layout = layout.clamp(0, LAYOUT_NAMES.len() as i32 - 1);
            tracing::info!(layout = s.default_layout, "Default layout updated");
        });
    }

    // ── AI Assist ──
    wire_ai(ui, ctx, state);
}

/// Fire an AI action: send prompt, accumulate response, call on_complete when done.
fn fire_ai_action(
    ui_weak: &slint::Weak<App>,
    bridge: &Arc<CompanionBridge>,
    ai_state: &Rc<RefCell<super::ai_assist::AiAssistState>>,
    prompt: String,
    action_name: &str,
    on_complete: impl Fn(String) + 'static,
) {
    // Cancel any in-progress request
    {
        let mut s = ai_state.borrow_mut();
        s.timer = None;
        s.is_working = true;
    }

    // Set status
    if let Some(ui) = ui_weak.upgrade() {
        ui.set_pres_ai_working(true);
        ui.set_pres_ai_status(format!("{}...", action_name).into());
        ui.set_pres_ai_has_proposal(false);
    }

    let token_rx = bridge.send_message(prompt);
    let weak = ui_weak.clone();
    let accumulated = Rc::new(RefCell::new(String::new()));
    let acc2 = accumulated.clone();
    let state_clone = ai_state.clone();
    let start_time = std::time::Instant::now();
    let timeout = Duration::from_secs(90);

    let timer = Timer::default();
    let token_count = Rc::new(std::cell::Cell::new(0u32));
    let tc2 = token_count.clone();
    let action_label = action_name.to_string();
    timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
        let mut done = false;
        while let Ok(token) = token_rx.try_recv() {
            if token == "__DONE__" {
                done = true;
                break;
            }
            // __REPLACE__ means "discard everything so far" — tool calls finished,
            // next tokens are the final response.
            if token == "__REPLACE__" {
                acc2.borrow_mut().clear();
                tc2.set(0);
                continue;
            }
            if token.starts_with("__") && token.ends_with("__") {
                continue;
            }
            acc2.borrow_mut().push_str(&token);
            tc2.set(tc2.get() + 1);
        }

        // Update status with token count for progress feedback
        if !done && tc2.get() > 0 {
            if let Some(ui) = weak.upgrade() {
                ui.set_pres_ai_status(
                    format!("{}... ({} tokens)", action_label, tc2.get()).into(),
                );
            }
        }

        // Timeout check
        if !done && start_time.elapsed() > timeout {
            if let Some(ui) = weak.upgrade() {
                ui.set_pres_ai_working(false);
                ui.set_pres_ai_status("AI timed out. Try again.".into());
            }
            state_clone.borrow_mut().timer = None;
            state_clone.borrow_mut().is_working = false;
            return;
        }

        if done {
            if let Some(ui) = weak.upgrade() {
                ui.set_pres_ai_working(false);
                ui.set_pres_ai_status("".into());
            }
            state_clone.borrow_mut().timer = None;
            state_clone.borrow_mut().is_working = false;
            let response = accumulated.borrow().clone();
            on_complete(response);
        }
    });

    ai_state.borrow_mut().timer = Some(timer);
}

/// Wire AI assist callbacks for yPresent.
fn wire_ai(ui: &App, ctx: &AppContext, state: Rc<RefCell<PresentationState>>) {
    let bridge = ctx.bridge.clone();
    let ai_state = super::ai_assist::AiAssistState::new();

    // ── Generate Deck ──
    {
        let st = state.clone();
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_ai_generate_deck(move |topic| {
            let instruction = if let Some(ui) = ui_weak.upgrade() {
                ui.get_pres_ai_instruction().to_string()
            } else { return; };
            let topic_str = topic.to_string();
            let prompt = if topic_str.trim().is_empty() {
                if instruction.trim().is_empty() { return; }
                super::ai_assist::pres_generate_deck_json_prompt(&instruction, "")
            } else {
                super::ai_assist::pres_generate_deck_json_prompt(&topic_str, &instruction)
            };

            let st2 = st.clone();
            let weak2 = ui_weak.clone();
            fire_ai_action(&ui_weak, &bridge, &ai_st, prompt, "Generate Deck", move |response| {
                tracing::info!(len = response.len(), "Generate Deck AI response received");
                if let Some(deck) = parse_ai_json::<AiDeckProposal>(&response) {
                    let mut slides = deck.slides;
                    for s in &mut slides { normalize_proposal(s); }
                    tracing::info!(slide_count = slides.len(), "Parsed deck proposal");
                    if !slides.is_empty() {
                        let mut s = st2.borrow_mut();
                        s.pending_proposal = Some(PendingProposal {
                            action_name: "Generate Deck".into(),
                            proposed_slides: slides,
                            original_slide: None,
                            preview_index: 0,
                            is_deck_proposal: true,
                        });
                        if let Some(ui) = weak2.upgrade() {
                            sync_proposal_to_ui(&ui, &s);
                        }
                    }
                } else {
                    tracing::warn!(
                        response_preview = %response.chars().take(500).collect::<String>(),
                        "Failed to parse AI deck response as JSON"
                    );
                    if let Some(ui) = weak2.upgrade() {
                        ui.set_pres_ai_status("AI response was not valid JSON. Try again.".into());
                    }
                }
            });
        });
    }

    // ── Structure Text ──
    {
        let st = state.clone();
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_ai_structure_text(move |text| {
            let instruction = if let Some(ui) = ui_weak.upgrade() {
                ui.get_pres_ai_instruction().to_string()
            } else { return; };
            let text_str = text.to_string();
            if text_str.trim().is_empty() && instruction.trim().is_empty() { return; }
            let source = if text_str.trim().is_empty() { instruction.clone() } else { text_str };
            let prompt = super::ai_assist::pres_structure_text_json_prompt(&source, &instruction);

            let st2 = st.clone();
            let weak2 = ui_weak.clone();
            fire_ai_action(&ui_weak, &bridge, &ai_st, prompt, "Structure Text", move |response| {
                if let Some(deck) = parse_ai_json::<AiDeckProposal>(&response) {
                    let mut slides = deck.slides;
                    for s in &mut slides { normalize_proposal(s); }
                    if !slides.is_empty() {
                        let mut s = st2.borrow_mut();
                        s.pending_proposal = Some(PendingProposal {
                            action_name: "Structure Text".into(),
                            proposed_slides: slides,
                            original_slide: None,
                            preview_index: 0,
                            is_deck_proposal: true,
                        });
                        if let Some(ui) = weak2.upgrade() {
                            sync_proposal_to_ui(&ui, &s);
                        }
                    }
                } else {
                    tracing::warn!("Failed to parse AI structure-text response as JSON");
                    if let Some(ui) = weak2.upgrade() {
                        ui.set_pres_ai_status("AI response was not valid JSON. Try again.".into());
                    }
                }
            });
        });
    }

    // ── Generate All Notes ──
    {
        let st = state.clone();
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_ai_generate_all_notes(move || {
            let s = st.borrow();
            let slides_data: Vec<(String, Vec<String>)> = s.slides.iter()
                .map(|sl| (sl.title.clone(), body_to_bullets(&sl.body)))
                .collect();
            drop(s);
            let prompt = super::ai_assist::pres_generate_all_notes_json_prompt(&slides_data);

            let st2 = st.clone();
            let weak2 = ui_weak.clone();
            fire_ai_action(&ui_weak, &bridge, &ai_st, prompt, "Generate All Notes", move |response| {
                if let Some(split) = parse_ai_json::<AiSplitProposal>(&response) {
                    let mut slides = split.slides;
                    for s in &mut slides { normalize_proposal(s); }
                    if !slides.is_empty() {
                        let mut s = st2.borrow_mut();
                        s.pending_proposal = Some(PendingProposal {
                            action_name: "Generate All Notes".into(),
                            proposed_slides: slides,
                            original_slide: None,
                            preview_index: 0,
                            is_deck_proposal: true,
                        });
                        if let Some(ui) = weak2.upgrade() {
                            sync_proposal_to_ui(&ui, &s);
                        }
                    }
                } else {
                    tracing::warn!("Failed to parse AI all-notes response as JSON");
                    if let Some(ui) = weak2.upgrade() {
                        ui.set_pres_ai_status("AI response was not valid JSON. Try again.".into());
                    }
                }
            });
        });
    }

    // ── Improve Slide ──
    {
        let st = state.clone();
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_ai_improve_slide(move || {
            let instruction = if let Some(ui) = ui_weak.upgrade() {
                ui.get_pres_ai_instruction().to_string()
            } else { return; };
            let s = st.borrow();
            let slide = s.slides[s.current_index].clone();
            let deck_title = s.slides.first().map(|sl| sl.title.as_str()).unwrap_or("Untitled");
            let bullets = body_to_bullets(&slide.body);
            let prompt = super::ai_assist::pres_improve_slide_json_prompt(
                &slide.title, &bullets, &slide.notes, deck_title, &instruction,
            );
            drop(s);

            let st2 = st.clone();
            let weak2 = ui_weak.clone();
            let orig = slide.clone();
            fire_ai_action(&ui_weak, &bridge, &ai_st, prompt, "Improve Slide", move |response| {
                if let Some(mut proposed) = parse_ai_json::<AiSlideProposal>(&response) {
                    normalize_proposal(&mut proposed);
                    let mut s = st2.borrow_mut();
                    s.pending_proposal = Some(PendingProposal {
                        action_name: "Improve Slide".into(),
                        proposed_slides: vec![proposed],
                        original_slide: Some(orig.clone()),
                        preview_index: 0,
                        is_deck_proposal: false,
                    });
                    if let Some(ui) = weak2.upgrade() {
                        sync_proposal_to_ui(&ui, &s);
                    }
                } else {
                    tracing::warn!("Failed to parse AI improve-slide response as JSON");
                    if let Some(ui) = weak2.upgrade() {
                        ui.set_pres_ai_status("AI response was not valid JSON. Try again.".into());
                    }
                }
            });
        });
    }

    // ── Generate Notes (single slide) ──
    {
        let st = state.clone();
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_ai_generate_notes(move || {
            let s = st.borrow();
            let slide = s.slides[s.current_index].clone();
            let deck_title = s.slides.first().map(|sl| sl.title.as_str()).unwrap_or("Untitled");
            let bullets = body_to_bullets(&slide.body);
            let prompt = super::ai_assist::pres_generate_notes_json_prompt(
                &slide.title, &bullets, deck_title,
            );
            drop(s);

            let st2 = st.clone();
            let weak2 = ui_weak.clone();
            let orig = slide.clone();
            fire_ai_action(&ui_weak, &bridge, &ai_st, prompt, "Generate Notes", move |response| {
                if let Some(mut proposed) = parse_ai_json::<AiSlideProposal>(&response) {
                    normalize_proposal(&mut proposed);
                    let mut s = st2.borrow_mut();
                    s.pending_proposal = Some(PendingProposal {
                        action_name: "Generate Notes".into(),
                        proposed_slides: vec![proposed],
                        original_slide: Some(orig.clone()),
                        preview_index: 0,
                        is_deck_proposal: false,
                    });
                    if let Some(ui) = weak2.upgrade() {
                        sync_proposal_to_ui(&ui, &s);
                    }
                } else {
                    tracing::warn!("Failed to parse AI generate-notes response as JSON");
                    if let Some(ui) = weak2.upgrade() {
                        ui.set_pres_ai_status("AI response was not valid JSON. Try again.".into());
                    }
                }
            });
        });
    }

    // ── Simplify ──
    {
        let st = state.clone();
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_ai_simplify(move |audience| {
            let s = st.borrow();
            let slide = s.slides[s.current_index].clone();
            let bullets = body_to_bullets(&slide.body);
            let audience_str = audience.to_string();
            let prompt = super::ai_assist::pres_simplify_json_prompt(
                &slide.title, &bullets, &slide.notes, &audience_str,
            );
            drop(s);

            let st2 = st.clone();
            let weak2 = ui_weak.clone();
            let orig = slide.clone();
            fire_ai_action(&ui_weak, &bridge, &ai_st, prompt, "Simplify", move |response| {
                if let Some(mut proposed) = parse_ai_json::<AiSlideProposal>(&response) {
                    normalize_proposal(&mut proposed);
                    let mut s = st2.borrow_mut();
                    s.pending_proposal = Some(PendingProposal {
                        action_name: "Simplify".into(),
                        proposed_slides: vec![proposed],
                        original_slide: Some(orig.clone()),
                        preview_index: 0,
                        is_deck_proposal: false,
                    });
                    if let Some(ui) = weak2.upgrade() {
                        sync_proposal_to_ui(&ui, &s);
                    }
                } else {
                    tracing::warn!("Failed to parse AI simplify response as JSON");
                    if let Some(ui) = weak2.upgrade() {
                        ui.set_pres_ai_status("AI response was not valid JSON. Try again.".into());
                    }
                }
            });
        });
    }

    // ── Split Slide ──
    {
        let st = state.clone();
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_ai_split_slide(move || {
            let s = st.borrow();
            let slide = s.slides[s.current_index].clone();
            let bullets = body_to_bullets(&slide.body);
            let prompt = super::ai_assist::pres_split_slide_json_prompt(
                &slide.title, &bullets, &slide.notes,
            );
            drop(s);

            let st2 = st.clone();
            let weak2 = ui_weak.clone();
            let orig = slide.clone();
            fire_ai_action(&ui_weak, &bridge, &ai_st, prompt, "Split Slide", move |response| {
                if let Some(split) = parse_ai_json::<AiSplitProposal>(&response) {
                    let mut slides = split.slides;
                    for s in &mut slides { normalize_proposal(s); }
                    if slides.len() >= 2 {
                        let mut s = st2.borrow_mut();
                        s.pending_proposal = Some(PendingProposal {
                            action_name: "Split Slide".into(),
                            proposed_slides: slides,
                            original_slide: Some(orig.clone()),
                            preview_index: 0,
                            is_deck_proposal: false,
                        });
                        if let Some(ui) = weak2.upgrade() {
                            sync_proposal_to_ui(&ui, &s);
                        }
                    }
                } else {
                    tracing::warn!("Failed to parse AI split-slide response as JSON");
                    if let Some(ui) = weak2.upgrade() {
                        ui.set_pres_ai_status("AI response was not valid JSON. Try again.".into());
                    }
                }
            });
        });
    }

    // ── Suggest Layout ──
    {
        let st = state.clone();
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_ai_suggest_layout(move || {
            let s = st.borrow();
            let slide = s.slides[s.current_index].clone();
            let bullets = body_to_bullets(&slide.body);
            let prompt = super::ai_assist::pres_suggest_layout_json_prompt(
                &slide.title, &bullets, &slide.notes,
            );
            drop(s);

            let st2 = st.clone();
            let weak2 = ui_weak.clone();
            let orig = slide.clone();
            fire_ai_action(&ui_weak, &bridge, &ai_st, prompt, "Suggest Layout", move |response| {
                if let Some(mut proposed) = parse_ai_json::<AiSlideProposal>(&response) {
                    normalize_proposal(&mut proposed);
                    let mut s = st2.borrow_mut();
                    s.pending_proposal = Some(PendingProposal {
                        action_name: "Suggest Layout".into(),
                        proposed_slides: vec![proposed],
                        original_slide: Some(orig.clone()),
                        preview_index: 0,
                        is_deck_proposal: false,
                    });
                    if let Some(ui) = weak2.upgrade() {
                        sync_proposal_to_ui(&ui, &s);
                    }
                } else {
                    tracing::warn!("Failed to parse AI suggest-layout response as JSON");
                    if let Some(ui) = weak2.upgrade() {
                        ui.set_pres_ai_status("AI response was not valid JSON. Try again.".into());
                    }
                }
            });
        });
    }

    // ── Freeform ──
    {
        let st = state.clone();
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_ai_freeform(move |prompt_input| {
            let prompt_str = prompt_input.to_string();
            if prompt_str.trim().is_empty() { return; }
            let s = st.borrow();
            let slide = s.slides[s.current_index].clone();
            let deck_title = s.slides.first().map(|sl| sl.title.as_str()).unwrap_or("Untitled");
            let bullets = body_to_bullets(&slide.body);
            let bullets_json: Vec<String> = bullets.iter().map(|b| format!("\"{}\"", b.replace('"', "\\\""))).collect();
            let context_prompt = format!(
                r#"You are a presentation assistant. The user is editing a slide deck.

Deck: {}
Current slide:
{{
  "title": "{}",
  "bullets": [{}],
  "speaker_notes": "{}"
}}

User request: {}

If the request is about modifying the slide, return ONLY valid JSON:
{{
  "title": "updated title",
  "bullets": ["updated bullet 1", "updated bullet 2"],
  "speaker_notes": "updated notes",
  "slide_type": "title_and_content"
}}

If the request is a question (not a modification), respond with plain text."#,
                deck_title,
                slide.title.replace('"', "\\\""),
                bullets_json.join(", "),
                slide.notes.replace('"', "\\\""),
                prompt_str,
            );
            drop(s);

            let st2 = st.clone();
            let weak2 = ui_weak.clone();
            let orig = slide.clone();
            fire_ai_action(&ui_weak, &bridge, &ai_st, context_prompt, "AI Assist", move |response| {
                // Try to parse as structured slide proposal
                if let Some(mut proposed) = parse_ai_json::<AiSlideProposal>(&response) {
                    normalize_proposal(&mut proposed);
                    let mut s = st2.borrow_mut();
                    s.pending_proposal = Some(PendingProposal {
                        action_name: "AI Assist".into(),
                        proposed_slides: vec![proposed],
                        original_slide: Some(orig.clone()),
                        preview_index: 0,
                        is_deck_proposal: false,
                    });
                    if let Some(ui) = weak2.upgrade() {
                        sync_proposal_to_ui(&ui, &s);
                    }
                } else {
                    // Plain text response — show as status message
                    if let Some(ui) = weak2.upgrade() {
                        let display = if response.len() > 500 { &response[..500] } else { &response };
                        ui.set_pres_ai_status(display.into());
                    }
                }
            });
        });
    }

    // ── Apply ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_ai_apply(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let mut s = st.borrow_mut();
                if let Some(prop) = s.pending_proposal.take() {
                    let apply_title = ui.get_pres_ai_apply_title();
                    let apply_body = ui.get_pres_ai_apply_body();
                    let apply_notes = ui.get_pres_ai_apply_notes();

                    s.push_undo();

                    if prop.is_deck_proposal {
                        // Replace entire deck
                        s.slides = prop.proposed_slides.iter().map(|p| {
                            Slide {
                                title: if apply_title { p.title.clone() } else { String::new() },
                                body: if apply_body { bullets_to_body(&p.bullets) } else { String::new() },
                                notes: if apply_notes { p.speaker_notes.clone() } else { String::new() },
                                layout: slide_type_to_layout(&p.slide_type),
                                image_path: None,
                                formatting: SlideFormatting::default(),
                                transition: 0,
                            }
                        }).collect();
                        s.current_index = 0;
                    } else if prop.proposed_slides.len() > 1 {
                        // Split: remove current, insert replacements
                        let idx = s.current_index;
                        let base_slide = s.slides[idx].clone();
                        s.slides.remove(idx);
                        for (i, p) in prop.proposed_slides.iter().enumerate() {
                            s.slides.insert(idx + i, Slide {
                                title: if apply_title { p.title.clone() } else { base_slide.title.clone() },
                                body: if apply_body { bullets_to_body(&p.bullets) } else { base_slide.body.clone() },
                                notes: if apply_notes { p.speaker_notes.clone() } else { base_slide.notes.clone() },
                                layout: slide_type_to_layout(&p.slide_type),
                                image_path: None,
                                formatting: base_slide.formatting.clone(),
                                transition: 0,
                            });
                        }
                    } else if let Some(p) = prop.proposed_slides.first() {
                        // Single slide update
                        let idx = s.current_index;
                        if apply_title { s.slides[idx].title = p.title.clone(); }
                        if apply_body { s.slides[idx].body = bullets_to_body(&p.bullets); }
                        if apply_notes { s.slides[idx].notes = p.speaker_notes.clone(); }
                    }

                    s.is_modified = true;
                    sync_to_ui(&ui, &s);
                    sync_proposal_to_ui(&ui, &s);
                }
            }
        });
    }

    // ── Dismiss ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_ai_dismiss(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let mut s = st.borrow_mut();
                s.pending_proposal = None;
                sync_proposal_to_ui(&ui, &s);
                ui.set_pres_ai_status("".into());
            }
        });
    }

    // ── Preview navigation ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_ai_preview_slide(move |idx| {
            if let Some(ui) = ui_weak.upgrade() {
                let mut s = st.borrow_mut();
                if let Some(ref mut prop) = s.pending_proposal {
                    let new_idx = idx as usize;
                    if new_idx < prop.proposed_slides.len() {
                        prop.preview_index = new_idx;
                    }
                }
                sync_proposal_to_ui(&ui, &s);
            }
        });
    }

    // ── Regenerate ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_ai_regenerate(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let mut s = st.borrow_mut();
                s.pending_proposal = None;
                sync_proposal_to_ui(&ui, &s);
                ui.set_pres_ai_status("Dismissed. Click an action to regenerate.".into());
            }
        });
    }

    // ── Template gallery toggle ──
    {
        let ui_weak = ui.as_weak();
        ui.on_pres_open_template_gallery(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let open = ui.get_pres_template_gallery_open();
                ui.set_pres_template_gallery_open(!open);
            }
        });
    }

    // ── Template selection from gallery ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_select_template(move |idx| {
            let mut s = state.borrow_mut();
            // Map gallery template index to theme index
            // 0=Minimal->Dark(0), 1=Corporate->Light(1), 2=Creative->Blue(2),
            // 3=Academic->Green(3), 4=Pitch Deck->Dark(0)
            let theme_idx = match idx {
                0 => 0, // Minimal -> Dark
                1 => 1, // Corporate -> Light
                2 => 2, // Creative -> Blue
                3 => 3, // Academic -> Green
                4 => 0, // Pitch Deck -> Dark
                _ => 0,
            };
            let total = THEME_PRESETS.len() + s.custom_themes.len();
            s.current_theme = theme_idx.min(total.saturating_sub(1));
            tracing::debug!(template = idx, theme = s.current_theme, "Template gallery selection applied");
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }

    // ── Save speaker note ──
    {
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_pres_save_speaker_note(move |text| {
            let mut s = state.borrow_mut();
            let idx = s.current_index;
            let note_text = text.to_string();
            if note_text.is_empty() {
                s.speaker_notes.remove(&idx);
            } else {
                s.speaker_notes.insert(idx, note_text.clone());
            }
            // Also update the slide's notes field
            if idx < s.slides.len() {
                s.slides[idx].notes = note_text;
                s.is_modified = true;
            }
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &s);
            }
        });
    }
}
