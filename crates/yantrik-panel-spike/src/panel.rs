//! One Wayland surface drawn by one Slint window: the software renderer into a wl_shm buffer.

use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor};
use slint::platform::WindowEvent;
use smithay_client_toolkit::shell::wlr_layer::LayerSurface;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::SlotPool;
use wayland_client::protocol::wl_shm;

/// What the loop counts, to answer "what does this cost" with measurements instead of guesses.
#[derive(Default, Debug, Clone, Copy)]
pub struct Stats {
    pub renders: u64,
    pub pixels_painted: u64,
    pub commits: u64,
}

pub struct Panel {
    pub window: Rc<MinimalSoftwareWindow>,
    pub layer: LayerSurface,
    pub size: (u32, u32),
    /// Kept between frames: `ReusedBuffer` repaints only the dirty region into it.
    pixels: Vec<PremultipliedRgbaColor>,
    pool: SlotPool,
    pub stats: Stats,
}

impl Panel {
    pub fn new(window: Rc<MinimalSoftwareWindow>, layer: LayerSurface, shm: &smithay_client_toolkit::shm::Shm) -> Self {
        let pool = SlotPool::new(4096, shm).expect("a shm pool");
        Panel { window, layer, size: (0, 0), pixels: Vec::new(), pool, stats: Stats::default() }
    }

    /// The compositor said how big the surface is. A new size means a fresh, fully dirty buffer.
    pub fn resize(&mut self, w: u32, h: u32) {
        if (w, h) == self.size || w == 0 || h == 0 {
            return;
        }
        self.size = (w, h);
        self.pixels = vec![PremultipliedRgbaColor::default(); (w * h) as usize];
        self.window.set_size(slint::PhysicalSize::new(w, h));
        self.window.request_redraw();
    }

    /// Paint if Slint says something changed; otherwise do nothing at all (no buffer, no commit).
    /// Returns whether a frame was committed.
    pub fn draw_if_needed(&mut self) -> bool {
        let (w, h) = self.size;
        if w == 0 || h == 0 {
            return false;
        }
        let mut dirty = None;
        let pixels = &mut self.pixels;
        self.window.draw_if_needed(|renderer| {
            dirty = Some(renderer.render(pixels, w as usize));
        });
        let Some(region) = dirty else { return false };
        let (origin, size) = (region.bounding_box_origin(), region.bounding_box_size());
        self.stats.renders += 1;
        self.stats.pixels_painted += (size.width * size.height) as u64;

        let stride = w as i32 * 4;
        let Ok((buffer, canvas)) = self.pool.create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888) else {
            return false;
        };
        // Argb8888 is little-endian B,G,R,A in memory and premultiplied, which is what the
        // software renderer produces. The slot may be fresh memory, so copy the whole frame; the
        // saving of `ReusedBuffer` is the RENDER, and the compositor is told only the damage.
        for (px, out) in self.pixels.iter().zip(canvas.chunks_exact_mut(4)) {
            out.copy_from_slice(&[px.blue, px.green, px.red, px.alpha]);
        }
        let surface = self.layer.wl_surface();
        if buffer.attach_to(surface).is_err() {
            return false;
        }
        surface.damage_buffer(origin.x, origin.y, size.width as i32, size.height as i32);
        self.layer.commit();
        self.stats.commits += 1;
        true
    }

    pub fn pointer(&self, event: WindowEvent) {
        self.window.dispatch_event(event);
    }
}
