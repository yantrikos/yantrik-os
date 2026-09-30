//! The Minds panel: it draws a person's accounts and their meters inside the screen, and every
//! control reports what it was pressed for.

use super::*;
use slint::{ModelRc, VecModel};

fn meter(name: &str, used: Option<f32>, value: &str) -> AccountMeter {
    AccountMeter { name: name.into(), used: used.unwrap_or(0.0), known: used.is_some(), value: value.into() }
}

fn account(id: &str, label: &str, plan: &str, state: &str, note: &str, meters: Vec<AccountMeter>) -> AccountRow {
    AccountRow {
        id: id.into(),
        label: label.into(),
        plan: plan.into(),
        state: state.into(),
        note: note.into(),
        meters: ModelRc::new(VecModel::from(meters)),
    }
}

fn group(id: &str, name: &str, accounts: Vec<AccountRow>) -> ProviderGroup {
    ProviderGroup { id: id.into(), name: name.into(), accounts: ModelRc::new(VecModel::from(accounts)) }
}

fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let f = BufWriter::new(File::create(path)?);
    let mut e = png::Encoder::new(f, 1280, 800);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(pixels.as_bytes())?;
    Ok(())
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = MindsProbe::new()?;
    ui.set_groups(ModelRc::new(VecModel::from(vec![
        group(
            "claude",
            "Claude",
            vec![
                account("claude:primary", "Main", "Max 20x", "active", "", vec![meter("Today", None, "1.2M tokens")]),
                account("claude:account-2", "Account 2", "Max 20x", "ready", "", vec![meter("Today", None, "84K tokens")]),
            ],
        ),
        group(
            "codex",
            "Codex",
            vec![account(
                "codex:primary",
                "Main",
                "Pro",
                "active",
                "",
                vec![meter("Session", Some(0.37), "4h 42m"), meter("Weekly", Some(0.92), "4d 12h"), meter("Today", None, "610K tokens")],
            )],
        ),
        group("gemini", "Gemini", vec![account("gemini:primary", "Main", "", "signin", "", vec![])]),
        group("xai", "xAI", vec![account("xai:primary", "API key", "the companion's provider", "active", "", vec![])]),
    ])));
    ui.set_choices(ModelRc::new(VecModel::from(vec![
        AddChoice { vendor: "claude".into(), name: "Claude".into(), what: "Add account".into() },
        AddChoice { vendor: "qwen".into(), name: "Qwen".into(), what: "Set up a key".into() },
    ])));
    ui.set_today("1.9M tokens today".into());
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(1280, 800));
    let draw = || {
        slint::platform::update_timers_and_animations();
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(1280, 800);
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(p.make_mut_slice(), 1280); });
        p
    };
    let closed = draw();
    save(&closed, output)?;
    let h = ui.get_panel_height();
    assert!(h > 300.0 && 44.0 + h < 800.0, "the panel fits under the status bar on 1280x800: {h}");

    // The right-hand word of each account does the next thing: scan the right edge of the card.
    let mut seen = Vec::new();
    for y in (44..(44 + h as i32)).step_by(4) {
        for x in [1280.0 - 80.0, 1280.0 - 40.0, 1280.0 - 30.0] {
            ui.set_action("".into());
            click(w, x, y as f32);
            let a = ui.get_action().to_string();
            if !a.is_empty() && !seen.contains(&a) {
                seen.push(a);
            }
        }
        // A press may have opened the "+" list; keep the layout the scan started from.
        ui.set_adding(false);
    }
    for want in ["use:claude:account-2", "signin:gemini:primary", "manage"] {
        assert!(seen.iter().any(|a| a == want), "{want} did not answer a press; saw {seen:?}");
    }
    assert!(!seen.iter().any(|a| a.starts_with("use:claude:primary") || a.starts_with("use:codex")), "ACTIVE is not a button: {seen:?}");

    // The make tiles sit at the bottom of the card.
    let mut made = Vec::new();
    for x in [1280.0 - 300.0, 1280.0 - 190.0, 1280.0 - 80.0] {
        ui.set_action("".into());
        click(w, x, 44.0 + h - 40.0);
        made.push(ui.get_action().to_string());
    }
    assert_eq!(made, ["make:app", "make:theme", "make:recipe"]);

    // "+" opens the list of what can be brought here; a choice reports its vendor and closes it.
    ui.set_adding(true);
    let open = draw();
    save(&open, &output.replace(".png", "-adding.png"))?;
    assert!(ui.get_panel_height() > h + 50.0, "the list takes room");
    ui.set_action("".into());
    for y in (80..200).step_by(4) {
        click(w, 1280.0 - 200.0, y as f32);
        if ui.get_action().starts_with("add:") {
            break;
        }
    }
    assert_eq!(ui.get_action().as_str(), "add:claude");
    assert!(!ui.get_adding(), "choosing puts the list away");
    println!("PASS minds panel: rows, buttons, tiles and the + list");
    Ok(())
}
