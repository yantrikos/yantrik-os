//! The wallpaper behind the lock screen, blurred once when it is chosen.
//!
//! The lock screen shows the person's wallpaper, softened so the clock and the field read over it.
//! A blur drawn live would cost a full-screen filter on every frame, and the software renderer
//! (which is what most of this OS runs on) has none worth the name, so it is done here, once, off
//! the UI thread, when a wallpaper is chosen (and at start when the file is missing or was made
//! for another wallpaper). The result is an ordinary PNG in the cache that `yantrik-lock` and the
//! shell's own lock screen load like any other image.
//!
//! The source is small on purpose. A blurred picture holds no detail, so the blur is taken from
//! the 320×200 preview of a shipped wallpaper (already embedded for the Settings picker) or from
//! a custom file shrunk to 480 wide: a few hundred thousand pixels, not four million, and the
//! lock screen scales it up smoothly. Nothing here reads the lock's secret or draws anything.

use std::path::{Path, PathBuf};

/// The width a custom wallpaper is shrunk to before it is blurred.
const WORK_WIDTH: u32 = 480;
/// How far the blur spreads, in pixels of the working image, and how many box passes make it.
/// Two box passes of 2px on a 320-wide picture, shown at the screen's width, soften every edge
/// and keep the shape of the scene: the ridge and its reflection are still the lake. (7px in
/// three passes was tried first and left a dark smear in which nothing could be recognised.)
const RADIUS: usize = 2;
const PASSES: usize = 2;

/// The shipped wallpapers' previews, by preset id. A test makes every preset appear here, so a
/// wallpaper added to the picker without a preview here fails instead of leaving the lock screen
/// on the wrong picture.
fn preview(id: &str) -> Option<&'static [u8]> {
    macro_rules! p {
        ($name:literal) => {
            Some(include_bytes!(concat!("../../yantrik-ui-slint/ui/wallpapers/previews/", $name, ".png")) as &[u8])
        };
    }
    match id {
        "lake" => p!("lake"),
        "serenity" => p!("serenity"),
        "first-light" => p!("first-light"),
        "nightfall" => p!("nightfall"),
        "aurora" => p!("aurora"),
        "sunset" => p!("sunset"),
        "ocean" => p!("ocean"),
        "nebula" => p!("nebula"),
        _ => None,
    }
}

/// A shipped wallpaper's preview as an image for the shell's own windows (the Settings theme
/// cards), decoded from the same bytes the blur is made from.
pub fn preview_image(id: &str) -> Option<slint::Image> {
    let (rgb, w, h) = decode_png(preview(id)?).ok()?;
    Some(slint::Image::from_rgb8(slint::SharedPixelBuffer::clone_from_slice(&rgb, w as u32, h as u32)))
}

fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/root".into())).join(".cache"));
    base.join("yantrik")
}

fn image_path() -> PathBuf {
    cache_dir().join("lock-wallpaper.png")
}

/// What the blurred file was made from, kept beside it so a start can tell whether it is current.
fn stamp_path() -> PathBuf {
    cache_dir().join("lock-wallpaper.source")
}

/// The blurred wallpaper, if one has been made. `None` means the lock draws its solid charcoal.
pub fn ready() -> Option<PathBuf> {
    let path = image_path();
    path.is_file().then_some(path)
}

/// What identifies the source: a preset's id, or a file's path with its size and modified time,
/// so editing the picture in place counts as a new one.
fn source_stamp(source: &str) -> String {
    if preview(source).is_some() || source.is_empty() {
        return source.to_string();
    }
    let meta = std::fs::metadata(source).ok();
    let modified = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    format!("{source}|{}|{modified}", meta.map_or(0, |m| m.len()))
}

/// Make the lock wallpaper for `source` (a preset id, a file path, or empty for none) on a worker
/// thread. Never blocks the caller; a failure is logged and leaves whatever was there, which at
/// worst is the previous wallpaper's blur.
pub fn refresh(source: &str) {
    let source = source.to_string();
    std::thread::spawn(move || match build(&source, &cache_dir()) {
        Ok(()) => tracing::info!(wallpaper = %source, "Lock-screen wallpaper blurred"),
        Err(e) => tracing::warn!(wallpaper = %source, error = %e, "Could not make the lock-screen wallpaper"),
    });
}

/// At start: make it if there is none or it was made for another wallpaper. Cheap when current.
pub fn refresh_if_stale(source: &str) {
    let current = std::fs::read_to_string(stamp_path()).ok();
    if current.as_deref() == Some(source_stamp(source).as_str()) && (source.is_empty() || ready().is_some()) {
        return;
    }
    refresh(source);
}

/// The work: decode, shrink, blur, write. `dir` is the cache directory.
pub fn build(source: &str, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cache folder: {e}"))?;
    let image = dir.join("lock-wallpaper.png");
    let stamp = dir.join("lock-wallpaper.source");
    // No wallpaper (a solid colour was chosen): the lock has nothing to show but its charcoal.
    if source.is_empty() {
        let _ = std::fs::remove_file(&image);
        return std::fs::write(&stamp, "").map_err(|e| e.to_string());
    }
    let (mut rgb, w, h) = match preview(source) {
        Some(bytes) => decode_png(bytes)?,
        None => load_custom(source)?,
    };
    blur_rgb(&mut rgb, w, h, RADIUS, PASSES);
    // Written beside, then renamed in: a lock client reading it never sees half a file.
    let tmp = dir.join("lock-wallpaper.png.tmp");
    encode_png(&tmp, &rgb, w, h)?;
    std::fs::rename(&tmp, &image).map_err(|e| e.to_string())?;
    std::fs::write(&stamp, source_stamp(source)).map_err(|e| e.to_string())
}

/// A shipped preview, as packed RGB.
fn decode_png(bytes: &[u8]) -> Result<(Vec<u8>, usize, usize), String> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("picture too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (w, h) = (info.width as usize, info.height as usize);
    let rgb = match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgb, png::BitDepth::Eight) => buf[..w * h * 3].to_vec(),
        (png::ColorType::Rgba, png::BitDepth::Eight) => buf[..w * h * 4].chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect(),
        other => return Err(format!("unsupported preview format {other:?}")),
    };
    Ok((rgb, w, h))
}

/// The largest wallpaper file the lock's blur will read, and the most pixels it will decode: an
/// 8K photograph fits; a small file whose header claims billions of pixels is not decoded at all
/// (security review of #601).
const CUSTOM_MOST_BYTES: u64 = 64 * 1024 * 1024;
const CUSTOM_MOST_PIXELS: u64 = 8192 * 8192;

/// A person's own wallpaper file, through the same decoder the desktop uses, shrunk. Its size is
/// read from its header first, so a picture too big to hold is refused before it is decoded.
fn load_custom(path: &str) -> Result<(Vec<u8>, usize, usize), String> {
    let mut head = Vec::new();
    {
        use std::io::Read;
        let file = std::fs::File::open(Path::new(path)).map_err(|_| "the file could not be opened".to_string())?;
        let meta = file.metadata().map_err(|_| "the file could not be read".to_string())?;
        if !meta.is_file() || meta.len() > CUSTOM_MOST_BYTES {
            return Err("the file is too large for the lock screen".into());
        }
        file.take(64 * 1024).read_to_end(&mut head).map_err(|_| "the file could not be read".to_string())?;
    }
    let (hw, hh) = yantrik_ui_kit::lock_shared::picture_size(&head).ok_or("only a PNG or JPEG can be the lock screen's picture")?;
    if hw == 0 || hh == 0 || u64::from(hw) * u64::from(hh) > CUSTOM_MOST_PIXELS {
        return Err("the picture is too large for the lock screen".into());
    }
    let image = slint::Image::load_from_path(Path::new(path)).map_err(|_| "the file could not be loaded".to_string())?;
    let pixels = image.to_rgba8().ok_or("the picture has no pixels to read")?;
    let (sw, sh) = (pixels.width(), pixels.height());
    if sw == 0 || sh == 0 {
        return Err("the picture is empty".into());
    }
    let w = sw.min(WORK_WIDTH);
    let h = ((u64::from(sh) * u64::from(w)) / u64::from(sw)).max(1) as u32;
    let rgba: Vec<u8> = pixels.as_bytes().to_vec();
    Ok((shrink_rgba(&rgba, sw as usize, sh as usize, w as usize, h as usize), w as usize, h as usize))
}

/// Box-average an RGBA picture down to `(w, h)`, as packed RGB. Each output pixel is the mean of
/// the source pixels it covers, so a 2560×1600 photograph shrinks without shimmer.
fn shrink_rgba(src: &[u8], sw: usize, sh: usize, w: usize, h: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        let (y0, y1) = (y * sh / h, ((y + 1) * sh / h).max(y * sh / h + 1).min(sh));
        for x in 0..w {
            let (x0, x1) = (x * sw / w, ((x + 1) * sw / w).max(x * sw / w + 1).min(sw));
            let mut sum = [0u64; 3];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let at = (sy * sw + sx) * 4;
                    for c in 0..3 {
                        sum[c] += u64::from(src[at + c]);
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u64;
            out.extend(sum.iter().map(|s| (s / n) as u8));
        }
    }
    out
}

/// Blur packed RGB in place: `passes` box blurs of `radius`, horizontal then vertical each time.
/// Edges clamp, so the borders keep their colour instead of fading to black.
fn blur_rgb(px: &mut [u8], w: usize, h: usize, radius: usize, passes: usize) {
    if w == 0 || h == 0 || radius == 0 {
        return;
    }
    let mut line = vec![0u8; w.max(h) * 3];
    for _ in 0..passes {
        for y in 0..h {
            let row = &mut px[y * w * 3..(y + 1) * w * 3];
            box_line(row, &mut line[..w * 3], w, 3, radius);
        }
        for x in 0..w {
            // A column, gathered, blurred and put back.
            for y in 0..h {
                line[y * 3..y * 3 + 3].copy_from_slice(&px[(y * w + x) * 3..(y * w + x) * 3 + 3]);
            }
            let mut column = line[..h * 3].to_vec();
            box_line(&mut column, &mut line[..h * 3], h, 3, radius);
            for y in 0..h {
                px[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&column[y * 3..y * 3 + 3]);
            }
        }
    }
}

/// One box blur along a line of `n` pixels of `channels` bytes, with a running sum: O(n), not
/// O(n × radius). `scratch` is the same length as `line`.
fn box_line(line: &mut [u8], scratch: &mut [u8], n: usize, channels: usize, radius: usize) {
    scratch.copy_from_slice(line);
    let window = (2 * radius + 1) as u32;
    for c in 0..channels {
        let at = |i: isize| u32::from(scratch[i.clamp(0, n as isize - 1) as usize * channels + c]);
        let mut sum: u32 = (-(radius as isize)..=radius as isize).map(at).sum();
        for i in 0..n {
            line[i * channels + c] = ((sum + window / 2) / window) as u8;
            sum = sum + at(i as isize + radius as isize + 1) - at(i as isize - radius as isize);
        }
    }
}

fn encode_png(path: &Path, rgb: &[u8], w: usize, h: usize) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().and_then(|mut writer| writer.write_image_data(rgb)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-lockwp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn every_wallpaper_in_the_picker_has_a_preview_to_blur() {
        for id in crate::wire::settings::WALLPAPER_PRESETS {
            let bytes = preview(id).unwrap_or_else(|| panic!("no lock-screen source for the {id} wallpaper"));
            let (rgb, w, h) = decode_png(bytes).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(rgb.len(), w * h * 3, "{id}");
        }
    }

    #[test]
    fn a_hard_edge_becomes_a_slope_and_a_flat_picture_stays_flat() {
        // Left half black, right half white.
        let (w, h) = (40, 6);
        let mut px: Vec<u8> = (0..w * h).flat_map(|i| if i % w < w / 2 { [0, 0, 0] } else { [255, 255, 255] }).collect();
        blur_rgb(&mut px, w, h, 4, 3);
        let at = |x: usize| px[(2 * w + x) * 3];
        assert!(at(0) < 8 && at(w - 1) > 247, "the far sides keep their colour");
        assert!(at(w / 2 - 1) > 40 && at(w / 2 - 1) < 215, "the edge is now a slope, not a step ({})", at(w / 2 - 1));
        assert!((0..w - 1).all(|x| at(x) <= at(x + 1)), "and it only ever rises");

        let mut flat = vec![90u8; 20 * 20 * 3];
        blur_rgb(&mut flat, 20, 20, 7, 3);
        assert!(flat.iter().all(|&v| v == 90), "a flat picture is not tinted by the blur");
    }

    #[test]
    fn shrinking_averages_what_it_covers() {
        // A 4×2 picture of alternating black and white columns, shrunk to 2×1: every output
        // pixel is the mean of two black and two white pixels.
        let src: Vec<u8> = (0..8).flat_map(|i| if i % 2 == 0 { [0, 0, 0, 255] } else { [200, 200, 200, 255] }).collect();
        let out = shrink_rgba(&src, 4, 2, 2, 1);
        assert_eq!(out, vec![100, 100, 100, 100, 100, 100]);
    }

    #[test]
    fn choosing_a_wallpaper_writes_the_blurred_file_and_a_solid_one_removes_it() {
        let dir = scratch("build");
        build("lake", &dir).expect("lake blurs");
        let png = dir.join("lock-wallpaper.png");
        let (rgb, w, h) = decode_png(&std::fs::read(&png).unwrap()).expect("what was written is a PNG");
        assert_eq!((w, h), (320, 200), "the working size is the preview's");
        assert_eq!(rgb.len(), w * h * 3);
        assert!(!dir.join("lock-wallpaper.png.tmp").exists(), "no half file left behind");
        assert_eq!(std::fs::read_to_string(dir.join("lock-wallpaper.source")).unwrap(), "lake");

        build("", &dir).expect("a solid colour needs no picture");
        assert!(!png.exists(), "so the lock draws its charcoal rather than the last wallpaper's ghost");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_a_picture_leaves_the_previous_blur_alone() {
        let dir = scratch("bad");
        build("nebula", &dir).unwrap();
        let before = std::fs::read(dir.join("lock-wallpaper.png")).unwrap();
        assert!(build("/nonexistent/wallpaper.jpg", &dir).is_err());
        assert_eq!(std::fs::read(dir.join("lock-wallpaper.png")).unwrap(), before, "a failed change is not a changed lock screen");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_stamp_tells_a_preset_from_an_edited_file() {
        assert_eq!(source_stamp("lake"), "lake");
        let dir = scratch("stamp");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("w.png");
        std::fs::write(&file, b"one").unwrap();
        let first = source_stamp(file.to_str().unwrap());
        std::fs::write(&file, b"another length").unwrap();
        assert_ne!(first, source_stamp(file.to_str().unwrap()), "an edited picture is a new one");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
