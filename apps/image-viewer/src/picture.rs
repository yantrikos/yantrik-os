//! What a picture is to this app once it is read, how it is turned, and how it is drawn.
//!
//! Reading and turning are I/O and CPU on pictures that can be tens of megapixels, so both run
//! in jobs (`update.rs`) and come back as plain data: pixels, or an SVG's own text. Only
//! [`Picture::image`] makes a `slint::Image`, on the UI thread, because an image cannot cross
//! threads and its pixel buffer can.

use std::path::Path;
use std::sync::Arc;

use slint::{Rgba8Pixel, SharedPixelBuffer};

use crate::orient::Orientation;

/// A picture's pixels, row by row, shared rather than copied when the model and a job hold them.
#[derive(Clone)]
pub struct Pixels(pub SharedPixelBuffer<Rgba8Pixel>);

impl std::fmt::Debug for Pixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pixels({} × {})", self.0.width(), self.0.height())
    }
}

impl Pixels {
    pub fn width(&self) -> u32 {
        self.0.width()
    }

    pub fn height(&self) -> u32 {
        self.0.height()
    }
}

/// A picture as it was read from its file.
#[derive(Debug, Clone)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub source: Source,
}

#[derive(Clone)]
pub enum Source {
    /// A photograph or any other raster picture, decoded.
    Raster(Pixels),
    /// An SVG's own text: drawn as a vector, so it stays sharp at any zoom, and made into pixels
    /// only when the view turns it.
    Svg(Arc<[u8]>),
}

impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Raster(p) => write!(f, "Raster({p:?})"),
            Source::Svg(text) => write!(f, "Svg({} bytes)", text.len()),
        }
    }
}

/// Whether Slint reads this file as an SVG: its own rule (`load_image_from_path`), so a file
/// this app draws as a vector is exactly one the old window drew as one.
fn is_svg(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.ends_with(".svg") || text.ends_with(".svgz")
}

/// Read and decode the picture at `path`, as `slint::Image::load_from_path` did on the UI
/// thread: the same decoders, the same formats, the same failures. From a job only.
pub fn decode(path: &Path) -> Option<Picture> {
    // Slint's own loader takes a path only as UTF-8.
    path.to_str()?;
    let (image, source) = if is_svg(path) {
        let text: Arc<[u8]> = std::fs::read(path).ok()?.into();
        let image = slint::Image::load_from_svg_data(&text).ok()?;
        (image, Source::Svg(text))
    } else {
        let image = slint::Image::load_from_path(path).ok()?;
        let pixels = image.to_rgba8()?;
        (image, Source::Raster(Pixels(pixels)))
    };
    let size = image.size();
    // A picture with no area draws nothing; the window treats it as no picture at all.
    (size.width > 0 && size.height > 0).then_some(Picture {
        width: size.width,
        height: size.height,
        source,
    })
}

impl Picture {
    /// The picture as drawn when the view is not turned. On the UI thread.
    pub fn image(&self) -> slint::Image {
        match &self.source {
            Source::Raster(pixels) => slint::Image::from_rgba8(pixels.0.clone()),
            Source::Svg(text) => slint::Image::load_from_svg_data(text).unwrap_or_default(),
        }
    }

    /// The pixels the view turns: as decoded, or the SVG drawn at its own size, as the window
    /// always turned one. `None` if an SVG that loaded cannot be drawn. From a job only.
    fn pixels(&self) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
        match &self.source {
            Source::Raster(pixels) => Some(pixels.0.clone()),
            Source::Svg(text) => slint::Image::load_from_svg_data(text).ok()?.to_rgba8(),
        }
    }
}

/// The picture turned and mirrored to `orientation` (orient.rs), as new pixels. The file is
/// never touched. From a job only.
pub fn turn(picture: &Picture, orientation: Orientation) -> Option<Pixels> {
    let source = picture.pixels()?;
    let (turned, width, height) =
        orientation.apply(source.as_slice(), source.width() as usize, source.height() as usize);
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width as u32, height as u32);
    buffer.make_mut_slice().copy_from_slice(&turned);
    Some(Pixels(buffer))
}
