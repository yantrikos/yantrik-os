//! The largest leading piece of a document the software renderer can be handed safely (#328).
//!
//! Slint's software renderer stores every physical coordinate in an `i16`: `draw_glyph_run`
//! casts each glyph origin with an unchecked euclid `cast()` before clipping, so a document of
//! ~2000 lines or a ~3900-character line, both well inside the 1 MiB / 20,000-line input bounds,
//! aborted the process with `Vector2D::cast()` (vector.rs:688). Tabs are restored at launch, so
//! an oversized document killed every restart in a loop.
//!
//! The view is therefore bounded: whole leading lines up to `limit_px` at a worst-case 2em line
//! height, and per line a worst-case 2em advance per character (tabs count 4). Both bounds are
//! above any real font's metrics. When a line does not fit, the view stops at it, so what is
//! shown stays a leading slice of the document and the byte offsets inside it keep meaning what
//! they mean. The whole document lives on in the tab: save, undo, `describe` and the drafts all
//! see every byte.

/// What the renderer may be handed of a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    /// The view is `text[..end]`, a char boundary.
    pub end: usize,
    /// How many lines the view shows: what the gutter and the colour layers have to cover.
    pub lines: usize,
    /// Part of the document is withheld; the view is read-only then.
    pub limited: bool,
}

/// The view of `text` at this render limit and font size.
pub fn view_of(text: &str, limit_px: f32, font_px: i32) -> Viewport {
    let font = font_px.max(1) as f32;
    // One budget for both axes: `limit / 2em` lines, and `limit / 2em` character units per line.
    let budget = (limit_px / (font * 2.0)).max(1.0) as usize;
    let mut end = 0;
    let mut lines = 0;
    for line in text.split_inclusive('\n') {
        if lines == budget {
            break;
        }
        let mut units = 0;
        let mut take = line.len();
        for (i, ch) in line.char_indices() {
            let wide = if ch == '\t' { 4 } else { 1 };
            if units + wide > budget {
                take = i;
                break;
            }
            units += wide;
        }
        end += take;
        lines += 1;
        if take < line.len() {
            break;
        }
    }
    // An empty document still shows as the one empty line the editor draws.
    Viewport { end, lines: lines.max(1), limited: end < text.len() }
}

/// The status bar's sentence for a view that withholds part of the document; empty otherwise.
pub fn status(view: Viewport, text: &str) -> String {
    if !view.limited {
        return String::new();
    }
    format!(
        "Showing the first {} of {} lines — the whole document is kept and saved",
        view.lines,
        super::lines(text)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_or_tall_document_is_windowed_and_a_small_one_is_not() {
        let small = "fn main() {}\n";
        assert!(!view_of(small, 15000.0, 14).limited);
        let wide = "x".repeat(5_000);
        let v = view_of(&wide, 15000.0, 14);
        assert!(v.limited && v.end < wide.len());
        let tall = "let value = 1;\n".repeat(19_000);
        let v = view_of(&tall, 15000.0, 14);
        assert!(v.limited && v.lines < 1000);
        assert!(view_of(&tall, 15000.0, 22).lines < v.lines, "a bigger font fits fewer lines");
        assert!(status(v, &tall).starts_with("Showing the first"));
        assert_eq!(view_of("", 15000.0, 14).lines, 1);
    }
}
