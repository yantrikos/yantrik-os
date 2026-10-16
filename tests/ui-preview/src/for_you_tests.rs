//! The For you inbox and Settings → Minds → Initiative (design/initiative-inbox-2026-10-16.md):
//! the production components drawn from fixture cards and settings, in both themes, at 1280×800
//! and in a narrow window, and driven from the keyboard alone. Every control a card or a row has
//! must be reachable with Tab and pressed with Enter, and every press must say what it was for.
use super::*;
use slint::{ModelRc, VecModel};
use std::cell::RefCell;

fn card(id: &str, title: &str, body: &str, family: &str, action: &str, interrupt: bool) -> NoticeCardData {
    NoticeCardData {
        id: id.into(),
        title: title.into(),
        body: body.into(),
        why: "You asked for this reminder. · task:41".into(),
        sender: "\u{201c}Yantrik Mind\u{201d} says the caller \u{b7} yantrik-mind (pid 812) by this machine".into(),
        family: family.into(),
        action_label: action.into(),
        interrupt,
        unread: true,
        note: "".into(),
    }
}

fn family(id: &str, label: &str, description: &str, setting: &str, can_be_off: bool) -> FamilyRowData {
    FamilyRowData { family: id.into(), label: label.into(), description: description.into(), setting: setting.into(), can_be_off }
}

fn render(w: &MinimalSoftwareWindow, width: u32, height: u32) -> slint::SharedPixelBuffer<slint::Rgb8Pixel> {
    slint::platform::update_timers_and_animations();
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
    w.request_redraw();
    w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
    pixels
}

fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    Ok(())
}

pub fn run(w: &MinimalSoftwareWindow, output: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let ui = ForYouProbe::new()?;
    let inbox = ui.global::<InboxState>();
    inbox.set_cards(ModelRc::new(VecModel::from(vec![
        card("41", "Stretch", "You asked me at 14:02 to remind you to stretch.", "Reminders", "Open note", true),
        // A mind's words are drawn as they are: markup in them is text, not formatting.
        card("42", "Backup finished", "42 files, 1.2 GB, no errors. <b>Nothing</b> was **skipped**.", "Jobs you handed over", "", false),
        card("43", "A paper you were reading has a new version", "Version 3 of the paper in ~/Papers changes the second table and the conclusions.", "Things a mind noticed", "Open arxiv.org", false),
    ])));
    inbox.set_more(2);
    let state = ui.global::<InitiativeState>();
    state.set_families(ModelRc::new(VecModel::from(vec![
        family("reminder", "Reminders", "Reminders you asked a mind for. Never off: you asked for them.", "on", false),
        family("delegated", "Jobs you handed over", "The results of jobs you handed to a mind. Never off: you asked for them.", "on", false),
        family("machine", "This machine", "Something about this machine a mind thinks you should know.", "inbox_only", true),
        family("calendar", "Calendar", "Your events, before they start.", "on", true),
        family("discovery", "Things a mind noticed", "Something a mind came across that it thinks you would want.", "off", true),
        family("start_of_day", "Start of day", "A short note as your day begins.", "on", true),
    ])));
    state.set_quiet_hours("22:00–07:00".into());

    let pressed: std::rc::Rc<RefCell<Vec<String>>> = Default::default();
    {
        let p = pressed.clone();
        inbox.on_open_action(move |id| p.borrow_mut().push(format!("open:{id}")));
        let p = pressed.clone();
        inbox.on_feedback(move |id, kind| p.borrow_mut().push(format!("feedback:{id}:{kind}")));
        let p = pressed.clone();
        inbox.on_open_centre(move || p.borrow_mut().push("centre".into()));
        let p = pressed.clone();
        state.on_set_quiet(move |on| p.borrow_mut().push(format!("quiet:{on}")));
        let p = pressed.clone();
        state.on_set_family(move |f, s| p.borrow_mut().push(format!("family:{f}:{s}")));
        let p = pressed.clone();
        state.on_next_quiet_hours(move || p.borrow_mut().push("quiet-hours".into()));
    }

    ui.show()?;
    let narrow = width < 900;
    let pages: &[i32] = if narrow { &[1, 2] } else { &[0] };
    for &page in pages {
        ui.set_page(page);
        for light in [false, true] {
            ui.set_light(light);
            w.set_size(slint::PhysicalSize::new(width, height));
            let pixels = render(w, width, height);
            let name = format!("{}{}{}.png", output.trim_end_matches(".png"), ["", "-inbox", "-initiative"][page as usize], if light { "-light" } else { "" });
            save(&pixels, &name, width, height)?;
            // Both themes draw something on the page: not the window's ground alone.
            let ground = pixels.as_slice()[0];
            let differing = pixels.as_slice().iter().filter(|p| **p != ground).count();
            assert!(differing > (width * height / 50) as usize, "page {page} (light: {light}) drew almost nothing");
        }
    }

    // Keyboard alone, on the widest page: Tab to every control, Enter on each.
    ui.set_page(if narrow { 1 } else { 0 });
    ui.set_light(false);
    render(w, width, height);
    for _ in 0..60 {
        key(w, slint::SharedString::from(slint::platform::Key::Tab));
        key(w, slint::SharedString::from(slint::platform::Key::Return));
        render(w, width, height);
    }
    let seen = pressed.borrow().clone();
    let mut want = vec!["open:41", "feedback:41:useful", "feedback:41:not_useful", "feedback:41:not_now", "feedback:41:never_family", "feedback:41:inbox_only", "feedback:41:wrong", "open:43", "centre"];
    if !narrow {
        want.extend(["quiet-hours", "family:discovery:off", "family:machine:inbox_only"]);
        assert!(seen.iter().any(|s| s.starts_with("quiet:")), "the quiet initiative switch is not reachable: {seen:?}");
        // A reminder's row offers no Off.
        assert!(!seen.iter().any(|s| s == "family:reminder:off" || s == "family:delegated:off"), "{seen:?}");
    }
    for want in want {
        assert!(seen.iter().any(|s| s == want), "{want} was not reachable from the keyboard; pressed: {seen:?}");
    }
    assert!(!seen.iter().any(|s| s == "open:42"), "a card with no button has no button: {seen:?}");
    println!("PASS for you: cards, feedback and initiative rows drawn in both themes at {width}x{height}, every control reached by Tab and Enter");
    Ok(())
}
