//! The boot splash and the shell's boot screen are one picture, so the hand-off does not move.
//!
//! Three things draw the emblem on Lake's ground before the desktop: plymouth's theme
//! (deploy/yantrik-os/boot-unlock/theme/yantrik.script, from the kernel's first frame), swaybg under
//! labwc (config/labwc/autostart, until the shell draws), and the shell's boot screen (ui/boot.slint,
//! its first frame). Each states the size and colour in its own language, so nothing but this test
//! keeps them the same: the PNG plymouth and swaybg show is the size the boot screen draws, and the
//! three grounds are one colour.

use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &str) -> String {
    std::fs::read_to_string(repo().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Width and height from a PNG's IHDR, which is always the first chunk.
fn png_size(path: &str) -> (u32, u32) {
    let bytes = std::fs::read(repo().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "{path} is a PNG");
    assert_eq!(&bytes[12..16], b"IHDR", "{path}");
    let n = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    (n(16), n(20))
}

/// The value of `property <ty> name: value;` in a .slint source.
fn slint_property<'a>(source: &'a str, name: &str) -> &'a str {
    let line = source
        .lines()
        .find(|l| l.trim_start().starts_with("property <") && l.contains(&format!("> {name}:")))
        .unwrap_or_else(|| panic!("boot.slint has no property {name}"));
    line.split(':').nth(1).unwrap().trim().trim_end_matches(';').trim()
}

const BOOT: &str = "crates/yantrik-ui-slint/ui/boot.slint";
const SCRIPT: &str = "deploy/yantrik-os/boot-unlock/theme/yantrik.script";
const EMBLEM: &str = "deploy/yantrik-os/boot-unlock/theme/emblem.png";
const AUTOSTART: &str = "config/labwc/autostart";

#[test]
fn the_splash_emblem_is_the_size_the_boot_screen_draws() {
    let boot = read(BOOT);
    let size = slint_property(&boot, "splash-emblem");
    let px: u32 = size.trim_end_matches("px").parse().unwrap_or_else(|_| panic!("splash-emblem: {size}"));
    assert_eq!(png_size(EMBLEM), (px, px), "{EMBLEM} is drawn at its own size by plymouth and swaybg");
    // And brand/render.py, which makes it, makes it at that size.
    assert!(read("brand/render.py").contains(&format!("SPLASH_EMBLEM_SIZE = {px}")));
}

#[test]
fn one_ground_for_the_splash_swaybg_and_the_boot_screen() {
    let boot = read(BOOT);
    let ground = slint_property(&boot, "splash-ground").to_lowercase();
    assert_eq!(ground, "#0e1117", "Lake's bg-deep");
    assert!(boot.contains("background: root.splash-ground;"), "the boot screen opens on it");
    let hex = |at: usize| u8::from_str_radix(&ground[at..at + 2], 16).unwrap();
    let rgb = [hex(1), hex(3), hex(5)];

    // plymouth: the splash's ground, flat (top and bottom the same), as fractions of 255.
    let script = read(SCRIPT);
    let body: String = script
        .split("fun splash_ground()")
        .nth(1)
        .expect("yantrik.script has splash_ground()")
        .split('}')
        .next()
        .unwrap()
        .to_string();
    let colours: Vec<[u8; 3]> = body
        .lines()
        .filter(|l| l.contains("Window.SetBackground"))
        .map(|l| {
            let args = l.split('(').nth(1).unwrap().split(')').next().unwrap();
            let v: Vec<u8> = args.split(',').map(|x| (x.trim().parse::<f64>().unwrap() * 255.0).round() as u8).collect();
            [v[0], v[1], v[2]]
        })
        .collect();
    assert_eq!(colours, vec![rgb, rgb], "plymouth's splash ground is {ground}, top and bottom");

    // swaybg, under labwc until the shell draws.
    let autostart = read(AUTOSTART);
    let swaybg = autostart.lines().find(|l| l.contains("swaybg ")).expect("autostart starts swaybg");
    assert!(swaybg.contains(&format!("-c '{ground}'")), "{swaybg}");
    assert!(swaybg.contains("-m center"), "at its own size, centred: {swaybg}");
    assert!(autostart.contains("SPLASH=/usr/share/plymouth/themes/yantrik/emblem.png"), "the image plymouth shows");
}

#[test]
fn the_first_frame_puts_the_emblem_where_the_splash_did() {
    let boot = read(BOOT);
    // Centred on the screen until the screen settles, as the theme centres it.
    assert!(boot.contains("x: (root.width - root.splash-emblem) / 2;"));
    assert!(boot.contains("y: root.settled ? 76px : (root.height - root.splash-emblem) / 2;"));
    assert!(boot.contains("property <bool> settled: false;"), "the first frame is the splash");
    let script = read(SCRIPT);
    assert!(script.contains("emblem.SetX(cx - emblem_image.GetWidth() / 2);"));
    assert!(script.contains("emblem.SetY(cy - emblem_image.GetHeight() / 2);"));
    // The scene and the words come up after it; nothing but the ground and the emblem at first.
    assert_eq!(boot.matches("opacity: root.settled ? ").count(), 3, "wallpaper, wash, and everything else");
}
