//! An app's name under its tile is one object, held where it was soft (4 Oct 2026).
//!
//! Beside an iPhone home screen, the shell's tiles were crisp and the names under them were not:
//! 11-12px regular, often grey, centred at fractional offsets. Every tile label is now the kit's
//! AppLabel, which draws Theme.fs-label / fw-label on whole pixels. These read the source, so they
//! run without building Slint.

use std::path::{Path, PathBuf};

fn read(rel: &str) -> String {
    let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

const UI: &str = "crates/yantrik-ui-slint/ui/";

/// Every place the shell writes an app's (or a mind's desk's) name under its tile, and the
/// expression it writes it with.
const LABELS: &[(&str, &str)] = &[
    ("components/desktop_home.slint", "text: item.label;"),
    ("components/app_grid.slint", "text: root.name;"),
    ("components/alt_tab.slint", "text: root.cell.app-name;"),
    ("components/grounded_dock.slint", "text: root.over-name;"),
    ("agents_workroom.slint", "text: root.desk.via == \"\" ? root.desk.mind"),
];

/// The name of the element whose body holds byte `at`: the word before the nearest `{` behind it.
fn element_around(src: &str, at: usize) -> &str {
    let open = src[..at].rfind('{').expect("an element encloses the property");
    src[..open].trim_end().rsplit(|c: char| c.is_whitespace()).next().unwrap_or("")
}

#[test]
fn every_app_label_is_the_kits_app_label() {
    for (file, expr) in LABELS {
        let src = read(&format!("{UI}{file}"));
        assert!(src.contains("import { AppLabel } from \"app_label.slint\";"), "{file} does not import AppLabel");
        let hits: Vec<usize> = src.match_indices(expr).map(|(at, _)| at).collect();
        assert!(!hits.is_empty(), "{file} no longer writes `{expr}`; update this test with where the label went");
        for at in hits {
            assert_eq!(element_around(&src, at), "AppLabel", "{file} draws `{expr}` in a bare element, not AppLabel");
        }
    }
}

/// AppLabel is the label token and nothing else, top-aligned in a box of whole label lines, so a
/// box on a whole pixel puts the baseline on one (Barlow's ascent is exactly its size).
#[test]
fn app_label_draws_the_label_token_on_whole_pixels() {
    let src = read("crates/yantrik-ui-kit/slint/app_label.slint");
    let sizes: Vec<&str> = src.lines().filter(|l| l.trim_start().starts_with("font-size:")).map(str::trim).collect();
    assert!(!sizes.is_empty() && sizes.iter().all(|l| *l == "font-size: Theme.fs-label;"), "AppLabel sizes: {sizes:?}");
    let weights: Vec<&str> = src.lines().filter(|l| l.trim_start().starts_with("font-weight:")).map(str::trim).collect();
    assert!(!weights.is_empty() && weights.iter().all(|l| *l == "font-weight: Theme.fw-label;"), "AppLabel weights: {weights:?}");
    assert!(src.contains("height: root.lines * Theme.lh-label;"), "AppLabel's box is whole label lines");
    assert!(!src.contains("vertical-alignment: center"), "centred text lands at (box - 15.6px) / 2, a fraction");
    assert!(src.contains("color: Theme.label-shadow;"), "a label on the wallpaper keeps its shadow");
}

/// The tokens: 13px medium, and a line that is a whole number of pixels tall.
#[test]
fn the_label_tokens_are_13px_medium_on_a_whole_pixel_line() {
    let theme = read("crates/yantrik-design-tokens/slint/theme.slint");
    for token in [
        "out property <length> fs-label: 13px;",
        "out property <int>    fw-label: 500;",
        "out property <length> lh-label: 16px;",
        "out property <color> label-on-wallpaper: #ffffff;",
    ] {
        assert!(theme.contains(token), "theme.slint lacks `{token}`");
    }
}

/// A tile centres its name by a rounded offset: `alignment: center` in the launcher's cell put
/// the name 13.5px down a 112px cell.
#[test]
fn tiles_centre_their_labels_by_whole_pixels() {
    let grid = read(&format!("{UI}components/app_grid.slint"));
    let cell = &grid[grid.find("component LauncherCell").expect("the launcher cell")..grid.find("export component AppGrid").expect("the grid")];
    assert!(!cell.contains("alignment: center;\n        spacing: 7px;"), "the launcher cell centres by layout again");
    assert!(cell.contains("padding-top: Math.round("), "the launcher cell's offset is rounded");
    let dock = read(&format!("{UI}components/grounded_dock.slint"));
    let label = &dock[dock.find("// ── Name label ──").expect("the dock's name label")..dock.find("// ── The window list ──").expect("the list")];
    assert!(label.matches("Math.round(").count() >= 2, "the dock's pill and the name in it sit on whole pixels");
}
