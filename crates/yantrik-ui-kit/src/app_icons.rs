//! The app icon set (design/icons/generate.py) held to the colour table it is drawn from.
//!
//! The PNGs are committed so the build needs neither Python nor resvg; these are what notice
//! when they fall behind. Every app `AppColor.hue-for-app` names has art at every size, the
//! Slint mapping (slint/app_icons.slint) offers exactly that art, the set was generated from
//! today's inputs, and each tile keeps the hue the table gave it. They read files, so they run
//! without building Slint.

use std::path::{Path, PathBuf};

use crate::colour_roles::{hue, in_a_reserved_family};
use crate::lock_shared::picture_size;

/// The sizes the generator renders, in pixels.
const SIZES: [u32; 6] = [32, 48, 64, 96, 128, 256];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn icons() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app-icons")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The body of `public pure function <name>(` in app_color.slint, up to its closing brace.
fn function_body<'a>(src: &'a str, name: &str) -> &'a str {
    let start = src.find(&format!("public pure function {name}(")).unwrap_or_else(|| panic!("no function {name}"));
    &src[start..start + src[start..].find("\n    }").expect("the function ends")]
}

/// `(id, hue, tile, ink)` for every app in the table, as the generator reads it (palette.py).
fn table() -> Vec<(String, String, String, String)> {
    let src = read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("slint/app_color.slint"));
    let tile = |hue: &str| {
        let key = format!("out property <color> tile-{hue}:");
        let line = src.lines().find(|l| l.trim_start().starts_with(&key)).unwrap_or_else(|| panic!("no tile-{hue}"));
        line[line.find('#').expect("a colour")..].trim_end_matches(';').trim().to_lowercase()
    };
    let on_tile = function_body(&src, "on-tile");
    let light = &on_tile[..on_tile.find("? root.tile-ink").expect("the dark ink's hues")];
    let ink = tile("ink");
    function_body(&src, "hue-for-app")
        .lines()
        .filter_map(|line| {
            let after = &line[line.find("id == \"")? + 7..];
            let (id, rest) = after.split_once('"')?;
            let hue = rest.split_once('?')?.1.trim().strip_prefix('"')?.split('"').next()?;
            let ink = if light.contains(&format!("name == \"{hue}\"")) { ink.clone() } else { "#ffffff".into() };
            Some((id.to_string(), hue.to_string(), tile(hue), ink))
        })
        .collect()
}

#[test]
fn every_app_has_art_at_every_size() {
    let apps = table();
    assert!(apps.len() >= 30, "read the colour table, found {} apps", apps.len());
    let mut names: Vec<String> = apps.iter().map(|a| a.0.clone()).collect();
    names.extend(["_plate-light".into(), "_plate-dark".into(), "_mask".into()]);
    for name in &names {
        for size in SIZES {
            let path = icons().join(format!("{size}/{name}.png"));
            let bytes = std::fs::read(&path).unwrap_or_else(|_| panic!("{name} has no {size}px art; run design/icons/generate.py"));
            assert_eq!(picture_size(&bytes), Some((size, size)), "{} is not {size}x{size}", path.display());
        }
    }
    for (id, ..) in &apps {
        assert!(icons().join(format!("{id}.svg")).is_file(), "{id} has no source SVG");
    }
}

/// AppIcons offers exactly the generated art: every app at every size, each size's two plates
/// (light and dark) and mask, and the empty image an unknown id falls through to. Nothing it
/// names is missing.
#[test]
fn the_slint_mapping_offers_exactly_the_art() {
    let src = read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("slint/app_icons.slint"));
    let apps = table();
    for (id, ..) in &apps {
        assert!(src.contains(&format!("id == \"{id}\"\n")) || src.contains(&format!("id == \"{id}\";")), "has() leaves out {id}");
        for size in SIZES {
            assert!(src.contains(&format!("@image-url(\"../assets/app-icons/{size}/{id}.png\")")), "{id} at {size}px is not mapped");
        }
    }
    let urls: Vec<&str> = src.split("@image-url(\"").skip(1).map(|s| &s[..s.find('"').unwrap()]).collect();
    assert_eq!(urls.len(), SIZES.len() * (apps.len() + 4), "one image per app and size, two plates, a mask and an empty fallback per size");
    for url in urls.iter().filter(|u| !u.is_empty()) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("slint").join(url);
        assert!(path.is_file(), "the mapping names {url}, which is not there");
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a64(data: &[u8]) -> u64 {
    data.iter().fold(FNV_OFFSET, |h, b| (h ^ *b as u64).wrapping_mul(FNV_PRIME))
}

/// Every file under `dir`, as (path from `root` with `/`, path), skipping Python's caches.
fn files_under(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "__pycache__") {
                files_under(root, &path, out);
            }
        } else {
            let rel = path.strip_prefix(root).unwrap().components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            out.push((rel, path));
        }
    }
}

/// The generator's inputs hashed the way design/icons/fingerprint.py hashes them.
fn fingerprint() -> u64 {
    let dir = repo().join("design/icons");
    let mut files = Vec::new();
    files_under(&dir, &dir, &mut files);
    files.sort();
    let mut bytes = Vec::new();
    for (name, path) in &files {
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(b'\n');
        bytes.extend(std::fs::read(path).unwrap().into_iter().filter(|b| *b != b'\r'));
        bytes.push(b'\n');
    }
    for (id, hue, tile, ink) in table() {
        bytes.extend_from_slice(format!("{id}:{hue}:{tile}:{ink}\n").as_bytes());
    }
    fnv1a64(&bytes)
}

/// The committed art is what the generator makes from today's table and today's design/icons/.
/// An app added to the table, a hue changed there, or a glyph or the template changed without
/// re-running the generator fails here, not on a screen.
#[test]
fn the_icons_are_generated_from_the_current_inputs() {
    let written = read(&icons().join("fingerprint.txt"));
    let recorded = written.lines().find_map(|l| l.strip_prefix("fnv1a64 ")).expect("the fingerprint line");
    assert_eq!(
        recorded,
        format!("{:016x}", fingerprint()),
        "the app icons are out of date with design/icons/ or AppColor's table: run \
         `python design/icons/generate.py` (requirements in design/icons/requirements.txt) and commit the result"
    );
}

#[test]
fn the_fingerprint_is_fnv1a() {
    assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
}

/// A tile's gradient keeps the hue the table gave it, top and bottom, so the art cannot carry an
/// app into a colour role it was not given: teal is the minds' and only Browser's tile wears it,
/// and amber is only Notes'.
#[test]
fn a_tile_keeps_its_hue() {
    for (id, _, tile, _) in table() {
        let svg = read(&icons().join(format!("{id}.svg")));
        let bg = &svg[svg.find("<linearGradient id=\"bg\"").expect("the background gradient")..];
        let bg = &bg[..bg.find("</linearGradient>").unwrap()];
        let stops: Vec<&str> = bg.split("stop-color=\"").skip(1).map(|s| &s[..7]).collect();
        assert_eq!(stops.len(), 2, "{id}: a top and a bottom");
        let want = hue(&tile).unwrap_or_else(|| panic!("{id}'s tile {tile} is grey"));
        for stop in stops {
            let got = hue(stop).unwrap_or_else(|| panic!("{id}: {stop} has no hue"));
            let off = (got - want).abs().min(360.0 - (got - want).abs());
            assert!(off <= 3.0, "{id}: the gradient stop {stop} drifts {off:.1} degrees from its tile {tile}");
            if in_a_reserved_family(stop) == Some("teal (a mind)") {
                assert_eq!(id, "browser", "{id}'s tile is teal, the minds' colour");
            }
        }
    }
    let notes = table().into_iter().find(|a| a.0 == "notes").expect("notes");
    assert_eq!(notes.1, "amber");
    assert!(table().iter().filter(|a| a.1 == "amber").all(|a| a.0 == "notes"), "amber is only Notes'");
}
