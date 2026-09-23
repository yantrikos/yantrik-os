#[path="../../../crates/yantrik-ui/src/models.rs"]
mod production_models;
mod files_tests;
mod settings_tests;
mod agents_tests;
mod mind_panel_tests;
mod recipes_tests;
mod formations_tests;
mod lens_tests;
mod overview_tests;
use slint::{
    platform::{
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
        Platform, WindowAdapter,
    },
    ComponentHandle,
};
use std::{fs::File, io::BufWriter, rc::Rc};
slint::include_modules!();

struct Headless(Rc<MinimalSoftwareWindow>);
impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let output = args.get(1).map(String::as_str).unwrap_or("preview.png");
    let width: u32 = args.get(2).map(|s| s.parse()).transpose()?.unwrap_or(1280);
    let height: u32 = args.get(3).map(|s| s.parse()).transpose()?.unwrap_or(800);
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless(window.clone())))?;
    if args.iter().any(|a| a == "verify-settings") { return settings_tests::run(&window, output, width, height); }
    if args.iter().any(|a| a == "verify-files") { return files_tests::run(&window); }
    if args.iter().any(|a| a == "verify-agents") { return agents_tests::run(&window, output); }
    if args.iter().any(|a| a == "verify-agents-catalog") { return agents_tests::run_catalog(&window, output); }
    if args.iter().any(|a| a == "verify-mind-panel") { return mind_panel_tests::run(&window, output); }
    if args.iter().any(|a| a == "verify-recipes") { return recipes_tests::run(&window, output); }
    if args.iter().any(|a| a == "verify-formations") { return formations_tests::run(&window, output); }
    if args.iter().any(|a| a == "verify-lens-agents") { return agents_tests::run_lens(&window, output); }
    if args.iter().any(|a| a == "verify-lens-answers") { return lens_tests::run(&window, output); }
    if args.iter().any(|a| a == "lens-answer") { return lens_tests::run_lens(&window, output); }
    if args.iter().any(|a| a == "verify-agents-overview") { return overview_tests::run(&window, output); }
    if args.iter().any(|a| a == "verify-idle") {
        let probe = TerminalProbe::new()?;
        probe.show()?;
        window.set_size(slint::PhysicalSize::new(800, 600));
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(800, 600);
        for _ in 0..8 {
            slint::platform::update_timers_and_animations();
            window.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), 800); });
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let mut redraws = 0;
        for _ in 0..15 {
            slint::platform::update_timers_and_animations();
            if window.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), 800); }) {
                redraws += 1;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert_eq!(redraws, 0, "Idle Terminal must not continuously repaint");
        println!("PASS: idle Terminal produced zero requested redraws over 1.5 seconds");
        return Ok(());
    }
    if args.iter().any(|a| a == "verify-overflow") {
        let probe = HeaderProbe::new()?;
        probe.show()?;
        window.set_size(slint::PhysicalSize::new(800, 600));
        let redraw = || {
            slint::platform::update_timers_and_animations();
            let mut buffer = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(800, 600);
            window.request_redraw();
            window.draw_if_needed(|r| { r.render(buffer.make_mut_slice(), 800); });
        };
        redraw();
        click(&window, 750., 24.);
        redraw();
        key(&window, "\n".into());
        assert_eq!(probe.get_count(), 0, "Disabled menu entry cannot run");
        key(&window, slint::platform::Key::DownArrow.into());
        key(&window, "\n".into());
        assert_eq!(probe.get_chosen(), "export", "Arrow/Enter reaches overflow action");
        assert_eq!(probe.get_count(), 1);
        click(&window, 750., 24.);
        redraw();
        click(&window, 660., 180.);
        assert_eq!(probe.get_chosen(), "close", "Last menu action is reachable by pointer");
        assert_eq!(probe.get_count(), 2);
        click(&window, 750., 24.);
        redraw();
        key(&window, slint::platform::Key::Escape.into());
        click(&window, 660., 180.);
        assert_eq!(probe.get_count(), 2, "Escape dismisses the menu");
        println!("PASS: overflow keyboard activation, disabled guard, last action and Escape");
        return Ok(());
    }
    if args.iter().any(|a| a == "verify-controls") {
        let probe = ControlsProbe::new()?;
        probe.show()?;
        window.set_size(slint::PhysicalSize::new(360, 180));
        probe.invoke_focus_action();
        key(&window, "\n".into());
        assert_eq!(
            probe.get_activations(),
            1,
            "Enter activates a focused button"
        );
        key(&window, " ".into());
        assert_eq!(
            probe.get_activations(),
            2,
            "Space activates a focused button"
        );
        probe.set_disabled(true);
        key(&window, "\n".into());
        click(&window, 306., 116.);
        assert_eq!(
            probe.get_activations(),
            2,
            "Disabled buttons cannot activate"
        );
        assert_eq!(
            probe.get_input_value(),
            "Keep me",
            "Disabled inputs cannot be cleared"
        );
        probe.set_disabled(false);
        probe.set_loading(true);
        probe.invoke_focus_action();
        key(&window, "\n".into());
        assert_eq!(
            probe.get_activations(),
            2,
            "Loading buttons cannot activate"
        );
        probe.set_loading(false);
        probe.invoke_focus_action();
        key(&window, "\n".into());
        assert_eq!(
            probe.get_activations(),
            3,
            "Re-enabled buttons remain usable"
        );
        click(&window, 306., 116.);
        assert_eq!(probe.get_input_value(), "", "Enabled input clear works");
        println!(
            "PASS: Enter, Space, disabled/loading activation, disabled and enabled input clear"
        );
        return Ok(());
    }
    let ui = Preview::new()?;
    ui.set_canvas_width(width as f32);
    ui.set_canvas_height(height as f32);
    ui.set_light(args.iter().any(|a| a == "light"));
    ui.set_scene(
        args.get(4)
            .filter(|a| {
                ["notes", "files", "files-grid", "files-empty", "files-home", "files-home-list", "settings"].contains(&a.as_str())
            })
            .cloned()
            .unwrap_or_default()
            .into(),
    );
    ui.set_app_view(args.iter().any(|a| a == "app"));
    ui.set_actual_desktop(args.iter().any(|a| a == "desktop" || a == "agent"));
    ui.set_agent_view(args.iter().any(|a| a == "agent"));
    ui.set_launcher_open(args.iter().any(|a| a == "launcher"));
    ui.set_empty_launcher(args.iter().any(|a| a == "empty-launcher"));
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(width, height));
    slint::platform::update_timers_and_animations();
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
    window.request_redraw();
    assert!(window.draw_if_needed(|renderer| {
        renderer.render(pixels.make_mut_slice(), width as usize);
    }));
    if args.iter().any(|a| a == "verify-apps") {
        assert_eq!((width, height), (1280, 800), "App probe requires 1280x800");
        let redraw = || {
            slint::platform::update_timers_and_animations();
            let mut buffer = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
            window.request_redraw();
            window.draw_if_needed(|renderer| {
                renderer.render(buffer.make_mut_slice(), width as usize);
            });
        };
        ui.set_scene("settings".into());
        redraw();
        click(&window, 400., 240.);
        assert_eq!(
            ui.get_app_action_count(),
            0,
            "Selecting the active theme does not toggle it"
        );
        click(&window, 900., 240.);
        assert_eq!(
            ui.get_app_action(),
            "theme",
            "Light theme dispatches the existing callback"
        );
        assert_eq!(ui.get_app_action_count(), 1);
        redraw();
        key(&window, " ".into());
        assert_eq!(
            ui.get_app_action_count(),
            1,
            "Space on the now-active theme is idempotent"
        );
        click(&window, 490., 510.);
        assert_eq!(ui.get_app_action(), "wallpaper:first-light");
        key(&window, "\n".into());
        assert_eq!(
            ui.get_app_action_count(),
            3,
            "Wallpaper choices also activate with Enter"
        );
        ui.set_scene("files".into());
        redraw();
        let previous = ui.get_app_action_count();
        click(&window, 62., 74.);
        assert_eq!(
            ui.get_app_action_count(),
            previous,
            "Disabled Forward does not navigate"
        );
        click(&window, 26., 74.);
        assert_eq!(ui.get_app_action(), "back");
        assert_eq!(ui.get_app_action_count(), previous + 1);
        ui.set_scene("files-grid".into());
        redraw();
        click(&window, 260., 230.);
        key(&window, "\n".into());
        assert_eq!(
            ui.get_app_action(),
            "folder:Projects",
            "First grid cell navigates its own folder"
        );
        click(&window, 418., 230.);
        key(&window, "\n".into());
        assert_eq!(
            ui.get_app_action(),
            "folder:Personal",
            "Second grid cell is independently reachable"
        );
        // Resize the real component and verify the second grid row at 800.
        ui.set_canvas_width(800.);
        ui.set_canvas_height(600.);
        window.set_size(slint::PhysicalSize::new(800, 600));
        redraw();
        click(&window, 718., 340.);
        assert_eq!(
            ui.get_app_action(),
            "file:Budget.csv",
            "Compact grid wraps and selects the correct file after resizing"
        );
        ui.set_scene("notes".into());
        redraw();
        assert!(ui.get_notes_library() && !ui.get_notes_assistant());
        click(&window, 360., 24.);
        redraw();
        assert!(ui.get_notes_assistant() && !ui.get_notes_library(), "Assistant preserves compact writing space");
        click(&window, 675., 24.);
        assert_eq!(ui.get_app_action(), "save-note", "Save remains reachable beside the open assistant");
        click(&window, 260., 24.);
        redraw();
        assert!(ui.get_notes_library() && !ui.get_notes_assistant(), "Library also preserves compact writing space");
        ui.set_canvas_width(1280.);
        window.set_size(slint::PhysicalSize::new(1280, 600));
        redraw();
        click(&window, 410., 24.);
        redraw();
        assert!(ui.get_notes_library() && ui.get_notes_assistant(), "Wide Notes can show both panels");
        ui.set_canvas_width(800.);
        window.set_size(slint::PhysicalSize::new(800, 600));
        redraw();
        assert!(!ui.get_notes_library() && ui.get_notes_assistant(), "Resizing keeps the assistant without squeezing the editor");
        println!("PASS: Notes panel toggles, reachable Save, and resize handling");
        println!("PASS: theme selection/idempotence, keyboard wallpaper activation, disabled navigation, adaptive file grid hit targets");
        return Ok(());
    }
    if args.iter().any(|a| a == "verify-launcher") {
        ui.set_launcher_open(true);
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(pixels.make_mut_slice(), width as usize);
        });
        key(&window, slint::platform::Key::RightArrow.into());
        key(&window, "\n".into());
        assert_eq!(
            ui.get_launched(),
            "notes",
            "Right + Enter launches the selected app"
        );
        assert!(!ui.get_launcher_open(), "Launching closes the launcher");
        ui.set_launcher_open(true);
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(pixels.make_mut_slice(), width as usize);
        });
        key(&window, "n".into());
        key(&window, slint::platform::Key::Tab.into());
        key(&window, "\n".into());
        assert_eq!(
            ui.get_launched(),
            "notes",
            "Tab selects a result while search contains text"
        );
        assert!(!ui.get_launcher_open());
        ui.set_empty_launcher(true);
        ui.set_launcher_open(true);
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(pixels.make_mut_slice(), width as usize);
        });
        key(&window, "\n".into());
        assert!(
            ui.get_launcher_open(),
            "Enter with no results does not launch or close"
        );
        key(&window, slint::platform::Key::Escape.into());
        assert!(
            !ui.get_launcher_open(),
            "Escape dismisses the reopened launcher"
        );
        println!("PASS: launcher arrow/Tab selection, launch, reopen, empty results and Escape");
        return Ok(());
    }
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(output)?), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()?
        .write_image_data(pixels.as_bytes())?;
    println!("Rendered {output} ({width}×{height})");
    Ok(())
}

fn key(window: &MinimalSoftwareWindow, text: slint::SharedString) {
    use slint::platform::WindowEvent;
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}

fn click(window: &MinimalSoftwareWindow, x: f32, y: f32) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(x, y);
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}
