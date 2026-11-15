//! One look at a display: what it says, and how much of that had to be read.

use std::time::Instant;

use crate::memory::Memory;
use crate::read::{Line, Reader, Timing};

/// What one look found, and what it cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub width: u32,
    pub height: u32,
    pub lines: Vec<Line>,
    /// Rectangles read afresh on this look.
    pub read: usize,
    /// Rectangles that held exactly the pixels they held when last read, and were not read.
    pub remembered: usize,
    /// The whole image was the one last looked at: nothing was found or read again.
    pub unchanged: bool,
    pub timing: Timing,
    pub seconds: f64,
}

impl Reading {
    /// The reading as the shell hands it on. `lines` and `clipped` are the shape `read_screen`
    /// has always answered with; the rest says what the look cost.
    pub fn to_json(&self, max_lines: usize) -> serde_json::Value {
        let tenths = |seconds: f64| (seconds * 10.0).round() / 10.0;
        let millis = |seconds: f64| (seconds * 1000.0).round() as u64;
        serde_json::json!({
            "clipped": self.lines.len() > max_lines,
            "width": self.width,
            "height": self.height,
            "seconds": tenths(self.seconds),
            "lines": self.lines.iter().take(max_lines).map(|l| serde_json::json!({
                "text": l.text,
                "box": [l.x, l.y, l.w, l.h],
            })).collect::<Vec<_>>(),
            "cost": {
                "read": self.read,
                "remembered": self.remembered,
                "unchanged": self.unchanged,
                "find_ms": millis(self.timing.find),
                "read_ms": millis(self.timing.read),
                "total_ms": millis(self.seconds),
            },
        })
    }
}

/// Look at an RGB image of the display `space`, reading only what `memory` does not already
/// hold for exactly these pixels.
pub fn look(reader: &Reader, memory: &mut Memory, space: &str, rgb: &[u8], width: u32, height: u32) -> Result<Reading, String> {
    let started = Instant::now();
    let image = memory.image_key(rgb, width, height);
    if let Some(lines) = memory.same_image(space, image) {
        return Ok(Reading { width, height, lines, read: 0, remembered: 0, unchanged: true, timing: Timing::default(), seconds: started.elapsed().as_secs_f64() });
    }

    let mut keys = Vec::new();
    let mut recalled = Vec::new();
    let found = reader.find_and_read(rgb, width, height, |bounds| {
        bounds
            .iter()
            .map(|bounds| {
                let key = memory.line_key(rgb, width, bounds);
                let said = memory.recall(space, key, bounds);
                let unread = said.is_none();
                keys.push(key);
                recalled.push(said);
                unread
            })
            .collect()
    })?;

    let (mut read, mut remembered) = (0, 0);
    let mut lines = Vec::new();
    for (((bounds, fresh), key), said) in found.bounds.iter().zip(found.lines).zip(keys).zip(recalled) {
        let line = match said {
            Some(line) => {
                remembered += 1;
                line
            }
            None => {
                read += 1;
                memory.keep(space, key, bounds, fresh.as_ref());
                fresh
            }
        };
        lines.extend(line);
    }
    memory.keep_image(space, image, &lines);
    Ok(Reading { width, height, lines, read, remembered, unchanged: false, timing: found.timing, seconds: started.elapsed().as_secs_f64() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(lines: usize) -> Reading {
        let line = |n: usize| Line { text: format!("line {n}"), x: 4, y: 20 * n as i32, w: 100, h: 16 };
        Reading { width: 1280, height: 800, lines: (0..lines).map(line).collect(), read: 2, remembered: lines.saturating_sub(2), unchanged: false, timing: Timing { find: 0.4012, read: 0.0304 }, seconds: 0.4321 }
    }

    #[test]
    fn a_reading_keeps_the_shape_the_shell_has_always_been_given() {
        let json = reading(3).to_json(300);
        assert_eq!(json["width"], 1280);
        assert_eq!(json["height"], 800);
        assert_eq!(json["seconds"], 0.4);
        assert_eq!(json["clipped"], false);
        assert_eq!(json["lines"][1], serde_json::json!({ "text": "line 1", "box": [4, 20, 100, 16] }));
    }

    #[test]
    fn a_reading_says_what_it_cost() {
        let json = reading(40).to_json(300);
        assert_eq!(json["cost"], serde_json::json!({ "read": 2, "remembered": 38, "unchanged": false, "find_ms": 401, "read_ms": 30, "total_ms": 432 }));
    }

    #[test]
    fn more_lines_than_asked_for_are_cut_and_said_to_be() {
        let json = reading(10).to_json(4);
        assert_eq!(json["lines"].as_array().map(Vec::len), Some(4));
        assert_eq!(json["clipped"], true);
    }
}
