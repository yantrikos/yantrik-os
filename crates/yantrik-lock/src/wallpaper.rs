//! The lock screen's picture, read only once the session is locked, and only if it is small.
//!
//! The file is the shell's pre-blurred copy under `~/.cache/yantrik/`, which any process of the
//! person's can replace, a mind with file tools included. Decoded before the lock was taken, a
//! huge PNG, a decompression bomb or a decoder panic killed this client before it ever locked, at
//! every restart, and left the session behind only the shell's own screen: the one other windows
//! can be raised over (#313; security review of #601). So: the lock first, the solid charcoal
//! drawn, and then this, which refuses a link, anything that is not a plain file, a file larger
//! than [`MOST_BYTES`], and a PNG wider or taller than [`MOST_SIDE`] by its own header, before
//! decoding a byte of picture. It decodes the bytes it read, so the file cannot be swapped between
//! the check and the load, and a panic in the decoder is caught: the worst a bad file does is
//! leave the charcoal.

use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// Larger than any blurred wallpaper the shell writes (a blur compresses to a few hundred KB).
pub const MOST_BYTES: u64 = 8 * 1024 * 1024;
/// Wider or taller than any screen this lock is drawn on.
pub const MOST_SIDE: u32 = 4096;

/// The picture, or `None` for anything this will not show.
pub fn load(path: &Path) -> Option<slint::Image> {
    let bytes = read_small(path)?;
    // The shell writes a PNG; the header says how big before any decoding.
    let (w, h) = yantrik_ui_kit::lock_shared::picture_size(&bytes)?;
    if w == 0 || h == 0 || w > MOST_SIDE || h > MOST_SIDE {
        return None;
    }
    let pixels = std::panic::catch_unwind(|| decode(&bytes, w, h)).ok()??;
    Some(slint::Image::from_rgba8(pixels))
}

/// The file's bytes: a plain file, not followed through a link, at most [`MOST_BYTES`].
fn read_small(path: &Path) -> Option<Vec<u8>> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > MOST_BYTES {
        return None;
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    // Read no more than the cap even if the file grew after the check.
    file.take(MOST_BYTES + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= MOST_BYTES).then_some(bytes)
}

/// Decode to RGBA8, with the decoder held to what the header promised.
fn decode(bytes: &[u8], w: u32, h: u32) -> Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>> {
    let mut decoder = png::Decoder::new_with_limits(
        std::io::Cursor::new(bytes),
        png::Limits { bytes: (MOST_SIDE as usize) * (MOST_SIDE as usize) * 8 },
    );
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut buf).ok()?;
    if (frame.width, frame.height) != (w, h) {
        return None;
    }
    let data = &buf[..frame.buffer_size()];
    let mut out = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w, h);
    let px = out.make_mut_slice();
    let channels = match frame.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => return None, // EXPAND turns palettes into RGB; anything left is unexpected
    };
    for (dst, src) in px.iter_mut().zip(data.chunks_exact(channels)) {
        *dst = match channels {
            4 => slint::Rgba8Pixel::new(src[0], src[1], src[2], src[3]),
            3 => slint::Rgba8Pixel::new(src[0], src[1], src[2], 0xff),
            2 => slint::Rgba8Pixel::new(src[0], src[0], src[0], src[1]),
            _ => slint::Rgba8Pixel::new(src[0], src[0], src[0], 0xff),
        };
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(&vec![40u8; (w * h * 3) as usize]).unwrap();
        out
    }

    fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-lock-wp-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lock-wallpaper.png");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn a_small_png_loads() {
        let path = temp("ok", &png_bytes(64, 40));
        let image = load(&path).expect("a small PNG is shown");
        assert_eq!((image.size().width, image.size().height), (64, 40));
    }

    #[test]
    fn a_header_claiming_a_huge_picture_is_refused_before_decoding() {
        let mut bytes = png_bytes(8, 8);
        // Rewrite IHDR's width to 100,000 (the CRC no longer matches: never reached).
        bytes[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        assert!(load(&temp("huge", &bytes)).is_none());
    }

    #[test]
    fn garbage_a_link_and_a_missing_file_leave_the_charcoal() {
        assert!(load(&temp("junk", b"not a picture at all, just words")).is_none());
        let real = temp("target", &png_bytes(8, 8));
        let link = real.with_file_name("link.png");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(load(&link).is_none(), "a symlink is not followed");
        assert!(load(Path::new("/nonexistent/lock-wallpaper.png")).is_none());
    }

    #[test]
    fn a_truncated_png_is_refused_not_a_panic() {
        let bytes = png_bytes(32, 32);
        assert!(load(&temp("cut", &bytes[..bytes.len() / 2])).is_none());
    }
}
