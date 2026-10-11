//! Stills for a design review: the light theme (default soft-blue accent) and Nightfall, which
//! a test VM cannot be switched into without a pointer. One still per run, because every scene
//! shares the one headless window:
//!
//!   review-still appearance        Settings → Appearance, light
//!   review-still quick-settings    the whole shell with Quick Settings open, light
//!   review-still quick-settings-dark  the same, dark, so the two can be held side by side
//!   review-still lens-composer[-dark]  the chat composer with its "where the words go" line
//!   review-still app-header        Calendar with its header (filled New Event, outline Today…), light
//!   review-still approval          one approval card at its natural height, light
//!   review-still approval-nightfall  the same card on Nightfall (dark, violet, its palette)
//!   review-still approval-dangerous[-nightfall]  a dangerous card, Details and source open
//!   review-still approval-lens[-nightfall]  the card in the Lens at 1280x800, where it is clamped
//!   review-still icons-desktop|icons-launcher  the app tiles, dark and light (icon_stills.rs)
//!   review-still question-erase    a Keep/Erase question in the Lens, with the shell's "Erase removes" line
//!   review-still question-erase-long  the same with thirty spans: the list scrolls and Erase waits
//!
//! Fixture data only; nothing is acted on.
use super::*;
use slint::{ModelRc, SharedString, VecModel};

pub(crate) type Pixels = slint::SharedPixelBuffer<slint::Rgb8Pixel>;

pub(crate) fn save(pixels: &Pixels, path: &str, w: u32, h: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), w, h);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({w}x{h})");
    Ok(())
}

/// Render until two frames in a row match: slide-ins and fades have come to rest.
pub(crate) fn settle(w: &MinimalSoftwareWindow, width: u32, height: u32) -> Pixels {
    let render = || {
        slint::platform::update_timers_and_animations();
        let mut p = Pixels::new(width, height);
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(p.make_mut_slice(), width as usize); });
        p
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut prev = render();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(40));
        let next = render();
        if next.as_slice() == prev.as_slice() || std::time::Instant::now() > deadline {
            return next;
        }
        prev = next;
    }
}

const NIGHTFALL: &str = include_str!("../../../crates/yantrik-design-tokens/themes/nightfall.toml");

fn palette(key: &str) -> slint::Color {
    let line = NIGHTFALL.lines().filter(|l| l.trim_start().starts_with(&format!("{key} ="))).last().unwrap();
    let n = u32::from_str_radix(line.split('"').nth(1).unwrap().trim_start_matches('#'), 16).unwrap();
    slint::Color::from_rgb_u8((n >> 16) as u8, (n >> 8) as u8, n as u8)
}

/// What wire/theme.rs `apply_palette` does for Nightfall: dark, the violet preset, its overrides.
fn nightfall(o: &ThemeOverrides, mode: &ThemeMode, accent: &AccentPreset) {
    mode.set_dark(true);
    accent.set_index(2);
    o.set_bg_deep_override(palette("bg_deep"));
    o.set_bg_surface_override(palette("bg_surface"));
    o.set_bg_card_override(palette("bg_card"));
    o.set_bg_elevated_override(palette("bg_elevated"));
    o.set_amber_override(palette("amber"));
    o.set_cyan_override(palette("cyan"));
    o.set_text_primary_override(palette("text_primary"));
    o.set_text_secondary_override(palette("text_secondary"));
    o.set_text_dim_override(palette("text_dim"));
    o.set_accent_override(palette("accent"));
    o.set_enabled(true);
}

fn lines(rows: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(rows.iter().map(|r| SharedString::from(*r)).collect::<Vec<_>>()))
}

/// calendar.delete_event asked for by a program started from a terminal, as row_for builds it:
/// the app's own description, its first sentence as the summary, and no session offer because
/// the app says it cannot be undone. Since the sign-off of 4 October: the verified fact first and
/// the claim under it, the red "Delete event" button, the consequence rows — and no separate
/// warning, because the undo row already says it (approval_wording.rs).
pub(crate) fn delete_card() -> ApprovalRequest {
    ApprovalRequest {
        id: "appr-review-1".into(),
        agent: "".into(),
        on_behalf: "".into(),
        requester: "design-sweep".into(),
        verified: "a program started from a terminal: sshd-session (pid 2290461)".into(),
        identity: "Caller process confirmed: sshd-session \u{b7} PID 2290461 \u{b7} from a terminal".into(),
        identity_tag: "".into(),
        claim: "Claimed name: \u{201c}design-sweep\u{201d}".into(),
        confirm_label: "Delete event".into(),
        destructive: true,
        what: "Deletes: id: sweep-demo-not-real".into(),
        exactly: "id: sweep-demo-not-real".into(),
        undo: "Undo: not possible, the app says so".into(),
        discrepancies: lines(&[]),
        app: "calendar".into(),
        action: "delete_event".into(),
        summary: "Take an event off the calendar.".into(),
        purpose: "Take an event off the calendar. It is not recoverable".into(),
        caller_says: "".into(),
        grade: "sensitive".into(),
        args: lines(&["id: sweep-demo-not-real"]),
        target: "".into(),
        explained: "".into(),
        warning: "".into(),
        can_session: false,
        decision: "".into(),
        record: "".into(),
        age_text: "Expires in 2 min, then declined".into(),
        decided_at: "".into(),
        session: false,
    }
}

/// system-monitor.kill_process, graded dangerous, asked by a script that names itself after the
/// attached mind: Details and source open by default, a discrepancy in red, and the warning kept
/// beside the undo row because it says more than the row does.
pub(crate) fn dangerous_card() -> ApprovalRequest {
    ApprovalRequest {
        id: "appr-review-2".into(),
        requester: "hermes".into(),
        verified: "python3 sweep.py (pid 31337)".into(),
        identity: "Caller process confirmed: python3.12 \u{b7} PID 31337".into(),
        identity_tag: "".into(),
        claim: "Claimed name: \u{201c}hermes\u{201d}".into(),
        confirm_label: "Kill process".into(),
        destructive: true,
        what: "Kills: force: true; pid: 2210".into(),
        exactly: "force: true; pid: 2210".into(),
        discrepancies: lines(&["Calls itself hermes, but is not the attached mind's process."]),
        app: "system-monitor".into(),
        action: "kill_process".into(),
        summary: "End a running process by pid".into(),
        purpose: "End a running process by pid".into(),
        caller_says: "Tidy up a stuck helper so the build can finish.".into(),
        grade: "dangerous".into(),
        args: lines(&["force: true", "pid: 2210"]),
        warning: "This is graded dangerous \u{2014} it can destroy work or state.".into(),
        age_text: "Expires in 2 min, then declined".into(),
        ..Default::default()
    }
}

/// The card alone at its natural height, on the Lens panel's ground at the panel's width:
/// nothing clamped, so the arguments and the warning are in the picture.
fn approval_card(w: &MinimalSoftwareWindow, output: &str, night: bool) -> Result<(), Box<dyn std::error::Error>> {
    approval_card_of(w, output, night, delete_card())
}

fn approval_card_of(w: &MinimalSoftwareWindow, output: &str, night: bool, card: ApprovalRequest) -> Result<(), Box<dyn std::error::Error>> {
    let ui = ApprovalCardProbe::new()?;
    if night {
        nightfall(&ui.global::<ThemeOverrides>(), &ui.global::<ThemeMode>(), &ui.global::<AccentPreset>());
    } else {
        ui.global::<ThemeMode>().set_dark(false);
        ui.global::<AccentPreset>().set_index(0);
    }
    ui.set_data(card);
    ui.show()?;
    let h = ui.get_card_h().ceil() as u32;
    let h = if h < 50 { 640 } else { h };
    println!("card height {h}");
    w.set_size(slint::PhysicalSize::new(440, h));
    settle(w, 440, h);
    save(&settle(w, 440, h), output, 440, h)
}

/// The same card where the person meets it: in the Lens at 1280x800, where the panel clamps
/// its details to scroll between the pinned head and the pinned buttons.
fn approval(w: &MinimalSoftwareWindow, output: &str, night: bool) -> Result<(), Box<dyn std::error::Error>> {
    let ui = ApprovalLensProbe::new()?;
    if night {
        nightfall(&ui.global::<ThemeOverrides>(), &ui.global::<ThemeMode>(), &ui.global::<AccentPreset>());
    } else {
        ui.global::<ThemeMode>().set_dark(false);
        ui.global::<AccentPreset>().set_index(0);
    }
    ui.set_messages(ModelRc::new(VecModel::from(Vec::<MessageData>::new())));
    ui.set_approvals(ModelRc::new(VecModel::from(vec![delete_card()])));
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(1280, 800));
    settle(w, 1280, 800);
    std::thread::sleep(std::time::Duration::from_millis(400));
    save(&settle(w, 1280, 800), output, 1280, 800)
}

fn appearance(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = SettingsProbe::new()?;
    ui.set_canvas_width(1280.);
    ui.set_canvas_height(800.);
    let all: Vec<_> = ["Appearance", "AI & Intelligence", "Desktop", "Network", "Accounts", "Privacy & Security", "System", "Skills", "Harnesses"]
        .iter().enumerate().map(|(i, s)| SettingsCategoryItem { id: i as i32, label: (*s).into(), icon: "".into() }).collect();
    ui.set_categories(ModelRc::new(VecModel::from(all)));
    ui.set_themes(ModelRc::new(VecModel::from(vec![
        lock_theme_tests::theme_card(include_str!("../../../crates/yantrik-design-tokens/themes/lake.toml"), "lake", "lake"),
        lock_theme_tests::theme_card(NIGHTFALL, "nightfall", "nightfall"),
    ])));
    ui.set_theme("lake".into());
    ui.set_accent("cyan".into());
    ui.set_wallpaper("lake".into());
    ui.set_category(0);
    ui.set_dark(false);
    ui.global::<AccentPreset>().set_index(0);
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(1280, 800));
    settle(w, 1280, 800);
    save(&settle(w, 1280, 800), output, 1280, 800)?;
    // The rest of the page, below the fold.
    w.dispatch_event(slint::platform::WindowEvent::PointerScrolled {
        position: slint::LogicalPosition::new(900., 500.), delta_x: 0., delta_y: -600.,
    });
    std::thread::sleep(std::time::Duration::from_millis(400));
    save(&settle(w, 1280, 800), &output.replace(".png", "-scrolled.png"), 1280, 800)
}

fn quick_settings(w: &MinimalSoftwareWindow, output: &str, dark: bool) -> Result<(), Box<dyn std::error::Error>> {
    let ui = App::new()?;
    ui.global::<ThemeMode>().set_dark(dark);
    ui.global::<AccentPreset>().set_index(0);
    ui.set_current_screen(1);
    ui.set_clock_text("10:24".into());
    ui.set_date_text("Sun 4 Oct".into());
    ui.set_wallpaper_path("lake".into());
    ui.set_mind_mode("ask".into());
    ui.set_mind_mode_label("Ask".into());
    ui.set_settings_dark_mode(dark);
    ui.set_dnd_mode(false);
    quick_settings_tests::laptop_shape(&ui);
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(1280, 800));
    w.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: slint::LogicalPosition::new(300.0, 500.0) });
    settle(w, 1280, 800);
    ui.set_quick_settings_open(true);
    std::thread::sleep(std::time::Duration::from_millis(400));
    save(&settle(w, 1280, 800), output, 1280, 800)
}

/// The chat panel's composer with its "where the words go" line and the "Mode:" chip, as the
/// answering mind's facts give them on VM 520 (Yantrik Mind on Ollama Cloud).
fn lens_composer(w: &MinimalSoftwareWindow, output: &str, dark: bool) -> Result<(), Box<dyn std::error::Error>> {
    let ui = ChatProbe::new()?;
    ui.global::<ThemeMode>().set_dark(dark);
    ui.global::<AccentPreset>().set_index(0);
    ui.set_messages(ModelRc::new(VecModel::from(Vec::<MessageData>::new())));
    ui.set_destination("Sends your message and conversation context to: Ollama Cloud \u{b7} deepseek-v4.1-flash".into());
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(1280, 800));
    settle(w, 1280, 800);
    save(&settle(w, 1280, 800), output, 1280, 800)
}

/// A mind's Keep/Erase question in the Lens: its own words, and under them the shell's line saying
/// exactly what Erase removes. The prompt is one of the review's probes — an inch mark before the
/// quoted name — so the still shows that only the visibly quoted "Priya" is listed. The line is
/// the text `wire::agents::erase_removes` makes for this prompt.
fn question_erase(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    question_erase_of(
        w,
        output,
        "You asked me to forget your sister's name. The 27\" monitor note stays. Forget \"Priya\"?",
        "Erase removes:\n\u{201c}Priya\u{201d}".to_string(),
    )
}

/// Thirty quoted spans: every one is listed, the list is taller than the card allows, so it
/// scrolls, and Erase is disabled — "Scroll to the end of the list to erase." — until it has been
/// read to its end.
fn question_erase_long(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let spans: Vec<String> = (1..=30).map(|i| format!("old address line {i:02}")).collect();
    let prompt = format!("Forget these? {}", spans.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(" "));
    let mut removes = String::from("Erase removes:");
    for s in &spans {
        removes.push_str(&format!("\n\u{201c}{s}\u{201d}"));
    }
    question_erase_of(w, output, &prompt, removes)
}

fn question_erase_of(w: &MinimalSoftwareWindow, output: &str, prompt: &str, removes: String) -> Result<(), Box<dyn std::error::Error>> {
    let ui = ChatProbe::new()?;
    ui.global::<ThemeMode>().set_dark(false);
    ui.global::<AccentPreset>().set_index(0);
    ui.set_messages(ModelRc::new(VecModel::from(Vec::<MessageData>::new())));
    let options: Vec<SharedString> = vec!["Keep".into(), "Erase".into()];
    ui.set_questions(ModelRc::new(VecModel::from(vec![QuestionRequest {
        agent: "yantrik-mind:main".into(),
        mind: "Yantrik Mind".into(),
        request: "forget-1".into(),
        prompt: prompt.into(),
        options: ModelRc::new(VecModel::from(options)),
        asked: "16:16".into(),
        removes: removes.into(),
    }])));
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(1280, 800));
    settle(w, 1280, 800);
    std::thread::sleep(std::time::Duration::from_millis(400));
    save(&settle(w, 1280, 800), output, 1280, 800)
}

fn app_header(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = CalendarStillProbe::new()?;
    ui.global::<ThemeMode>().set_dark(false);
    ui.global::<AccentPreset>().set_index(0);
    // October 2026 starts on a Thursday: four leading blanks, 31 days, blanks to 42. Today is the 4th.
    let busy = [2, 4, 7, 9, 14, 15, 21, 28];
    let days: Vec<CalendarDay> = (0..42).map(|i| {
        let n = i as i32 - 3;
        let real = (1..=31).contains(&n);
        CalendarDay {
            day_number: if real { n } else { 0 },
            is_today: n == 4,
            is_selected: n == 4,
            is_current_month: real,
            has_events: real && busy.contains(&n),
            event_count: if n == 4 { 3 } else if real && busy.contains(&n) { 1 } else { 0 },
        }
    }).collect();
    ui.set_days(ModelRc::new(VecModel::from(days)));
    let ev = |id: i32, title: &str, time: &str, all_day: bool, rgb: (u8, u8, u8)| CalendarEvent {
        id,
        title: title.into(),
        date_text: "Oct 4, 2026".into(),
        time_text: time.into(),
        color: slint::Color::from_rgb_u8(rgb.0, rgb.1, rgb.2),
        is_all_day: all_day,
    };
    ui.set_events(ModelRc::new(VecModel::from(vec![
        ev(1, "Farmers' market", "All day", true, (0x2b, 0x64, 0xa8)),
        ev(2, "Call with Mum", "11:00 – 11:30", false, (0x73, 0x38, 0xb4)),
        ev(3, "Review the 0.4 release notes", "16:00 – 17:00", false, (0x2b, 0x64, 0xa8)),
    ])));
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(1280, 800));
    settle(w, 1280, 800);
    save(&settle(w, 1280, 800), output, 1280, 800)
}

pub fn run(w: &MinimalSoftwareWindow, output: &str, which: &str) -> Result<(), Box<dyn std::error::Error>> {
    match which {
        "appearance" => appearance(w, output),
        "quick-settings" => quick_settings(w, output, false),
        "quick-settings-dark" => quick_settings(w, output, true),
        "lens-composer" => lens_composer(w, output, false),
        "lens-composer-dark" => lens_composer(w, output, true),
        "app-header" => app_header(w, output),
        "approval" => approval_card(w, output, false),
        "approval-nightfall" => approval_card(w, output, true),
        "approval-dangerous" => approval_card_of(w, output, false, dangerous_card()),
        "approval-dangerous-nightfall" => approval_card_of(w, output, true, dangerous_card()),
        "approval-lens" => approval(w, output, false),
        "approval-lens-nightfall" => approval(w, output, true),
        "icons-desktop" | "icons-launcher" => super::icon_stills::run(w, output, which),
        "question-erase" => question_erase(w, output),
        "question-erase-long" => question_erase_long(w, output),
        other => Err(format!("unknown still {other:?}").into()),
    }
}
