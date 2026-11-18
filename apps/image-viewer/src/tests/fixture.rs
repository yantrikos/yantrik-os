//! What the tests open: a folder of real pictures, written here byte by byte so nothing outside
//! the standard library is needed, and the app started over it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use yantrik_app::mind::fake::FakeMind;
use yantrik_app::{Paths, Services};
use yantrik_testing::TestApp;

use crate::ImageViewer;

/// A folder of the test's own under the temporary directory, removed when the test ends.
pub struct Folder(pub PathBuf);

impl Folder {
    /// `a.png` (3 × 2, every pixel different), `b.png` (2 × 2), `notes.png` (text named like a
    /// picture) and `readme.txt` (not a picture, so not in the folder's list).
    pub fn new(name: &str) -> Folder {
        let dir = std::env::temp_dir().join(format!("yantrik-viewer-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.png"), png(3, 2, pixel)).unwrap();
        std::fs::write(dir.join("b.png"), png(2, 2, pixel)).unwrap();
        std::fs::write(dir.join("notes.png"), "this is text, not an image\n").unwrap();
        std::fs::write(dir.join("readme.txt"), "not a picture").unwrap();
        Folder(dir)
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    pub fn text(&self, name: &str) -> String {
        self.path(name).display().to_string()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The colour of pixel (x, y) in the test pictures: every one different.
pub fn pixel(x: u32, y: u32) -> [u8; 4] {
    [10 * x as u8 + 1, 10 * y as u8 + 1, 7, 255]
}

/// `a.png`'s pixels, row by row.
pub fn a_pixels() -> Vec<[u8; 4]> {
    (0..2).flat_map(|y| (0..3).map(move |x| pixel(x, y))).collect()
}

/// What the window draws now, row by row.
pub fn drawn(app: &TestApp<ImageViewer>) -> (u32, u32, Vec<[u8; 4]>) {
    app.ui(|ui| {
        let image = ui.get_current_image();
        let size = image.size();
        let pixels = image
            .to_rgba8()
            .map(|b| b.as_slice().iter().map(|p| [p.r, p.g, p.b, p.a]).collect())
            .unwrap_or_default();
        (size.width, size.height, pixels)
    })
}

pub fn start() -> (TestApp<ImageViewer>, Arc<FakeMind>) {
    start_with(FakeMind::new(), Vec::new())
}

pub fn start_with(mind: FakeMind, args: Vec<std::ffi::OsString>) -> (TestApp<ImageViewer>, Arc<FakeMind>) {
    let mind = Arc::new(mind);
    let app = TestApp::<ImageViewer>::start_at_with(Paths::temporary("image-viewer"), args, mind.clone(), Services::fakes());
    (app, mind)
}

/// The app with `name` in `folder` open and on screen.
pub fn showing(folder: &Folder, name: &str) -> TestApp<ImageViewer> {
    let (app, _) = start();
    open(&app, &folder.path(name)).unwrap_or_else(|e| panic!("{name} did not open: {e}"));
    app
}

/// The person's `open`, answered once the picture has landed.
pub fn open(app: &TestApp<ImageViewer>, path: &Path) -> Result<serde_json::Value, String> {
    let pending = app.act("open", serde_json::json!({ "path": path.display().to_string() }))?;
    app.run_jobs();
    pending.settle()
}

/// A PNG of `width` × `height` RGBA pixels, stored without compression.
pub fn png(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut raw = Vec::new();
    for y in 0..height {
        raw.push(0); // filter: none
        for x in 0..width {
            raw.extend_from_slice(&pixel(x, y));
        }
    }
    // zlib: a header, stored deflate blocks, and the Adler-32 of what they hold.
    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, block) in blocks.iter().enumerate() {
        let len = block.len() as u16;
        zlib.push(u8::from(i + 1 == blocks.len()));
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, no interlace

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [(&b"IHDR"[..], &ihdr[..]), (b"IDAT", &zlib), (b"IEND", &[])] {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut chunk = kind.to_vec();
        chunk.extend_from_slice(data);
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&crc32(&chunk).to_be_bytes());
    }
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { 0xEDB8_8320 ^ (crc >> 1) } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}
