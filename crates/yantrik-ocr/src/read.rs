//! One image in, its lines of text out, each with the rectangle it occupies.

use std::path::Path;

use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
use rten::Model;

/// One line of text found on the image, and where: `x, y, w, h` in the image's own pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// The detection and recognition models, loaded once.
pub struct Reader {
    engine: OcrEngine,
}

impl Reader {
    /// Load `text-detection.rten` and `text-recognition.rten` from `dir`.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let load = |name: &str| {
            let path = dir.join(name);
            Model::load_file(&path).map_err(|e| format!("cannot load the OCR model {}: {e}", path.display()))
        };
        let engine = OcrEngine::new(OcrEngineParams {
            detection_model: Some(load("text-detection.rten")?),
            recognition_model: Some(load("text-recognition.rten")?),
            ..Default::default()
        })
        .map_err(|e| format!("cannot start the OCR engine: {e}"))?;
        Ok(Self { engine })
    }

    /// Every line of text in an RGB image `width` × `height`, in the engine's reading order:
    /// a column of text is read to its end before the next one starts.
    pub fn lines(&self, rgb: &[u8], width: u32, height: u32) -> Result<Vec<Line>, String> {
        let source = ImageSource::from_bytes(rgb, (width, height)).map_err(|e| format!("not an image: {e}"))?;
        let input = self.engine.prepare_input(source).map_err(|e| format!("cannot prepare the image: {e}"))?;
        let words = self.engine.detect_words(&input).map_err(|e| format!("text detection failed: {e}"))?;
        let rects = self.engine.find_text_lines(&input, &words);
        let texts = self.engine.recognize_text(&input, &rects).map_err(|e| format!("text recognition failed: {e}"))?;
        Ok(texts
            .iter()
            .flatten()
            .filter_map(|line| {
                // The upright rectangle around every character: screen text is not rotated.
                let r = line.bounding_rect();
                keep(Line {
                    text: line.to_string(),
                    x: r.left() as i32,
                    y: r.top() as i32,
                    w: r.width() as i32,
                    h: r.height() as i32,
                })
            })
            .collect())
    }
}

/// A line worth reporting: trimmed, and with at least one word in it — two letters or digits
/// together. What falls out is icon glyphs read as `@`, `©`, a lone `J`, or a title bar's
/// minimise and close read as `o x`: noise a reader would have to learn to skip.
pub fn keep(mut line: Line) -> Option<Line> {
    line.text = line.text.split_whitespace().collect::<Vec<_>>().join(" ");
    let has_word = line
        .text
        .split_whitespace()
        .any(|token| token.chars().filter(|c| c.is_alphanumeric()).count() >= 2);
    has_word.then_some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Line {
        Line { text: text.into(), x: 1, y: 2, w: 3, h: 4 }
    }

    #[test]
    fn icon_glyphs_are_not_text() {
        for noise in ["@", "©", "J", "  + ", "o x", ":::"] {
            assert_eq!(keep(line(noise)), None, "{noise:?} should be dropped");
        }
    }

    #[test]
    fn a_line_keeps_its_words_and_loses_its_air() {
        assert_eq!(keep(line("  root   root  4201 ")).unwrap().text, "root root 4201");
        assert_eq!(keep(line("OK")).unwrap().text, "OK");
    }
}
