//! Settings → AI → Free AI accounts (components/free_ai_card.slint), drawn with one row in every
//! state, at its widest. Checks that it draws, stays within 600px, and that every press reports
//! a row's own id and nothing else: no callback on this card can carry a key.

use super::*;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

fn row(id: &str, name: &str, state: &str, words: &str, detail: &str, trains: bool, values: &[(&str, &str, bool)]) -> FreeAiRow {
    FreeAiRow {
        id: id.into(),
        name: name.into(),
        state: state.into(),
        state_words: words.into(),
        offer: "1,000 requests and 200K tokens a day per model. One account per person (its terms).".into(),
        needs: "Needs: an account · about 3 min".into(),
        detail: detail.into(),
        trains,
        on: state != "off",
        values: ModelRc::new(VecModel::from(
            values.iter().map(|(id, label, done)| FreeAiValue { id: (*id).into(), label: (*label).into(), r#where: "the key page".into(), done: *done }).collect::<Vec<_>>(),
        )),
        opt_out: if trains { "Its training can be turned off in its own settings; the label stays either way.".into() } else { "".into() },
    }
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (600u32, 1800u32);
    let ui = FreeAiProbe::new()?;
    let g = ui.global::<FreeAiState>();
    let rows = vec![
        row("groq", "Groq", "ready", "ready", "Ready · key ends in 7f3k", false, &[("groq", "API key", true)]),
        row("cloudflare", "Cloudflare Workers AI", "waiting", "waiting for 2 values", "Paste each value from the page, in order.", false, &[("cloudflare_account", "Account ID", true), ("cloudflare", "API token", false)]),
        row("ovh", "OVHcloud AI Endpoints", "open", "no account needed", "", false, &[]),
        row("openrouter", "OpenRouter", "rejected", "key rejected", "OpenRouter refused this key (401). Copy it again from the key page; it may have been cut short or revoked. Nothing was saved.", true, &[("openrouter", "API key", false)]),
        row("gemini", "Google Gemini", "skipped", "", "", true, &[]),
        row("zai", "Z.ai", "not-set-up", "not set up", "", true, &[]),
        row("mistral", "Mistral", "sign-up-opened", "sign-up opened", "The sign-up page is open in the browser. When the account exists, open the key page.", true, &[]),
        row("kilo", "Kilo Gateway", "off", "off", "", true, &[]),
    ];
    let ids: Vec<String> = rows.iter().map(|r| r.id.to_string()).chain(["cloudflare_account".to_string()]).collect();
    let (trains, clean): (Vec<FreeAiRow>, Vec<FreeAiRow>) = rows.iter().cloned().partition(|r| r.trains);
    g.set_clean_rows(ModelRc::new(VecModel::from(clean)));
    g.set_trains_rows(ModelRc::new(VecModel::from(trains)));
    g.set_rows(ModelRc::new(VecModel::from(rows)));
    g.set_summary("3 of 8 ready · 4 to set up · 1 skipped".into());
    g.set_next_line("Next: Z.ai · an account · about 4 min".into());
    g.set_next_id("zai".into());
    g.set_next_name("Z.ai".into());
    g.set_expanded(true);

    // Every press is recorded with what it carried.
    let pressed: Rc<RefCell<Vec<(String, String)>>> = Rc::default();
    macro_rules! record {
        ($($on:ident => $name:literal),*) => {$(
            let p = pressed.clone();
            g.$on(move |id| p.borrow_mut().push(($name.to_string(), id.to_string())));
        )*};
    }
    record!(on_start => "start", on_have_key => "have-key", on_open_keys => "open-keys", on_paste => "paste", on_skip => "skip",
            on_unskip => "unskip", on_toggle => "toggle", on_replace => "replace", on_remove => "remove",
            on_confirm_remove => "confirm-remove", on_cancel_remove => "cancel-remove");

    ui.set_tall(height as f32);
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let draw = || {
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(p.make_mut_slice(), width as usize); });
        }
        p
    };
    let shot = draw();
    let f = BufWriter::new(File::create(output)?);
    let mut e = png::Encoder::new(f, width, height);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(shot.as_bytes())?;

    let card_w = ui.get_card_width();
    let card_h = ui.get_card_height();
    assert!(card_w <= 600.0, "the card is {card_w}px wide; it must fit beside a window");
    assert!(card_h > 400.0 && card_h < height as f32, "the card is {card_h}px tall: rows missing, or clipped");
    println!("free AI card: {card_w}x{card_h}");

    // Press everywhere on the card; every press reports a known id and nothing else.
    for y in (0..card_h as i32).step_by(7) {
        for x in (20..card_w as i32).step_by(23) {
            click(w, x as f32, y as f32);
            slint::platform::update_timers_and_animations();
        }
    }
    let pressed = pressed.borrow();
    for (what, id) in pressed.iter() {
        assert!(ids.contains(id), "{what} carried {id:?}, which is no row's or value's id");
    }
    let kinds: std::collections::BTreeSet<&str> = pressed.iter().map(|(w, _)| w.as_str()).collect();
    for expected in ["start", "have-key", "open-keys", "paste", "skip", "unskip", "toggle", "replace", "remove"] {
        assert!(kinds.contains(expected), "no button answered with {expected}; pressed: {kinds:?}");
    }
    assert!(pressed.iter().any(|(w, id)| w == "paste" && id == "cloudflare"), "Cloudflare's token has its own Paste");
    assert!(!pressed.iter().any(|(w, id)| w == "paste" && id == "cloudflare_account"), "a value already in is not pasted again");
    println!("free AI card: {} presses, kinds {:?}", pressed.len(), kinds);
    Ok(())
}
