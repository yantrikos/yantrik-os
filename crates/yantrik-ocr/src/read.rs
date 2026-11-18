//! One image in, its lines of text out, each with the rectangle it occupies.
//!
//! Reading has two halves that cost very differently. *Finding* the lines is one pass of a
//! model over the whole image, the same work whatever is on it. *Reading* each line found is
//! another model, once per line, and it is most of the time for a screen full of text. So the
//! two are offered apart: [`Reader::find_and_read`] finds every line, shows the caller where
//! each one is, and reads only the ones the caller asks for. A caller that remembers what a
//! line said the last time it looked exactly like this (`memory`) asks for few.

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
use rten::Model;
use rten_imageproc::{RectF, RotatedRect};

use crate::settle::settle;

/// One line of text found on the image, and where: `x, y, w, h` in the image's own pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Where a line is, before anything in it is read: an upright rectangle inside the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Bounds {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// How long each half took, in seconds. `find` is everything before the models read a line:
/// finding, settling, and matching against what is remembered.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Timing {
    pub find: f64,
    pub read: f64,
}

/// What one look found.
pub struct Found {
    /// Every line found, in the engine's reading order: a column of text is read to its end
    /// before the next one starts. Each rectangle is settled onto the line's own ink (`settle`),
    /// and is exactly the rectangle the engine was given to read.
    pub bounds: Vec<Bounds>,
    /// For each of `bounds`: the line, when it was asked for and read as something worth
    /// reporting.
    pub lines: Vec<Option<Line>>,
    pub timing: Timing,
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

    /// Find every line of text in an RGB image `width` × `height`, and read the ones `wanted`
    /// says to. `wanted` is shown where every line is and answers, for each, whether to read it.
    pub fn find_and_read(&self, rgb: &[u8], width: u32, height: u32, wanted: impl FnOnce(&[Bounds]) -> Vec<bool>) -> Result<Found, String> {
        let source = ImageSource::from_bytes(rgb, (width, height)).map_err(|e| format!("not an image: {e}"))?;
        let input = self.engine.prepare_input(source).map_err(|e| format!("cannot prepare the image: {e}"))?;

        let started = Instant::now();
        let words = self.engine.detect_words(&input).map_err(|e| format!("text detection failed: {e}"))?;
        let rects = self.engine.find_text_lines(&input, &words);

        let settled = rects
            .iter()
            .map(|line| around(line.iter().flat_map(|word| word.corners()).map(|corner| (corner.x, corner.y)), width, height))
            .map(|found| settle(rgb, width, height, &found));
        let bounds = distinct(settled);
        let mut wanted = wanted(&bounds);
        wanted.resize(bounds.len(), false);
        let find = started.elapsed().as_secs_f64();
        // The engine reads the settled rectangle, not the words the finder drew: what a line
        // says is then decided by its pixels alone, and reads the same on every look.
        let chosen: Vec<Vec<RotatedRect>> = bounds.iter().zip(&wanted).filter(|(bounds, want)| **want && bounds.w > 0 && bounds.h > 0).map(|(bounds, _)| vec![upright(bounds)]).collect();

        let started = Instant::now();
        let texts = if chosen.is_empty() {
            Vec::new()
        } else {
            self.engine.recognize_text(&input, &chosen).map_err(|e| format!("text recognition failed: {e}"))?
        };
        let read = started.elapsed().as_secs_f64();

        let mut texts = texts.into_iter();
        let lines = wanted
            .iter()
            .zip(&bounds)
            .map(|(want, bounds)| {
                if !*want || bounds.w == 0 || bounds.h == 0 {
                    return None;
                }
                let line = texts.next().flatten()?;
                // The upright rectangle around every character: screen text is not rotated.
                let r = line.bounding_rect();
                keep(Line { text: line.to_string(), x: r.left(), y: r.top(), w: r.width(), h: r.height() })
            })
            .collect();
        Ok(Found { bounds, lines, timing: Timing { find, read } })
    }
}

/// The rectangles, each once, in the order they first came: two found rectangles that settle
/// onto the same ink are one line.
fn distinct(bounds: impl Iterator<Item = Bounds>) -> Vec<Bounds> {
    let mut seen = HashSet::new();
    bounds.filter(|bounds| seen.insert(*bounds)).collect()
}

/// A rectangle as the engine takes one: a single upright word.
fn upright(bounds: &Bounds) -> RotatedRect {
    let (left, top) = (bounds.x as f32, bounds.y as f32);
    RotatedRect::from_rect(RectF::from_tlbr(top, left, top + bounds.h as f32, left + bounds.w as f32))
}

/// The upright rectangle around a set of points, kept inside an image `width` × `height`. A
/// line with no points, or wholly outside, is an empty rectangle at the corner.
pub fn around(points: impl Iterator<Item = (f32, f32)>, width: u32, height: u32) -> Bounds {
    let (mut left, mut top, mut right, mut bottom) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in points {
        left = left.min(x);
        top = top.min(y);
        right = right.max(x);
        bottom = bottom.max(y);
    }
    if left > right || top > bottom {
        return Bounds { x: 0, y: 0, w: 0, h: 0 };
    }
    let inside = |value: f32, limit: u32| value.clamp(0.0, limit as f32) as u32;
    let (x, y) = (inside(left.floor(), width), inside(top.floor(), height));
    let (x1, y1) = (inside(right.ceil(), width), inside(bottom.ceil(), height));
    Bounds { x, y, w: x1.saturating_sub(x), h: y1.saturating_sub(y) }
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
    fn two_found_rectangles_over_the_same_ink_are_one_line() {
        let (w, h) = (100u32, 40u32);
        let mut rgb = vec![20u8; (w * h * 3) as usize];
        for y in 10..20 {
            for x in (10..60).filter(|x| x % 4 != 0) {
                let at = ((y * w + x) * 3) as usize;
                rgb[at..at + 3].fill(230);
            }
        }
        let wobbled = [Bounds { x: 8, y: 8, w: 54, h: 14 }, Bounds { x: 9, y: 9, w: 52, h: 12 }];
        let other = Bounds { x: 8, y: 28, w: 54, h: 8 };
        let settled = wobbled.iter().chain([&other]).map(|found| crate::settle::settle(&rgb, w, h, found));
        let lines = distinct(settled);
        assert_eq!(lines.len(), 2, "the two over the ink are one, the empty one is kept as found: {lines:?}");
        assert_eq!(lines[1], other);
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

    #[test]
    fn a_lines_rectangle_is_the_one_around_all_its_words_corners() {
        let corners = [(10.4, 20.6), (90.2, 20.6), (90.2, 37.1), (10.4, 37.1), (95.0, 21.0), (140.9, 36.0)];
        assert_eq!(around(corners.into_iter(), 1280, 800), Bounds { x: 10, y: 20, w: 131, h: 18 });
    }

    #[test]
    fn a_rectangle_never_reaches_outside_the_image() {
        let corners = [(-5.0, -3.0), (1290.0, 10.0), (1290.0, 805.0)];
        assert_eq!(around(corners.into_iter(), 1280, 800), Bounds { x: 0, y: 0, w: 1280, h: 800 });
        assert_eq!(around(std::iter::empty(), 1280, 800), Bounds { x: 0, y: 0, w: 0, h: 0 }, "no points, no rectangle");
        let outside = [(2000.0, 900.0), (2100.0, 950.0)];
        assert_eq!(around(outside.into_iter(), 1280, 800).w, 0, "wholly outside is empty");
    }
}
