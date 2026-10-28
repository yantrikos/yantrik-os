//! The placeholder backend: a deterministic PNG drawn from the prompt's hash. What an unconfigured
//! machine gets rather than an app that refuses to open, and what every test here makes pictures
//! with.

use crate::backend::{Backend, Request, Shot, Stop};
use crate::config::Backend as BackendConfig;

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
        Fake { settings: BackendConfig::defaults_for(crate::config::Kind::Fake), cap }
    }
}

impl Backend for Fake {
    fn kind(&self) -> &'static str {
        "fake"
    }

    fn generate(&self, request: &Request, cancel: &Stop) -> Result<Shot, String> {
        if cancel.is_set() {
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
            if (block_x + block_y).is_multiple_of(2) {
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
