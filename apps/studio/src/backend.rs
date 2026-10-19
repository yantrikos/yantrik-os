//! The three places a picture can come from, behind one trait.
//!
//! They are deliberately not equal. ComfyUI is the "your own GPU" route and speaks a graph; the
//! OpenAI-compatible route speaks a sentence and bills for it; the fake route draws a placeholder
//! and is what an unconfigured machine gets rather than an app that refuses to open. What they
//! share is one question — make this picture — and one answer, bytes plus the facts that go into
//! the sidecar beside the file.
//!
//! Nothing in here touches Slint or the filesystem. Every backend runs on a worker thread and is
//! tested against a fake rather than a service somebody has to be running.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use serde_json::json;

use crate::config::{Backend as BackendConfig, Config, Kind};

/// A flag a caller sets to say "stop". Checked between the steps that can be interrupted, which
/// for ComfyUI means between polls — the server is also told, because a render nobody is waiting
/// for is still burning a GPU somebody owns.
pub type Cancel = Arc<AtomicBool>;

pub fn new_cancel() -> Cancel {
    Arc::new(AtomicBool::new(false))
}

/// One picture, asked for. Everything here is filled in by the app before it reaches a backend:
/// the defaults, the clamps and the seed are decided once in `engine`, so a backend cannot be
/// handed a width of zero and have to invent a policy about it.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub prompt: String,
    pub negative: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub seed: u64,
    pub cfg: f64,
}

impl Request {
    /// The size the person asked for, as they would write it.
    pub fn requested(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }
}

/// One picture, made. `sent` exists next to `requested` because the two are not always the same:
/// a hosted service accepts a handful of sizes and the fake backend caps its own, and a sidecar
/// that recorded only the request would be a small lie about the file beside it.
#[derive(Clone, Debug)]
pub struct Shot {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub model: String,
    pub sent: String,
}

/// One question — make this picture — and one name.
///
/// The name is asked of the backend rather than read from the configuration beside it, because it
/// goes into the record saved next to the file, and that record has to say what actually made the
/// pixels. A resample says `studio-resample` for the same reason: nothing was asked, and the field
/// must not claim otherwise.
pub trait Backend: Send + Sync {
    /// The kind, as the configuration spells it.
    fn kind(&self) -> &'static str;

    /// Make one picture, or say why not. The `Err` text is shown to a person and read by a mind,
    /// so it names the thing that failed and what would fix it.
    fn generate(&self, request: &Request, cancel: &Cancel) -> Result<Shot, String>;
}

/// Build the backend a configuration asks for. One place, so the app and `set_backend` cannot
/// disagree about what a kind means.
pub fn build(config: &Config) -> Arc<dyn Backend> {
    match config.backend.kind {
        Kind::ComfyUi => Arc::new(ComfyUi::new(config.backend.clone())),
        Kind::OpenAiImages => Arc::new(OpenAiImages::new(config.backend.clone())),
        Kind::Fake => Arc::new(Fake::new(config.backend.clone())),
    }
}

// ── ComfyUI ─────────────────────────────────────────────────────────────

/// HTTP to a ComfyUI server: hand it a graph, poll until the graph has finished, fetch the file
/// it wrote. The API is the one the editor's own "Save (API Format)" produces, so a person can
/// replace the shipped graph with one they built.
pub struct ComfyUi {
    settings: BackendConfig,
    /// How long a single render is allowed to take. Long, because an SDXL pass on a modest GPU
    /// is measured in minutes and giving up at thirty seconds would look like a bug.
    budget: std::time::Duration,
}

impl ComfyUi {
    pub fn new(settings: BackendConfig) -> ComfyUi {
        ComfyUi { settings, budget: std::time::Duration::from_secs(15 * 60) }
    }

    fn agent() -> ureq::Agent {
        // A connect timeout, and no read timeout: the read that matters is the poll, which
        // answers immediately, and a render can legitimately take longer than any read timeout
        // worth setting. The overall `budget` is the real limit.
        ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(8))
            .build()
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.settings.base_url.trim_end_matches('/'), path)
    }

    /// What the server's own history entry names for the file, or an error quoting the server.
    fn queue(&self, graph: &serde_json::Value) -> Result<String, String> {
        let body = json!({ "prompt": graph, "client_id": "yantrik-studio" });
        match Self::agent().post(&self.url("/prompt")).send_json(body) {
            Ok(response) => match response.into_json::<serde_json::Value>() {
                Ok(value) => value
                    .get("prompt_id")
                    .and_then(|id| id.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| "the server accepted the graph but sent back no prompt id".to_string()),
                Err(e) => Err(format!("the server's reply was not readable ({e})")),
            },
            // A 400 here is almost always the graph: a node the server does not have, a
            // checkpoint it does not hold, a link that goes nowhere. Its own explanation is the
            // useful part, so it is passed through rather than summarised away.
            Err(ureq::Error::Status(code, response)) => {
                Err(format!("the server refused the graph (HTTP {code}): {}", explain(response)))
            }
            Err(ureq::Error::Transport(e)) => Err(unreachable_message(&self.settings.base_url, &e)),
        }
    }

    /// Poll `/history/<id>` until the entry appears with an output, or the budget runs out.
    fn wait(&self, id: &str, cancel: &Cancel) -> Result<Found, String> {
        let started = Instant::now();
        let path = format!("/history/{id}");
        loop {
            if cancel.load(Ordering::SeqCst) {
                self.interrupt();
                return Err("cancelled".to_string());
            }
            if started.elapsed() > self.budget {
                self.interrupt();
                return Err(format!(
                    "the server has been working on this for {} seconds and has not finished; it may still be running",
                    started.elapsed().as_secs()
                ));
            }
            match Self::agent().get(&self.url(&path)).call() {
                Ok(response) => match response.into_json::<serde_json::Value>() {
                    Ok(value) => {
                        if let Some(found) = Found::read(&value, id) {
                            return Ok(found);
                        }
                    }
                    Err(e) => return Err(format!("the server's history was not readable ({e})")),
                },
                // A server that restarts mid-render drops the entry and will never answer for
                // it. Two failures in a row are reported rather than polled forever.
                Err(ureq::Error::Transport(e)) => {
                    if started.elapsed() > std::time::Duration::from_secs(30) {
                        return Err(unreachable_message(&self.settings.base_url, &e));
                    }
                }
                Err(ureq::Error::Status(code, _)) => {
                    return Err(format!("the server would not answer about this render (HTTP {code})"))
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
    }

    fn fetch(&self, found: &Found) -> Result<Vec<u8>, String> {
        let mut path = format!("/view?filename={}&type={}", url_part(&found.filename), found.kind);
        if !found.subfolder.is_empty() {
            path.push_str(&format!("&subfolder={}", url_part(&found.subfolder)));
        }
        match Self::agent().get(&self.url(&path)).call() {
            Ok(response) => {
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut response.into_reader(), &mut bytes)
                    .map_err(|e| format!("the picture could not be read from the server ({e})"))?;
                if bytes.is_empty() {
                    return Err(format!("the server sent an empty file for {}", found.filename));
                }
                Ok(bytes)
            }
            Err(ureq::Error::Status(code, response)) => Err(format!(
                "the server would not hand over {} (HTTP {code}): {}",
                found.filename,
                explain(response)
            )),
            Err(ureq::Error::Transport(e)) => Err(unreachable_message(&self.settings.base_url, &e)),
        }
    }

    /// Tell the server to stop. Best-effort: the render is abandoned either way, and failing to
    /// say so must not replace the reason the caller is being told about.
    fn interrupt(&self) {
        let _ = Self::agent().post(&self.url("/interrupt")).call();
    }
}

impl Backend for ComfyUi {
    fn kind(&self) -> &'static str {
        "comfyui"
    }

    fn generate(&self, request: &Request, cancel: &Cancel) -> Result<Shot, String> {
        // The server's own copy is filed under the same date as Studio's, so the two can be
        // matched up by a person looking in both places.
        let prefix = format!("Studio/{}", chrono::Local::now().format("%Y-%m-%d"));
        let filled = crate::workflow::fill(
            crate::workflow::graph(&self.settings.workflow)?,
            request,
            &self.settings.model,
            &prefix,
        )?;
        if let Some(missed) = filled.missed.first() {
            // Not fatal — the picture may still be exactly what was asked for — but a person who
            // pointed Studio at their own graph and got something else deserves to know which
            // part of it did not take.
            tracing::warn!("the ComfyUI graph did not take everything: {missed}");
        }
        let id = self.queue(&filled.graph)?;
        let found = self.wait(&id, cancel)?;
        let bytes = self.fetch(&found)?;
        let (width, height) = crate::gallery::png_size(&bytes);
        // `sent` is what the graph was told to draw, which is the request snapped to the latent
        // grid; a graph with no latent at all drew at its own size and says so in `size`.
        let sent = match filled.size {
            Some((width, height)) => format!("{width}x{height}"),
            None => request.requested(),
        };
        Ok(Shot {
            bytes,
            width: width.unwrap_or(request.width),
            height: height.unwrap_or(request.height),
            model: self.settings.model.clone(),
            sent,
        })
    }
}

/// One file a ComfyUI history entry points at.
#[derive(Clone, Debug)]
struct Found {
    filename: String,
    subfolder: String,
    kind: String,
}

impl Found {
    /// Read the first output image out of a `/history` response. The entry appears as soon as the
    /// prompt is queued and gains its `outputs` only when the graph finishes, so an entry with no
    /// images yet is "still working", not "failed".
    fn read(history: &serde_json::Value, id: &str) -> Option<Found> {
        let entry = history.get(id)?;
        let outputs = entry.get("outputs")?.as_object()?;
        // Prefer a node that says it wrote an output; otherwise take the first image anywhere.
        let mut fallback: Option<Found> = None;
        for node in outputs.values() {
            let Some(images) = node.get("images").and_then(|i| i.as_array()) else { continue };
            for image in images {
                let filename = image.get("filename").and_then(|f| f.as_str()).unwrap_or_default();
                if filename.is_empty() {
                    continue;
                }
                let found = Found {
                    filename: filename.to_string(),
                    subfolder: image
                        .get("subfolder")
                        .and_then(|s| s.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    kind: image.get("type").and_then(|t| t.as_str()).unwrap_or("output").to_string(),
                };
                if found.kind == "output" {
                    return Some(found);
                }
                fallback.get_or_insert(found);
            }
        }
        fallback
    }
}

/// Percent-encode the parts of a query this builds by hand. A filename from a server is trusted
/// about as far as it has to be: `&` in one would change what the next request asks for.
fn url_part(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The short part of a server's error body, which is where the useful sentence is.
fn explain(response: ureq::Response) -> String {
    let mut text = response.into_string().unwrap_or_default();
    // A JSON error from ComfyUI carries the sentence in `error`, and the whole envelope is noise
    // next to it.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(message) = value.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str())
        {
            text = message.to_string();
        } else if let Some(message) = value.get("error").and_then(|e| e.as_str()) {
            text = message.to_string();
        }
    }
    let text = text.trim();
    if text.chars().count() > 300 {
        format!("{}…", text.chars().take(300).collect::<String>())
    } else if text.is_empty() {
        "no explanation was sent".to_string()
    } else {
        text.to_string()
    }
}

fn unreachable_message(base_url: &str, problem: &ureq::Transport) -> String {
    format!(
        "{} could not be reached ({}). Is ComfyUI running there, and is it listening on something other than 127.0.0.1 if this is not the same machine?",
        crate::config::shown_address(base_url),
        transport_words(problem)
    )
}

/// Why a request went nowhere, without the URL: ureq's own `Display` of a transport error begins
/// with the whole URL it was given, sign-in details and query included (review of #856).
fn transport_words(problem: &ureq::Transport) -> String {
    use std::error::Error;
    match problem.source() {
        Some(source) => format!("{}: {source}", problem.kind()),
        None => problem.kind().to_string(),
    }
}

// ── OpenAI-compatible images ────────────────────────────────────────────

/// Any endpoint that speaks `/images/generations`. The key is read from the environment at the
/// moment of the call and is not held in this struct, not written to a sidecar, and not put in
/// any message a person or a mind can read.
pub struct OpenAiImages {
    settings: BackendConfig,
}

/// The sizes this backend will ask for. Services differ in what they accept, and `gpt-image-1`
/// accepts exactly three; asking for 1023x769 gets a 400 that does not explain itself. Snapping
/// to the nearest supported size and recording both numbers in the sidecar is the honest version.
const OPENAI_SIZES: [&str; 3] = ["1024x1024", "1536x1024", "1024x1536"];

impl OpenAiImages {
    pub fn new(settings: BackendConfig) -> OpenAiImages {
        OpenAiImages { settings }
    }

    fn url(&self) -> String {
        format!("{}/images/generations", self.settings.base_url.trim_end_matches('/'))
    }
}

/// The nearest supported size, judging both how much picture it is and what shape it is.
/// Exported for its test: the rule looks obvious until a 2000x700 request has to choose, and
/// getting it wrong means a person who asked for a wide picture gets a square one back with
/// nothing anywhere saying that happened.
pub fn snap_to_a_supported_size(width: u32, height: u32) -> &'static str {
    if width == 0 || height == 0 {
        return "1024x1024";
    }
    let wanted_area = (width as f64) * (height as f64);
    let wanted_ratio = width as f64 / height as f64;
    let mut best = OPENAI_SIZES[0];
    let mut best_score = f64::MAX;
    for candidate in OPENAI_SIZES {
        let (wide, high) = dimensions(candidate);
        let area = ((wide * high) - wanted_area).abs() / wanted_area;
        let shape = ((wide / high) - wanted_ratio).abs() / wanted_ratio;
        // Shape counts for more than area: a square where a landscape was asked for is a
        // different picture, while a landscape somewhat smaller is the same picture.
        let score = area + shape * 4.0;
        if score < best_score {
            best_score = score;
            best = candidate;
        }
    }
    best
}

fn dimensions(size: &str) -> (f64, f64) {
    let mut parts = size.split('x');
    let width: f64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(1024.0);
    let height: f64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(1024.0);
    (width, height)
}

impl Backend for OpenAiImages {
    fn kind(&self) -> &'static str {
        "openai-images"
    }

    fn generate(&self, request: &Request, cancel: &Cancel) -> Result<Shot, String> {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".to_string());
        }
        let key = self.settings.api_key()?;
        let size = snap_to_a_supported_size(request.width, request.height);
        // `response_format` is deliberately not sent: `gpt-image-1` rejects it, and DALL·E
        // defaults to a URL. Both replies are handled below instead, which costs a few lines and
        // works against either.
        let body = json!({
            "model": self.settings.model,
            "prompt": request.prompt,
            "n": 1,
            "size": size,
        });
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(5 * 60))
            .build();
        let response = agent
            .post(&self.url())
            .set("Authorization", &format!("Bearer {key}"))
            .set("Content-Type", "application/json")
            .send_json(body)
            // The key is dropped here, before any message is built: an `Err` from this point can
            // carry the URL, the status and the server's own words, and none of them is the key.
            .map_err(|e| match e {
                ureq::Error::Status(code, response) => {
                    format!("{} refused the request (HTTP {code}): {}", crate::config::shown_address(&self.settings.base_url), explain(response))
                }
                ureq::Error::Transport(e) => format!("{} could not be reached ({})", crate::config::shown_address(&self.settings.base_url), transport_words(&e)),
            })?;
        drop(key);

        let value: serde_json::Value = response
            .into_json()
            .map_err(|e| format!("the reply from {} was not readable JSON ({e})", crate::config::shown_address(&self.settings.base_url)))?;
        let first = value
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|d| d.first())
            .ok_or_else(|| format!("{} sent no image and no error: {}", crate::config::shown_address(&self.settings.base_url), value))?;

        let bytes = if let Some(encoded) = first.get("b64_json").and_then(|b| b.as_str()) {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(encoded.trim())
                .map_err(|e| format!("the image came back base64-encoded but could not be decoded ({e})"))?
        } else if let Some(location) = first.get("url").and_then(|u| u.as_str()) {
            // The URL is the service's own, fetched immediately: it is usually signed and short
            // lived, and a sidecar pointing at an expired link is a picture that cannot be shown.
            let response = agent
                .get(location)
                .call()
                .map_err(|e| format!("the image was offered at a URL that could not be fetched ({e})"))?;
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut response.into_reader(), &mut bytes)
                .map_err(|e| format!("the image could not be read from its URL ({e})"))?;
            bytes
        } else {
            return Err(format!(
                "the reply from {} held neither image bytes nor a URL: {}",
                crate::config::shown_address(&self.settings.base_url),
                first
            ));
        };

        let (width, height) = crate::gallery::png_size(&bytes);
        let (sent_width, sent_height) = dimensions(size);
        Ok(Shot {
            bytes,
            width: width.unwrap_or(sent_width as u32),
            height: height.unwrap_or(sent_height as u32),
            model: self.settings.model.clone(),
            sent: size.to_string(),
        })
    }
}

// ── Fake ────────────────────────────────────────────────────────────────

/// Draws a deterministic placeholder from the prompt's hash: same prompt and seed, same picture.
///
/// This is not only a test double. It is what a machine with no GPU and no API key gets, so that
/// Studio opens, the gallery works, the sidecars are written, and a mind can drive the whole
/// surface end to end — with `describe` saying plainly that the pictures are placeholders. A
/// deterministic drawing is what makes it useful in tests too: the bytes can be compared.
pub struct Fake {
    settings: BackendConfig,
    /// The long edge this backend will draw. A placeholder has no reason to be 4096 pixels wide,
    /// and a test that asked for one would spend its time on pixels nobody looks at.
    cap: u32,
}

impl Fake {
    pub fn new(settings: BackendConfig) -> Fake {
        Fake { settings, cap: 1024 }
    }

    /// The tests' knob: a placeholder nobody will look at has no reason to be 1024 pixels across,
    /// and a test that drew one would spend its time on pixels. Compiled only under `cfg(test)`, so
    /// the app itself has exactly one way to build this backend — `new`, with the person's settings.
    #[cfg(test)]
    pub fn capped_at(cap: u32) -> Fake {
        Fake { settings: BackendConfig::defaults_for(Kind::Fake), cap }
    }
}

impl Backend for Fake {
    fn kind(&self) -> &'static str {
        "fake"
    }

    fn generate(&self, request: &Request, cancel: &Cancel) -> Result<Shot, String> {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".to_string());
        }
        let (width, height) = shrink(request.width.max(1), request.height.max(1), self.cap);
        let bytes = draw(&request.prompt, request.seed, width, height);
        Ok(Shot {
            bytes,
            width,
            height,
            model: if self.settings.model.is_empty() {
                "fake".to_string()
            } else {
                self.settings.model.clone()
            },
            sent: format!("{width}x{height}"),
        })
    }
}

fn shrink(width: u32, height: u32, cap: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= cap {
        return (width, height);
    }
    let scale = cap as f64 / longest as f64;
    (
        ((width as f64) * scale).round().max(1.0) as u32,
        ((height as f64) * scale).round().max(1.0) as u32,
    )
}

/// The drawing itself: a field of blocks whose colours come from a stream seeded by the prompt
/// and the seed, over a gradient, with a border so it is unmistakably a placeholder at a glance.
/// Pure — no clock, no randomness, no I/O — which is the whole reason it can be asserted on.
pub fn draw(prompt: &str, seed: u64, width: u32, height: u32) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(prompt.as_bytes());
    hasher.update(b"|");
    hasher.update(seed.to_le_bytes());
    let digest = hasher.finalize();

    // A 64-bit xorshift stream from the digest, so the drawing can be as detailed as it likes
    // without the digest running out.
    let mut state = u64::from_le_bytes(digest[..8].try_into().unwrap()) | 1;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    let base = [digest[8], digest[9], digest[10]];
    let accent = [digest[11], digest[12], digest[13]];
    let blocks = 6 + (next() % 5) as u32;

    let mut image = image::RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let along = if width > 1 { x as f64 / (width - 1) as f64 } else { 0.0 };
            let down = if height > 1 { y as f64 / (height - 1) as f64 } else { 0.0 };
            let mix = (along + down) / 2.0;
            let mut pixel = [
                (base[0] as f64 * (1.0 - mix) + accent[0] as f64 * mix) as u8,
                (base[1] as f64 * (1.0 - mix) + accent[1] as f64 * mix) as u8,
                (base[2] as f64 * (1.0 - mix) + accent[2] as f64 * mix) as u8,
            ];
            let block_x = (along * blocks as f64) as u32;
            let block_y = (down * blocks as f64) as u32;
            if (block_x + block_y) % 2 == 0 {
                pixel = [
                    pixel[0].saturating_add(24),
                    pixel[1].saturating_sub(12),
                    pixel[2].saturating_add(40),
                ];
            }
            // The border, so nobody mistakes this for a real render in a thumbnail.
            let edge = (width.min(height) / 24).max(2);
            if x < edge || y < edge || x + edge >= width || y + edge >= height {
                pixel = [28, 28, 32];
            }
            image.put_pixel(x, y, image::Rgb(pixel));
        }
    }

    let mut bytes = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        // An in-memory PNG encode of a picture this size does not fail; if it ever does, the
        // caller gets the reason rather than a panic in a worker thread nobody is watching.
        .expect("the placeholder picture could not be encoded as a PNG");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_prompt_and_seed_draw_the_same_picture() {
        let one = draw("a lighthouse in fog", 7, 64, 48);
        let two = draw("a lighthouse in fog", 7, 64, 48);
        assert_eq!(one, two);
    }

    #[test]
    fn a_different_word_or_a_different_seed_draws_a_different_picture() {
        let base = draw("a lighthouse in fog", 7, 64, 48);
        assert_ne!(base, draw("a lighthouse in rain", 7, 64, 48));
        assert_ne!(base, draw("a lighthouse in fog", 8, 64, 48));
    }

    #[test]
    fn the_placeholder_is_a_readable_png_of_the_size_asked_for() {
        let bytes = draw("a lighthouse", 1, 120, 80);
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']), "not a PNG");
        let (width, height) = crate::gallery::png_size(&bytes);
        assert_eq!((width, height), (Some(120), Some(80)));
    }

    #[test]
    fn the_fake_backend_caps_a_placeholder_rather_than_drawing_thousands_of_pixels_nobody_will_see() {
        let fake = Fake::capped_at(256);
        let shot = fake
            .generate(
                &Request {
                    prompt: "a very large request".into(),
                    negative: String::new(),
                    width: 4096,
                    height: 2048,
                    steps: 30,
                    seed: 3,
                    cfg: 7.0,
                },
                &new_cancel(),
            )
            .unwrap();
        assert_eq!((shot.width, shot.height), (256, 128));
        // And it says so, rather than letting the sidecar claim 4096x2048.
        assert_eq!(shot.sent, "256x128");
    }

    #[test]
    fn a_cancelled_request_never_reaches_a_backend() {
        let cancel = new_cancel();
        cancel.store(true, Ordering::SeqCst);
        let request = Request {
            prompt: "anything".into(),
            negative: String::new(),
            width: 64,
            height: 64,
            steps: 4,
            seed: 1,
            cfg: 7.0,
        };
        assert_eq!(
            Fake::capped_at(64).generate(&request, &cancel).unwrap_err(),
            "cancelled"
        );
        // The network backends check before they open a socket, so a cancelled generation costs
        // nothing and, for the hosted one, sends no prompt anywhere.
        let hosted = OpenAiImages::new(BackendConfig {
            api_key_env: "STUDIO_TEST_NO_SUCH_VARIABLE".into(),
            model: "gpt-image-1".into(),
            ..BackendConfig::defaults_for(Kind::OpenAiImages)
        });
        assert_eq!(hosted.generate(&request, &cancel).unwrap_err(), "cancelled");
    }

    #[test]
    fn a_hosted_backend_with_no_key_exported_says_what_to_export() {
        std::env::remove_var("STUDIO_TEST_MISSING_KEY");
        let hosted = OpenAiImages::new(BackendConfig {
            api_key_env: "STUDIO_TEST_MISSING_KEY".into(),
            model: "gpt-image-1".into(),
            ..BackendConfig::defaults_for(Kind::OpenAiImages)
        });
        let problem = hosted
            .generate(
                &Request {
                    prompt: "anything".into(),
                    negative: String::new(),
                    width: 64,
                    height: 64,
                    steps: 4,
                    seed: 1,
                    cfg: 7.0,
                },
                &new_cancel(),
            )
            .unwrap_err();
        assert!(problem.contains("STUDIO_TEST_MISSING_KEY"), "{problem}");
    }

    #[test]
    fn a_size_the_service_does_not_accept_is_snapped_and_both_numbers_are_kept() {
        assert_eq!(snap_to_a_supported_size(1024, 1024), "1024x1024");
        assert_eq!(snap_to_a_supported_size(512, 512), "1024x1024");
        assert_eq!(snap_to_a_supported_size(1920, 1080), "1536x1024");
        assert_eq!(snap_to_a_supported_size(1080, 1920), "1024x1536");
        assert_eq!(snap_to_a_supported_size(2000, 700), "1536x1024");
        assert_eq!(snap_to_a_supported_size(0, 0), "1024x1024");
    }

    #[test]
    fn a_filename_from_a_server_cannot_change_what_the_next_request_asks_for() {
        assert_eq!(url_part("Studio_00001_.png"), "Studio_00001_.png");
        assert_eq!(url_part("a&b=c d.png"), "a%26b%3Dc%20d.png");
        assert_eq!(url_part("sub/folder"), "sub%2Ffolder");
    }

    #[test]
    fn the_file_a_history_entry_points_at_is_found_whichever_node_wrote_it() {
        let history = json!({
            "abc": {
                "outputs": {
                    "9": { "images": [{ "filename": "Studio_00001_.png", "subfolder": "", "type": "output" }] }
                }
            }
        });
        let found = Found::read(&history, "abc").unwrap();
        assert_eq!(found.filename, "Studio_00001_.png");
        assert_eq!(found.kind, "output");

        // Queued but not finished: the entry is there, its outputs are not. That is "still
        // working", and treating it as a failure would end every render early.
        assert!(Found::read(&json!({ "abc": { "outputs": {} } }), "abc").is_none());
        assert!(Found::read(&json!({}), "abc").is_none());

        // A preview rather than an output is still a picture, and is taken if it is all there is.
        let preview = json!({
            "abc": { "outputs": { "3": { "images": [{ "filename": "p.png", "type": "temp" }] } } }
        });
        assert_eq!(Found::read(&preview, "abc").unwrap().kind, "temp");

        // And an entry naming an empty filename is not one.
        let empty = json!({
            "abc": { "outputs": { "9": { "images": [{ "filename": "", "type": "output" }] } } }
        });
        assert!(Found::read(&empty, "abc").is_none());
    }

    #[test]
    fn a_server_error_message_is_passed_through_rather_than_summarised_away() {
        // ComfyUI's own words for a graph it cannot run are the useful part of the failure.
        let body = r#"{"error":{"message":"Value not in list","type":"invalid_prompt"},"node_errors":{}}"#;
        let said = |body: &str| explain(ureq::Response::new(400, "Bad Request", body).unwrap());
        assert_eq!(said(body), "Value not in list");
        assert_eq!(said(r#"{"error":"not authenticated"}"#), "not authenticated");
        assert_eq!(said(""), "no explanation was sent");
        // A body that is not JSON at all is still the server's own words, trimmed to a length a
        // summary line can carry.
        assert_eq!(said("  upstream is down  "), "upstream is down");
        let shortened = said(&"x".repeat(900));
        assert!(shortened.chars().count() <= 301, "{}", shortened.chars().count());
        assert!(shortened.ends_with('…'));
    }
}
