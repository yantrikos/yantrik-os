//! Sharpness spike (desk/sharpness-spike): the shell's own scenes — app.slint's `App` on the
//! desktop, Files and Agents, filled with the same fixtures as the verify-* probes — drawn by
//! Slint's software renderer or, with the opt-in `skia` feature, by Skia's CPU raster, at any
//! logical size and scale factor, and timed. Measurement only: nothing here is asserted.
//!
//! ```text
//! sharp <software|skia> <desktop|desktop-menu|files|files-window|agents> <out.png> <logical-w> <logical-h> <scale> <dark|light> [frames]
//! ```
//!
//! Prints one `SHARP ...` line: the first frame, and the median of `frames` full repaints (every
//! timed frame redraws the whole window, the cost of a full-screen change on the VM), and the
//! process's peak RSS.
//!
//! The software path renders into the pixel the winit software backend uses (softbuffer's
//! 0x00RRGGBB, blended through premultiplied RGBA), and the Skia path into the same BGRA layout
//! the winit Skia software surface hands Skia, so the timings compare with what the VM would do.
use super::*;
use slint::platform::software_renderer::{PremultipliedRgbaColor, TargetPixel};
use slint::platform::WindowEvent;
use slint::{ModelRc, VecModel};
use std::time::{Duration, Instant};

#[derive(Copy, Clone, Default)]
#[repr(transparent)]
struct XrgbPixel(u32);

impl TargetPixel for XrgbPixel {
    fn blend(&mut self, color: PremultipliedRgbaColor) {
        let v = self.0;
        let mut x = PremultipliedRgbaColor { red: (v >> 16) as u8, green: (v >> 8) as u8, blue: v as u8, alpha: (v >> 24) as u8 };
        x.blend(color);
        self.0 = (x.alpha as u32) << 24 | (x.red as u32) << 16 | (x.green as u32) << 8 | x.blue as u32;
    }
    fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self(0xff000000 | (r as u32) << 16 | (g as u32) << 8 | b as u32)
    }
    fn background() -> Self {
        Self(0)
    }
}

/// One way of turning the scene into pixels.
trait Target {
    fn window(&self) -> &slint::Window;
    /// One full repaint; returns how long the renderer took.
    fn frame(&self) -> Duration;
    /// The last frame as packed RGB.
    fn rgb(&self) -> Vec<u8>;
}

struct Software {
    window: Rc<MinimalSoftwareWindow>,
    pixels: std::cell::RefCell<Vec<XrgbPixel>>,
    width: usize,
}

impl Target for Software {
    fn window(&self) -> &slint::Window {
        self.window.window()
    }
    fn frame(&self) -> Duration {
        let mut pixels = self.pixels.borrow_mut();
        let size = self.window.size();
        pixels.resize((size.width * size.height) as usize, XrgbPixel::default());
        self.window.request_redraw();
        let start = Instant::now();
        self.window.draw_if_needed(|r| {
            r.render(&mut pixels[..], self.width);
        });
        start.elapsed()
    }
    fn rgb(&self) -> Vec<u8> {
        self.pixels.borrow().iter().flat_map(|p| [(p.0 >> 16) as u8, (p.0 >> 8) as u8, p.0 as u8]).collect()
    }
}

#[cfg(feature = "skia")]
mod skia {
    use i_slint_core::partial_renderer::DirtyRegion;
    use i_slint_renderer_skia::software_surface::{RenderBuffer, SoftwareSurface};
    use i_slint_renderer_skia::{skia_safe, SkiaRenderer, SkiaSharedContext};
    use slint::platform::{Renderer, WindowAdapter, WindowEvent};
    use std::cell::{Cell, RefCell};
    use std::num::NonZeroU32;
    use std::rc::{Rc, Weak};

    /// Memory the Skia raster canvas draws into: BGRA, as softbuffer hands it over on the VM.
    /// Buffer age 0 on every frame, so each frame is a full repaint.
    #[derive(Default)]
    pub struct MemBuffer {
        pub pixels: RefCell<Vec<u8>>,
    }

    impl RenderBuffer for MemBuffer {
        fn with_buffer(
            &self,
            _window: &slint::Window,
            size: slint::PhysicalSize,
            render: &mut dyn FnMut(NonZeroU32, NonZeroU32, skia_safe::ColorType, u8, &mut [u8]) -> Result<Option<DirtyRegion>, slint::PlatformError>,
        ) -> Result<(), slint::PlatformError> {
            let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return Ok(()) };
            let mut pixels = self.pixels.borrow_mut();
            pixels.resize((size.width * size.height * 4) as usize, 0);
            render(w, h, skia_safe::ColorType::BGRA8888, 0, &mut pixels[..])?;
            Ok(())
        }
    }

    /// A window adapter with no window: Slint's own SkiaRenderer on its software surface,
    /// the same pairing the linuxkms backend makes for a DRM dumb buffer.
    pub struct SkiaWindow {
        window: slint::Window,
        pub renderer: SkiaRenderer,
        size: Cell<slint::PhysicalSize>,
        pub buffer: Rc<MemBuffer>,
    }

    impl SkiaWindow {
        pub fn new() -> Rc<Self> {
            let buffer = Rc::new(MemBuffer::default());
            let surface = SoftwareSurface::from(buffer.clone());
            Rc::new_cyclic(|weak: &Weak<Self>| {
                let weak: Weak<dyn WindowAdapter> = weak.clone();
                Self {
                    window: slint::Window::new(weak),
                    renderer: SkiaRenderer::new_with_surface(&SkiaSharedContext::default(), Box::new(surface)),
                    size: Cell::new(slint::PhysicalSize::new(0, 0)),
                    buffer,
                }
            })
        }
    }

    impl WindowAdapter for SkiaWindow {
        fn window(&self) -> &slint::Window {
            &self.window
        }
        fn size(&self) -> slint::PhysicalSize {
            self.size.get()
        }
        fn set_size(&self, size: slint::WindowSize) {
            let scale = self.window.scale_factor();
            self.size.set(size.to_physical(scale));
            self.window.dispatch_event(WindowEvent::Resized { size: size.to_logical(scale) });
        }
        fn renderer(&self) -> &dyn Renderer {
            &self.renderer
        }
        fn request_redraw(&self) {}
    }

    pub struct Headless(pub Rc<SkiaWindow>);
    impl slint::platform::Platform for Headless {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            Ok(self.0.clone())
        }
    }

    pub struct Skia(pub Rc<SkiaWindow>);
    impl super::Target for Skia {
        fn window(&self) -> &slint::Window {
            self.0.window()
        }
        fn frame(&self) -> std::time::Duration {
            let start = std::time::Instant::now();
            let _drawn = self.0.renderer.render().expect("skia render");
            start.elapsed()
        }
        fn rgb(&self) -> Vec<u8> {
            self.0.buffer.pixels.borrow().chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]).collect()
        }
    }
}

/// The desktop, Files at home and Agents, as the shell draws them: `App`, every screen, the
/// status bar, the taskbar and the mind panel, with the verify-* fixtures.
fn scene(name: &str, dark: bool) -> Result<App, Box<dyn std::error::Error>> {
    let ui = App::new()?;
    super::mind_panel_tests::fill(&ui);
    ui.global::<ThemeMode>().set_dark(dark);
    match name {
        "desktop" => ui.set_current_screen(1),
        // The mode menu open over the desktop: a floating surface with a drop shadow.
        "desktop-menu" => {
            ui.set_current_screen(1);
            ui.set_mind_menu_confirming(false);
            ui.set_mind_menu_open(true);
        }
        "files" | "files-window" => {
            let dir = |name: &str, count: i32, known: bool, changed: &str| FileEntry {
                name: name.into(),
                is_dir: true,
                item_count: count,
                count_known: known,
                count_reason: if known { "".into() } else { "permission denied".into() },
                changed_text: changed.into(),
                ..Default::default()
            };
            let file = |name: &str, size: &str, icon: &str, changed: &str| FileEntry {
                name: name.into(),
                size_text: size.into(),
                icon_char: icon.into(),
                changed_text: changed.into(),
                ..Default::default()
            };
            ui.set_file_browser_path("~".into());
            ui.set_file_breadcrumbs(ModelRc::new(VecModel::from(vec![BreadcrumbSegment { label: "Home".into(), full_path: "~".into() }])));
            ui.set_file_browser_entries(ModelRc::new(VecModel::from(vec![
                dir("Documents", 12, true, "2 h ago"),
                dir("Downloads", 31, true, "25 min ago"),
                dir("Music", 0, true, "2 mo ago"),
                dir("Pictures", 148, true, "3 d ago"),
                dir("Projects", 6, true, "just now"),
                dir("Shared", 0, false, "5 d ago"),
                dir("Videos", 1, true, "1 y ago"),
                file("notes.md", "4.1 KB", "≡", "8 min ago"),
                file("budget.csv", "8.7 KB", "◇", "1 d ago"),
                file("landscape.png", "2.4 MB", "▣", "3 h ago"),
            ])));
            let place = |id: &str, label: &str, path: &str| FilePlaceData { id: id.into(), label: label.into(), path: path.into() };
            ui.set_file_places(ModelRc::new(VecModel::from(vec![
                place("home", "Home", "~"),
                place("documents", "Documents", "~/Documents"),
                place("downloads", "Downloads", "~/Downloads"),
                place("pictures", "Pictures", "~/Pictures"),
                place("music", "Music", "~/Music"),
                place("videos", "Videos", "~/Videos"),
                place("projects", "Projects", "~/Projects"),
            ])));
            let recent = |name: &str, size: &str, changed: &str, icon: &str| FileRecentData {
                name: name.into(),
                size_text: size.into(),
                changed_text: changed.into(),
                icon_char: icon.into(),
            };
            ui.set_file_recent(ModelRc::new(VecModel::from(vec![
                recent("notes.md", "4.1 KB", "8 min ago", "≡"),
                recent("landscape.png", "2.4 MB", "3 h ago", "▣"),
                recent("budget.csv", "8.7 KB", "1 d ago", "◇"),
            ])));
            ui.set_file_free_space_text("42.8 GB available".into());
            ui.set_file_can_go_back(true);
            ui.set_file_grid_view(true);
            ui.set_current_screen(8);
            if name == "files-window" {
                // A floating window: its rounded gradient frame and its drop shadow are drawn,
                // which a maximized one does not have.
                ui.set_window_maximized(false);
                ui.set_window_x(140.);
                ui.set_window_y(70.);
                ui.set_window_w(860.);
                ui.set_window_h(560.);
            }
        }
        "agents" => {
            super::agents_tests::fill(&ui.global::<AgentsState>(), false);
            ui.set_current_screen(34);
        }
        other => return Err(format!("unknown sharp scene {other:?} (desktop, desktop-menu, files, files-window, agents)").into()),
    }
    Ok(ui)
}

fn peak_rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmHWM:")).and_then(|l| l.split_whitespace().nth(1)?.parse().ok()))
        .unwrap_or(0)
}

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let usage = "sharp <software|skia> <desktop|desktop-menu|files|files-window|agents> <out.png> <logical-w> <logical-h> <scale> <dark|light> [frames]";
    let arg = |i: usize| args.get(i).map(String::as_str).ok_or(usage);
    let renderer = arg(2)?;
    let name = arg(3)?;
    let output = arg(4)?;
    let (lw, lh): (f32, f32) = (arg(5)?.parse()?, arg(6)?.parse()?);
    let scale: f32 = arg(7)?.parse()?;
    let dark = arg(8)? != "light";
    let frames: usize = args.get(9).map(|s| s.parse()).transpose()?.unwrap_or(25);
    let (pw, ph) = ((lw * scale).round() as u32, (lh * scale).round() as u32);

    let target: Box<dyn Target> = match renderer {
        "software" => {
            let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
            slint::platform::set_platform(Box::new(Headless(window.clone())))?;
            Box::new(Software { window, pixels: Default::default(), width: pw as usize })
        }
        #[cfg(feature = "skia")]
        "skia" => {
            let window = skia::SkiaWindow::new();
            slint::platform::set_platform(Box::new(skia::Headless(window.clone())))?;
            Box::new(skia::Skia(window))
        }
        #[cfg(not(feature = "skia"))]
        "skia" => return Err("built without the `skia` feature: cargo run --features skia ...".into()),
        other => return Err(format!("unknown renderer {other:?}; {usage}").into()),
    };

    let ui = scene(name, dark)?;
    ui.show()?;
    let window = target.window();
    window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
    window.set_size(slint::PhysicalSize::new(pw, ph));

    slint::platform::update_timers_and_animations();
    let first = target.frame();
    // Past the shell windows' 200 ms geometry animation, then the frame that is saved.
    std::thread::sleep(Duration::from_millis(300));
    slint::platform::update_timers_and_animations();
    target.frame();

    let mut times: Vec<Duration> = (0..frames).map(|_| target.frame()).collect();
    times.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let pick = |q: f64| ms(times[((times.len() - 1) as f64 * q).round() as usize]);

    let rgb = target.rgb();
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(output)?), pw, ph);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&rgb)?;

    println!(
        "SHARP renderer={renderer} scene={name} theme={} logical={lw}x{lh} scale={scale} physical={pw}x{ph} frames={frames} first_ms={:.2} median_ms={:.2} p10_ms={:.2} p90_ms={:.2} peak_rss_kb={} out={output}",
        if dark { "dark" } else { "light" },
        ms(first),
        pick(0.5),
        pick(0.1),
        pick(0.9),
        peak_rss_kb(),
    );
    ui.hide()?;
    Ok(())
}
