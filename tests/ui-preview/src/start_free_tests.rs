//! "Start free, no account" (#1167), drawn by the production first-run page and Settings → Minds,
//! at 1280x800 and at 1024x768. Geometry is read from the picture: a primary button is one flat
//! fill (Theme.primary-fill), so where that fill is, is where the button is.
//!
//! - First-run, "Who should do the thinking?": the card leads and is fully visible without
//!   scrolling (its button, the last thing on it, is whole and inside the window, and the same
//!   width at both sizes), and says what is sent, to whom, on its face.
//! - Settings → Minds: the long red banner wraps and its words end inside the window; "Use Yantrik
//!   models" is never clipped (inside the window, and the same width at both sizes).

use super::*;
use slint::{ModelRc, VecModel};

type Frame = slint::SharedPixelBuffer<slint::Rgb8Pixel>;

fn draw(w: &MinimalSoftwareWindow, width: u32, height: u32) -> Frame {
    w.set_size(slint::PhysicalSize::new(width, height));
    let mut p = Frame::new(width, height);
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        w.request_redraw();
        w.draw_if_needed(|r| {
            r.render(p.make_mut_slice(), width as usize);
        });
    }
    p
}

fn save(p: &Frame, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let f = BufWriter::new(File::create(path)?);
    let mut e = png::Encoder::new(f, p.width(), p.height());
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(p.as_bytes())?;
    Ok(())
}

/// The box (left, top, right, bottom, inclusive) of the pixels within `slack` of `colour`.
fn bounds(p: &Frame, colour: slint::Color, slack: i32) -> Option<(u32, u32, u32, u32)> {
    let near = |a: u8, b: u8| (a as i32 - b as i32).abs() <= slack;
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for (i, px) in p.as_slice().iter().enumerate() {
        if near(px.r, colour.red()) && near(px.g, colour.green()) && near(px.b, colour.blue()) {
            let (x, y) = (i as u32 % p.width(), i as u32 / p.width());
            found = Some(match found {
                None => (x, y, x, y),
                Some((l, t, r, b)) => (l.min(x), t.min(y), r.max(x), b.max(y)),
            });
        }
    }
    found
}

/// The card as the shell publishes it for OVHcloud: the words of `start_free::offers`.
pub(crate) fn offer() -> StartFreeOffer {
    StartFreeOffer {
        account: "free-ovh".into(),
        provider: "OVHcloud AI Endpoints".into(),
        label: "No account needed".into(),
        lead: "What you tell your minds, and what they read on this computer, is sent to OVHcloud AI Endpoints, also when a mind works on its own.".into(),
        detail: "That includes your messages and name, the time, what Yantrik remembers about you (a place you saved too), files, pages, command output and any saved password a mind opens for you, the screen and clipboard when a mind looks at them, and your network's name, open apps, battery, processor, memory, disk and how long the computer has been idle. It applies to every mind on Yantrik models. It is free and rate-limited, so it can be slow or busy. Switch it off in Settings \u{2192} AI & Intelligence."
            .into(),
        button: "Start free with OVHcloud AI Endpoints".into(),
    }
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let still = |name: &str| output.replace(".png", &format!("-{name}.png"));
    let mut first_run_widths = Vec::new();
    // The button's place on the unscrolled page at 1280 wide with bigger text, for the late-card case below.
    let mut big_text_place = None;
    let mut minds_widths = Vec::new();

    // ── First-run ──
    let ui = StartFreePageProbe::new()?;
    ui.set_featured(ModelRc::new(VecModel::from(first_run_tests::featured_none_found())));
    ui.global::<StartFreeState>().set_offers(ModelRc::new(VecModel::from(vec![offer()])));
    // The Hermes sentence (default_mind::BEFORE_NEXT) is on this page before any choice: below the card.
    let hermes = "Choosing one of the cards below also sets up Hermes, your mind: about 3 GB from GitHub and Python's package index.";
    ui.set_default_mind_words(hermes.into());
    ui.show()?;
    for (width, height) in [(1280u32, 800u32), (1024, 768)] {
        ui.set_win_w(width as f32);
        ui.set_win_h(height as f32);
        let p = draw(w, width, height);
        save(&p, &still(&format!("first-run-{width}x{height}")))?;
        let fill = ui.get_button_fill();
        let (l, t, r, b) = bounds(&p, fill, 0).ok_or("the card's button is not on the page")?;
        assert!(b + 8 < height, "{width}x{height}: the card is fully visible without scrolling (its button ends at {b}, the window at {height})");
        assert!(r + 8 < width, "{width}x{height}: the button's right edge {r} is inside the window ({width})");
        assert!(t > 64, "{width}x{height}: the card sits under the page's top bar");
        first_run_widths.push(r - l);
        // The Hermes sentence is under the card, not above it: without it the card is where it was.
        ui.set_default_mind_words("".into());
        let without = bounds(&draw(w, width, height), fill, 0).ok_or("the card's button is gone without the Hermes line")?;
        ui.set_default_mind_words(hermes.into());
        assert_eq!(without.1, t, "{width}x{height}: the Hermes sentence moved the card, so it is drawn above it");
        // The line under "Look on your network" wraps, and a longer one (or bigger text) pushes the card
        // down by the room it needs, whatever its length. Measured in a window so tall that the page cannot
        // scroll, where the card's place is not moved by the scrolling the focus may cause.
        let hint = "Asks the other computers on your network, on Ollama's and llama.cpp's usual ports, for a few seconds. ";
        for scale in [1.0f32, 1.5] {
            ui.set_scale(scale);
            ui.set_win_h(2400.0);
            ui.set_nearby_words("".into());
            let base = bounds(&draw(w, width, 2400), fill, 0).ok_or("the card's button is not on the tall page")?;
            ui.set_nearby_words(hint.repeat(3).trim().into());
            let longer = bounds(&draw(w, width, 2400), fill, 0).ok_or("the card's button is gone when the line wraps")?;
            // Three times the words is at least two more lines (about 17 px each): the card made way.
            assert!(longer.1 >= base.1 + 28, "{width}x{height} at text {scale}: the card did not make way for the wrapped line ({} vs {})", longer.1, base.1);
            // Back at the window's real height the page is still at its top, at any text size: the card is
            // where it is on the tall page (its title is not under the top bar), or below the window's bottom
            // edge. `draw` resizes the window, and Slint scrolls a Flickable to its focused item on a resize:
            // this holds because the focus is on the page while the free card leads, not on an account card
            // that bigger text has pushed below the fold.
            ui.set_nearby_words("".into());
            ui.set_win_h(height as f32);
            let arrived = draw(w, width, height);
            save(&arrived, &still(&format!("first-run-{width}x{height}-text-{scale}")))?;
            if let Some(a) = bounds(&arrived, fill, 0) {
                assert_eq!(a.1, base.1, "{width}x{height} at text {scale}: the page did not start at its top (the card is at {}, not {})", a.1, base.1);
            }
            // With bigger text the button is below that window, so the check above says nothing there. The
            // same in a window that ends just under the button, with the account cards below the fold: the
            // button is in the picture at every text size, and this one is never skipped.
            let snug = base.3 + 24;
            ui.set_win_h(snug as f32);
            let a = bounds(&draw(w, width, snug), fill, 0).ok_or("the card's button is not in the window that ends just under it")?;
            assert_eq!(a.1, base.1, "{width}x{snug} at text {scale}: the page is not at its top (the card is at {}, not {})", a.1, base.1);
            if width == 1280 && scale == 1.5 {
                big_text_place = Some(base);
            }
        }
        ui.set_scale(1.0);
        ui.set_win_h(height as f32);
        ui.set_nearby_words("".into());
        assert!((28..=44).contains(&(b - t + 1)), "{width}x{height}: that is one button's height, not something else filled the same ({})", b - t + 1);
        // Only the card's button is drawn in that fill: with no offer, nothing on the page is.
        let offers = ui.global::<StartFreeState>().get_offers();
        ui.global::<StartFreeState>().set_offers(ModelRc::new(VecModel::from(Vec::<StartFreeOffer>::new())));
        assert!(bounds(&draw(w, width, height), fill, 0).is_none(), "{width}x{height}: something else on the page has the button's fill");
        ui.global::<StartFreeState>().set_offers(offers);
    }
    assert_eq!(first_run_widths[0], first_run_widths[1], "the card's button is not narrowed or clipped at the smaller size");

    // ── The free card arrives after the account cards ──
    // The shell's first account listing can end seconds after the page shows. The account cards are drawn,
    // the first one holds the focus, and the person has gone down the page; then the free card is offered.
    // The page takes the focus itself and goes to its top, so the card's title is not under the top bar.
    let ui = StartFreePageProbe::new()?;
    ui.set_featured(ModelRc::new(VecModel::from(first_run_tests::featured_none_found())));
    ui.set_scale(1.5);
    ui.show()?;
    let (width, height) = (1280u32, 800u32);
    ui.set_win_w(width as f32);
    ui.set_win_h(height as f32);
    let fill = ui.get_button_fill();
    assert!(bounds(&draw(w, width, height), fill, 0).is_none(), "no free card is drawn before one is offered");
    for _ in 0..12 {
        key(w, slint::platform::Key::Tab.into());
    }
    draw(w, width, height);
    ui.global::<StartFreeState>().set_offers(ModelRc::new(VecModel::from(vec![offer()])));
    // The window is never made taller than the page here: that alone would put the page back at its top.
    let base = big_text_place.ok_or("the place of the button with bigger text was not measured")?;
    let snug = base.3 + 24;
    ui.set_win_h(snug as f32);
    let late = draw(w, width, snug);
    save(&late, &still("first-run-late-card-text-1.5"))?;
    let a = bounds(&late, fill, 0).ok_or("the late card's button is not in the window that ends just under it")?;
    assert_eq!(a.1, base.1, "a free card offered after the account cards: the page is not at its top (the card is at {}, not {})", a.1, base.1);

    // ── Settings → Minds ──
    let ui = MindsPageProbe::new()?;
    ui.set_error(
        "Yantrik Companion remembers things about you, and no account may hear them yet. Set up a private lane, or allow sharing in AI & Intelligence \u{2192} AI accounts, then try again."
            .into(),
    );
    let row = HarnessRowData {
        id: "companion".into(),
        name: "Yantrik Companion".into(),
        state: "answering".into(),
        builtin: true,
        active: true,
        attached: true,
        can_assign_provider: true,
        provider_line: "Provider: its own settings".into(),
        ..Default::default()
    };
    ui.set_rows(ModelRc::new(VecModel::from(vec![row])));
    ui.show()?;
    for (width, height) in [(1280u32, 800u32), (1024, 768)] {
        ui.set_win_w(width as f32);
        ui.set_win_h(height as f32);
        let p = draw(w, width, height);
        save(&p, &still(&format!("minds-{width}x{height}")))?;
        let (l, t, r, b) = bounds(&p, ui.get_button_fill(), 0).ok_or("Use Yantrik models is not on the page")?;
        assert!(r + 8 < width, "{width}x{height}: the button's right edge {r} is inside the window ({width})");
        minds_widths.push(r - l);
        assert!((28..=44).contains(&(b - t + 1)), "{width}x{height}: one button's height");
        let (_, _, ink_right, _) = bounds(&p, ui.get_banner_ink(), 24).ok_or("the banner's words are not on the page")?;
        assert!(ink_right + 8 < width, "{width}x{height}: the banner's words end at {ink_right}, inside the window ({width})");
    }
    assert_eq!(minds_widths[0], minds_widths[1], "Use Yantrik models is the same width at both sizes: never clipped");
    println!("PASS start free: the first-run card and Settings -> Minds fit at 1280x800 and 1024x768");
    Ok(())
}
