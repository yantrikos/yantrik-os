//! The picture on an icon.
//!
//! One thumbnail is cut for each (path, mtime, size) and kept in the cache, so a picture is
//! decoded once, not every time the desktop is drawn, and a changed file gets a new one because
//! its key changed. The work is done by whoever calls [`load_or_make`], which the shell runs on a
//! worker thread; nothing here is called from the UI thread and nothing polls.
//!
//! Covered: PNG and JPEG, cut by downscale, and `.blend`, from the preview Blender saves inside
//! the file. A PDF has no renderer on this OS (no poppler, mupdf or pdfium in the tree or in the
//! image's package list), so it gets its glyph; so does anything else, and so does a file this
//! module will not decode (see the limits below).
//!
//! The file named is the person's, and its contents may be anything, so the decoder is held to
//! limits: a source over [`MAX_SOURCE_BYTES`] is not opened, an image wider or taller than
//! [`MAX_DIMENSION`] or needing more than [`MAX_DECODE_BYTES`] to decode is refused, and a
//! `.blend` is read only as far as [`BLEND_SCAN_BYTES`].

use std::io::Read;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use image::{DynamicImage, ImageFormat, RgbaImage};
use sha2::{Digest, Sha256};

/// The longest side of a thumbnail, in pixels: 2x the icon tile, so it is crisp on a 2x screen.
pub const EDGE: u32 = 128;
/// Part of every key. Bump it and every old thumbnail is simply never found again.
const VERSION: &str = "1";
pub const MAX_SOURCE_BYTES: u64 = 48 << 20;
pub const MAX_DIMENSION: u32 = 16_384;
pub const MAX_DECODE_BYTES: u64 = 256 << 20;
/// A saved preview is at most 256x256 RGBA, which sits right after the header.
pub const BLEND_SCAN_BYTES: u64 = 1 << 20;

/// Which files get a thumbnail at all, by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Image,
    Blend,
}

pub fn source_of(name: &str) -> Option<Source> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    match ext.to_ascii_lowercase().as_str() {
        "png" | "jpg" | "jpeg" => Some(Source::Image),
        "blend" => Some(Source::Blend),
        _ => None,
    }
}

/// A thumbnail as pixels, row by row, four bytes each (R, G, B, A).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumb {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum ThumbError {
    #[error("not a plain file")]
    NotAFile,
    #[error("no thumbnail is made for this kind of file")]
    Unsupported,
    #[error("the file is larger than a thumbnail is cut from ({0} bytes)")]
    TooBig(u64),
    #[error("the picture could not be decoded: {0}")]
    Decode(String),
    #[error("the file holds no preview")]
    NoPreview,
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// What a thumbnail is filed under: the file's path, when it was last changed and how big it is,
/// and this module's version. The same file unchanged is the same key; any edit is a new one.
pub fn key(path: &Path, mtime: SystemTime, size: u64) -> String {
    use std::os::unix::ffi::OsStrExt;
    let since = mtime.duration_since(UNIX_EPOCH).unwrap_or_default();
    let mut h = Sha256::new();
    h.update(VERSION.as_bytes());
    h.update([0]);
    h.update(path.as_os_str().as_bytes());
    h.update([0]);
    h.update(since.as_secs().to_le_bytes());
    h.update(since.subsec_nanos().to_le_bytes());
    h.update([0]);
    h.update(size.to_le_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Where the thumbnail with this key lives under `root`: a folder of the first two characters, so
/// no folder holds more than a few hundred files.
pub fn cache_file(root: &Path, key: &str) -> PathBuf {
    root.join(&key[..2]).join(format!("{key}.png"))
}

/// The thumbnail for `path`: from the cache when it is there, otherwise cut now and kept. Blocks
/// on disk and decoding, so call it off the UI thread.
pub fn load_or_make(root: &Path, path: &Path) -> Result<Thumb, ThumbError> {
    let source = path.file_name().and_then(|n| source_of(&n.to_string_lossy())).ok_or(ThumbError::Unsupported)?;
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(ThumbError::NotAFile);
    }
    let cached = cache_file(root, &key(path, meta.modified().unwrap_or(UNIX_EPOCH), meta.len()));
    if let Some(hit) = read_cached(&cached) {
        return Ok(hit);
    }
    if meta.len() > MAX_SOURCE_BYTES && source == Source::Image {
        return Err(ThumbError::TooBig(meta.len()));
    }
    let thumb = match source {
        Source::Image => cut_image(path)?,
        Source::Blend => cut_blend(path)?,
    };
    // A cache that cannot be written is a slower desktop, not a missing picture.
    if let Err(e) = keep(&cached, &thumb) {
        tracing::debug!(error = %e, "A thumbnail could not be cached");
    }
    Ok(thumb)
}

fn read_cached(file: &Path) -> Option<Thumb> {
    let img = image::open(file).ok()?.to_rgba8();
    (img.width() <= EDGE && img.height() <= EDGE).then(|| Thumb { width: img.width(), height: img.height(), rgba: img.into_raw() })
}

fn keep(file: &Path, thumb: &Thumb) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        // 0700: a thumbnail is a small copy of something the person may keep private.
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let img = RgbaImage::from_raw(thumb.width, thumb.height, thumb.rgba.clone()).ok_or(std::io::ErrorKind::InvalidData)?;
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, ImageFormat::Png).map_err(std::io::Error::other)?;
    crate::atomic::write(file, png.get_ref())
}

/// Open a regular file for reading and check it is one on the descriptor itself, not on the name:
/// a name that was a file a moment ago can be a FIFO now, and a read of that would stall the one
/// worker every picture waits on. `O_NONBLOCK` makes the open itself return at once.
fn open_regular(path: &Path) -> Result<std::fs::File, ThumbError> {
    let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY).open(path)?;
    if !file.metadata()?.is_file() {
        return Err(ThumbError::NotAFile);
    }
    Ok(file)
}

fn cut_image(path: &Path) -> Result<Thumb, ThumbError> {
    let file = open_regular(path)?;
    if file.metadata()?.len() > MAX_SOURCE_BYTES {
        return Err(ThumbError::TooBig(file.metadata()?.len()));
    }
    let mut reader = image::ImageReader::new(std::io::BufReader::new(file)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| ThumbError::Decode(e.to_string()))?;
    Ok(shrink(img))
}

/// Scaled down to fit [`EDGE`], aspect kept; a picture already that small is left as it is.
fn shrink(img: DynamicImage) -> Thumb {
    let img = if img.width() > EDGE || img.height() > EDGE { img.thumbnail(EDGE, EDGE) } else { img };
    let rgba = img.to_rgba8();
    Thumb { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() }
}

fn cut_blend(path: &Path) -> Result<Thumb, ThumbError> {
    let mut bytes = Vec::new();
    open_regular(path)?.take(BLEND_SCAN_BYTES).read_to_end(&mut bytes)?;
    let preview = blend_preview(&bytes).ok_or(ThumbError::NoPreview)?;
    let img = RgbaImage::from_raw(preview.width, preview.height, preview.rgba).ok_or(ThumbError::NoPreview)?;
    Ok(shrink(DynamicImage::ImageRgba8(img)))
}

/// The preview Blender saves in a `.blend`: a block coded `TEST` holding a width, a height and
/// that many RGBA pixels, bottom row first. Blender writes it first, before the scene, so it is
/// found in the first few blocks. A file written with compression (gzip or zstd) starts
/// differently and has no preview this can reach without decompressing: `None`.
///
/// Both header layouts are read: the classic one (`BLENDER-v305`: pointer size, byte order,
/// three-digit version, then 20- or 24-byte block headers) and the one from Blender 5
/// (`BLENDER17-01v0500`, 32-byte block headers).
pub fn blend_preview(bytes: &[u8]) -> Option<Thumb> {
    if !bytes.starts_with(b"BLENDER") {
        return None;
    }
    let (head, block, big_endian, ptr) = match *bytes.get(7)? {
        b'_' | b'-' => {
            let ptr = if bytes[7] == b'-' { 8 } else { 4 };
            (12, 4 + 4 + ptr + 4 + 4, *bytes.get(8)? == b'V', ptr)
        }
        // "17" is the header's length; "-01" the block format; then the byte order and version.
        b'0'..=b'9' => (17, 32, *bytes.get(12)? == b'V', 8),
        _ => return None,
    };
    let large = block == 32;
    let int = |at: usize| -> Option<i32> {
        let b: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
        Some(if big_endian { i32::from_be_bytes(b) } else { i32::from_le_bytes(b) })
    };
    let mut at = head;
    for _ in 0..64 {
        let code = bytes.get(at..at + 4)?;
        // Classic: code, len, old pointer, SDNA, count. Large: code, SDNA, old pointer, len (8), count (8).
        let len = if large {
            let b: [u8; 8] = bytes.get(at + 4 + 4 + ptr..at + 4 + 4 + ptr + 8)?.try_into().ok()?;
            if big_endian { i64::from_be_bytes(b) } else { i64::from_le_bytes(b) }
        } else {
            i64::from(int(at + 4)?)
        };
        let data = at + block;
        match code {
            b"TEST" => {
                let (w, h) = (int(data)?, int(data + 4)?);
                if !(1..=512).contains(&w) || !(1..=512).contains(&h) || len != 8 + i64::from(w) * i64::from(h) * 4 {
                    return None;
                }
                let (w, h) = (w as usize, h as usize);
                let pixels = bytes.get(data + 8..data + 8 + w * h * 4)?;
                // Bottom row first in the file; the picture is the right way up.
                let mut rgba = Vec::with_capacity(pixels.len());
                for row in (0..h).rev() {
                    rgba.extend_from_slice(&pixels[row * w * 4..(row + 1) * w * 4]);
                }
                return Some(Thumb { width: w as u32, height: h as u32, rgba });
            }
            b"ENDB" | b"DNA1" => return None,
            _ => {}
        }
        at = data.checked_add(usize::try_from(len).ok()?)?;
    }
    None
}

/// Keep the cache under `max_bytes` by removing the oldest thumbnails (and only thumbnails: files
/// this module wrote, in its own two-level folders). Returns how many bytes went.
pub fn trim(root: &Path, max_bytes: u64) -> std::io::Result<u64> {
    let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    for dir in std::fs::read_dir(root)? {
        let dir = dir?.path();
        if !dir.is_dir() || dir.file_name().is_none_or(|n| n.len() != 2) {
            continue;
        }
        for f in std::fs::read_dir(&dir)? {
            let f = f?;
            let p = f.path();
            if p.extension().is_some_and(|e| e == "png") {
                if let Ok(m) = f.metadata() {
                    if m.is_file() {
                        files.push((m.modified().unwrap_or(UNIX_EPOCH), m.len(), p));
                    }
                }
            }
        }
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    let mut freed = 0;
    for (_, len, p) in files {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total -= len;
            freed += len;
        }
    }
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn png(path: &Path, w: u32, h: u32) {
        RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x % 256) as u8, (y % 256) as u8, 90, 255])).save(path).unwrap();
    }

    #[test]
    fn only_png_jpeg_and_blend_are_cut() {
        assert_eq!(source_of("a.PNG"), Some(Source::Image));
        assert_eq!(source_of("a.jpeg"), Some(Source::Image));
        assert_eq!(source_of("scene.blend"), Some(Source::Blend));
        for other in ["a.pdf", "a.gif", "a.svg", "a", ".png", "a.blend1"] {
            assert_eq!(source_of(other), None, "{other}");
        }
    }

    /// The cache's keying: one file unchanged is one key; a changed mtime, size or path is another.
    #[test]
    fn a_key_follows_path_mtime_and_size_and_nothing_else() {
        let t0 = UNIX_EPOCH + Duration::new(1_700_000_000, 5);
        let a = key(Path::new("/h/Desktop/a.png"), t0, 100);
        assert_eq!(a, key(Path::new("/h/Desktop/a.png"), t0, 100), "stable");
        assert_ne!(a, key(Path::new("/h/Desktop/a.png"), t0 + Duration::from_nanos(1), 100), "mtime to the nanosecond");
        assert_ne!(a, key(Path::new("/h/Desktop/a.png"), t0, 101), "size");
        assert_ne!(a, key(Path::new("/h/Desktop/b.png"), t0, 100), "path");
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
        let f = cache_file(Path::new("/c"), &a);
        assert_eq!(f, PathBuf::from(format!("/c/{}/{a}.png", &a[..2])));
    }

    #[test]
    fn a_picture_is_cut_once_and_the_second_ask_is_the_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let (cache, src) = (tmp.path().join("cache"), tmp.path().join("big.png"));
        png(&src, 640, 320);
        let first = load_or_make(&cache, &src).unwrap();
        assert_eq!((first.width, first.height), (EDGE, EDGE / 2), "scaled down, aspect kept");
        assert_eq!(first.rgba.len(), (first.width * first.height * 4) as usize);
        let kept: Vec<_> = walk(&cache);
        assert_eq!(kept.len(), 1, "one thumbnail filed: {kept:?}");
        // Delete the source's pixels but keep its identity: the cache answers without decoding it.
        let meta = std::fs::metadata(&src).unwrap();
        let again = {
            // Same key (path, mtime, size) means the cached file is read, not the source.
            let k = key(&src, meta.modified().unwrap(), meta.len());
            std::fs::write(&src, vec![0u8; meta.len() as usize]).unwrap();
            let f = std::fs::File::options().write(true).open(&src).unwrap();
            f.set_modified(meta.modified().unwrap()).unwrap();
            assert_eq!(k, key(&src, std::fs::metadata(&src).unwrap().modified().unwrap(), meta.len()));
            load_or_make(&cache, &src).unwrap()
        };
        assert_eq!(again, first, "the cache, not a decode of the (now garbage) source");
    }

    #[test]
    fn an_edited_picture_gets_a_new_thumbnail() {
        let tmp = tempfile::tempdir().unwrap();
        let (cache, src) = (tmp.path().join("cache"), tmp.path().join("p.png"));
        png(&src, 200, 200);
        let old = load_or_make(&cache, &src).unwrap();
        png(&src, 100, 50);
        let f = std::fs::File::options().write(true).open(&src).unwrap();
        f.set_modified(SystemTime::now() + Duration::from_secs(5)).unwrap();
        let new = load_or_make(&cache, &src).unwrap();
        assert_ne!(old, new);
        assert_eq!((new.width, new.height), (100, 50), "already small: not scaled up");
        assert_eq!(walk(&cache).len(), 2);
    }

    #[test]
    fn jpeg_is_cut_too() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("p.jpg");
        image::RgbImage::from_fn(300, 150, |x, y| image::Rgb([x as u8, y as u8, 7])).save(&src).unwrap();
        let t = load_or_make(&tmp.path().join("c"), &src).unwrap();
        assert_eq!((t.width, t.height), (EDGE, EDGE / 2));
    }

    #[test]
    fn a_file_that_is_not_the_picture_it_says_is_a_glyph_not_a_crash() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("fake.png");
        std::fs::write(&src, b"this is not a png").unwrap();
        assert!(matches!(load_or_make(&tmp.path().join("c"), &src), Err(ThumbError::Decode(_))));
        std::fs::write(tmp.path().join("g.png"), b"GIF89a\x01\x00\x01\x00\x00\x00\x00;").unwrap();
        assert!(load_or_make(&tmp.path().join("c"), &tmp.path().join("g.png")).is_err());
        assert!(matches!(load_or_make(tmp.path(), &tmp.path().join("doc.pdf")), Err(ThumbError::Unsupported)));
    }

    /// A name that has become a FIFO since it was listed must not stall the worker: the open
    /// returns at once and the descriptor says it is not a file.
    #[test]
    fn a_fifo_is_refused_without_waiting_for_a_writer() {
        let tmp = tempfile::tempdir().unwrap();
        let fifo = tmp.path().join("pipe.png");
        let c = std::ffi::CString::new(std::os::unix::ffi::OsStrExt::as_bytes(fifo.as_os_str())).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        assert!(matches!(open_regular(&fifo), Err(ThumbError::NotAFile)));
        assert!(matches!(cut_image(&fifo), Err(ThumbError::NotAFile)));
        assert!(matches!(cut_blend(&tmp.path().join("pipe.png")), Err(ThumbError::NotAFile)));
    }

    #[test]
    fn the_cache_folders_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let (cache, src) = (tmp.path().join("cache"), tmp.path().join("p.png"));
        png(&src, 10, 10);
        load_or_make(&cache, &src).unwrap();
        let shard = std::fs::read_dir(&cache).unwrap().next().unwrap().unwrap().path();
        assert_eq!(std::fs::metadata(&shard).unwrap().permissions().mode() & 0o777, 0o700);
        let file = std::fs::read_dir(&shard).unwrap().next().unwrap().unwrap().path();
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn a_folder_or_a_missing_file_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("d.png")).unwrap();
        assert!(matches!(load_or_make(tmp.path(), &tmp.path().join("d.png")), Err(ThumbError::NotAFile)));
        assert!(matches!(load_or_make(tmp.path(), &tmp.path().join("nope.png")), Err(ThumbError::Io(_))));
    }

    /// A decompression bomb: a PNG whose header claims far more than the decoder may allocate.
    #[test]
    fn a_picture_claiming_a_giant_size_is_refused_before_it_is_decoded() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("bomb.png");
        let mut data = Vec::new();
        {
            let enc = png_encoder(&mut data, 60_000, 60_000);
            enc.write_header().unwrap();
            // Header only; no image data follows, which is itself invalid, and must not matter.
        }
        std::fs::write(&src, &data).unwrap();
        assert!(matches!(load_or_make(&tmp.path().join("c"), &src), Err(ThumbError::Decode(_))));
    }

    fn png_encoder(out: &mut Vec<u8>, w: u32, h: u32) -> ::png::Encoder<'_, &mut Vec<u8>> {
        let mut e = ::png::Encoder::new(out, w, h);
        e.set_color(::png::ColorType::Rgba);
        e.set_depth(::png::BitDepth::Eight);
        e
    }

    fn blend_v0(ptr8: bool, big: bool, w: i32, h: i32, pixels: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"BLENDER");
        b.push(if ptr8 { b'-' } else { b'_' });
        b.push(if big { b'V' } else { b'v' });
        b.extend_from_slice(b"305");
        let int = |v: i32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let len = 8 + pixels.len() as i32;
        b.extend_from_slice(b"TEST");
        b.extend_from_slice(&int(len));
        b.extend_from_slice(&vec![0u8; if ptr8 { 8 } else { 4 }]);
        b.extend_from_slice(&int(0));
        b.extend_from_slice(&int(1));
        b.extend_from_slice(&int(w));
        b.extend_from_slice(&int(h));
        b.extend_from_slice(pixels);
        b.extend_from_slice(b"ENDB");
        b
    }

    #[test]
    fn a_blend_gives_up_the_preview_it_saved_right_way_up() {
        // 2x2: bottom row (red, green) is stored first.
        let px = [255, 0, 0, 255, 0, 255, 0, 255, /* top row */ 0, 0, 255, 255, 9, 9, 9, 255];
        for (ptr8, big) in [(true, false), (false, false), (true, true), (false, true)] {
            let t = blend_preview(&blend_v0(ptr8, big, 2, 2, &px)).unwrap_or_else(|| panic!("ptr8={ptr8} big={big}"));
            assert_eq!((t.width, t.height), (2, 2));
            assert_eq!(&t.rgba[..8], &px[8..16], "the top row first");
            assert_eq!(&t.rgba[8..], &px[..8]);
        }
    }

    #[test]
    fn a_blend_from_blender_five_is_read_too() {
        let (w, h) = (1i32, 1i32);
        let mut b = Vec::new();
        b.extend_from_slice(b"BLENDER17-01v0500");
        b.extend_from_slice(b"TEST");
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        b.extend_from_slice(&12i64.to_le_bytes());
        b.extend_from_slice(&1i64.to_le_bytes());
        b.extend_from_slice(&w.to_le_bytes());
        b.extend_from_slice(&h.to_le_bytes());
        b.extend_from_slice(&[1, 2, 3, 4]);
        let t = blend_preview(&b).unwrap();
        assert_eq!((t.width, t.height, t.rgba.as_slice()), (1, 1, &[1u8, 2, 3, 4][..]));
    }

    #[test]
    fn a_blend_with_no_preview_or_a_lying_one_is_a_glyph() {
        assert!(blend_preview(b"\x1f\x8b\x08 gzip, compressed").is_none());
        assert!(blend_preview(b"BLENDER").is_none());
        assert!(blend_preview(b"not blender at all").is_none());
        // A header that claims a 512x512 picture and has no pixels behind it.
        let mut lie = blend_v0(true, false, 2, 2, &[0; 16]);
        lie[36..40].copy_from_slice(&512i32.to_le_bytes());
        assert!(blend_preview(&lie).is_none());
        // Blocks that never end in TEST: the walk is bounded, not endless.
        let mut b = b"BLENDER-v305".to_vec();
        for _ in 0..200 {
            b.extend_from_slice(b"DATA");
            b.extend_from_slice(&0i32.to_le_bytes());
            b.extend_from_slice(&[0u8; 8 + 8]);
        }
        assert!(blend_preview(&b).is_none());
    }

    #[test]
    fn a_blend_file_is_cut_from_its_own_preview_and_cached() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("summit.blend");
        std::fs::write(&src, blend_v0(true, false, 1, 1, &[10, 20, 30, 255])).unwrap();
        let t = load_or_make(&tmp.path().join("c"), &src).unwrap();
        assert_eq!(t.rgba, [10, 20, 30, 255]);
        assert_eq!(walk(&tmp.path().join("c")).len(), 1);
        std::fs::write(tmp.path().join("old.blend"), b"BLENDER-v280 and then nothing").unwrap();
        assert!(matches!(load_or_make(&tmp.path().join("c"), &tmp.path().join("old.blend")), Err(ThumbError::NoPreview)));
    }

    #[test]
    fn the_cache_is_trimmed_oldest_first_and_only_its_own_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join("ab")).unwrap();
        for (i, name) in ["old", "mid", "new"].iter().enumerate() {
            let p = root.join("ab").join(format!("{name}.png"));
            std::fs::write(&p, vec![0u8; 100]).unwrap();
            let f = std::fs::File::options().write(true).open(&p).unwrap();
            f.set_modified(UNIX_EPOCH + Duration::from_secs(1000 + i as u64)).unwrap();
        }
        std::fs::write(root.join("notes.png"), "not in a cache folder").unwrap();
        std::fs::write(root.join("ab").join("keep.txt"), "not a thumbnail").unwrap();
        assert_eq!(trim(root, 150).unwrap(), 200);
        assert!(!root.join("ab/old.png").exists() && !root.join("ab/mid.png").exists());
        assert!(root.join("ab/new.png").exists() && root.join("notes.png").exists() && root.join("ab/keep.txt").exists());
        assert_eq!(trim(root, 150).unwrap(), 0);
    }

    fn walk(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(root) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    out.extend(walk(&p));
                } else {
                    out.push(p);
                }
            }
        }
        out
    }
}
