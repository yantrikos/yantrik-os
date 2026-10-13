//! The reading direction, held where the dock can mirror it.
//!
//! The locale table already knew Arabic reads right-to-left; these keep the flag plumbed from
//! it to the one strip that mirrors today. They read the source, so they run without building
//! Slint.

use std::path::{Path, PathBuf};

fn read(rel: &str) -> String {
    let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The shell sets the flag from the locale; every window reads it. Off by default, so an app
/// that never sets it keeps today's layout.
#[test]
fn rtl_flag_exists_and_is_off_by_default() {
    let src = read("crates/yantrik-design-tokens/slint/theme.slint");
    let g = src.find("export global DirectionMode").expect("DirectionMode in theme.slint");
    let body = &src[g..g + 200];
    assert!(body.contains("in-out property <bool> rtl: false"), "rtl must default to false");
}

/// Translations reach Slint through wire/i18n.rs; the direction belongs beside them, and
/// `push_translations` is what a locale switch re-calls, so both must set it.
#[test]
fn the_shell_sets_the_flag_from_the_locale() {
    let src = read("crates/yantrik-ui/src/wire/i18n.rs");
    let wire = src.find("pub fn wire(").expect("wire()");
    let push = src.find("pub fn push_translations(").expect("push_translations()");
    assert!(src[wire..push].contains("set_rtl(") && src[wire..push].contains("is_rtl()"),
        "wire() never pushes the direction");
    assert!(src[push..].contains("set_rtl(") && src[push..].contains("is_rtl()"),
        "push_translations() never re-pushes the direction on a locale switch");
}

/// The dock is the one strip that mirrors: its row swaps its halves on the flag, and the
/// pager glyphs flip with it — page-back points the way the row came from.
#[test]
fn the_dock_mirrors_when_rtl() {
    let src = read("crates/yantrik-ui-slint/ui/components/grounded_dock.slint");
    assert!(src.contains("import { Theme, DirectionMode }"), "the dock never imports the direction");
    let body = src.find("body := HorizontalLayout").expect("the dock's row");
    assert!(src[body..].contains("if DirectionMode.rtl :") && src[body..].contains("if !DirectionMode.rtl :"),
        "the dock's row never reads the direction");
    let pagers = src.lines().filter(|l| l.trim_start().starts_with("text: DirectionMode.rtl")).count();
    assert_eq!(pagers, 2, "both pager glyphs must flip with the direction");
}

/// The flag starts in the locale table: Arabic is marked, and `is_rtl` answers it.
#[test]
fn arabic_is_marked_rtl() {
    let src = read("crates/yantrik-ui/src/i18n.rs");
    let ar = src.lines().find(|l| l.contains("code: \"ar\"")).expect("Arabic in the locale table");
    assert!(ar.contains("rtl: true"), "Arabic is not marked rtl");
    let f = src.find("pub fn is_rtl").expect("is_rtl()");
    assert!(src[f..f + 200].contains("i.rtl"), "is_rtl() does not return the locale's flag");
}
