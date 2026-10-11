//! The Minds panel: it draws a person's accounts and their meters inside the screen, and every
//! control reports what it was pressed for.

use super::*;
use slint::{ModelRc, VecModel};

pub(crate) fn meter(name: &str, used: Option<f32>, value: &str) -> AccountMeter {
    AccountMeter { name: name.into(), used: used.unwrap_or(0.0), known: used.is_some(), value: value.into() }
}

pub(crate) fn account(id: &str, label: &str, plan: &str, state: &str, note: &str, meters: Vec<AccountMeter>) -> AccountRow {
    AccountRow {
        id: id.into(),
        label: label.into(),
        plan: plan.into(),
        state: state.into(),
        note: note.into(),
        meters: ModelRc::new(VecModel::from(meters)),
        program: if id.starts_with("claude") { "Claude Code".into() } else { "".into() },
        used_by: if state == "active" && id.starts_with("claude") { "Used by Claude Code in a terminal. No mind here runs on it.".into() } else { "".into() },
    }
}

pub(crate) fn group(id: &str, name: &str, accounts: Vec<AccountRow>) -> ProviderGroup {
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
        AddChoice { vendor: "gemini".into(), name: "Gemini".into(), what: "Sign in".into() },
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

    // At its fullest: what the shell's panel view (wire/minds_panel.rs) makes of many vendors of
    // two accounts each, three vendors, the answering account with its tightest meter, the
    // other on one line, and "N more". It must stay on a 1280x800 screen with room to spare.
    let two = |v: &str, plan: &str| {
        vec![
            account(&format!("{v}:primary"), "Main", plan, "active", "", vec![meter("Weekly", Some(0.6), "4d 12h")]),
            account(&format!("{v}:account-2"), "Account 2", plan, "ready", "", vec![]),
        ]
    };
    ui.set_groups(ModelRc::new(VecModel::from(vec![
        group("claude", "Claude", two("claude", "Max 20x")),
        group("codex", "Codex", two("codex", "Pro")),
        group("gemini", "Gemini", two("gemini", "Pro")),
    ])));
    ui.set_answering("Yantrik Mind".into());
    ui.set_more(2);
    let full = draw();
    save(&full, &output.replace(".png", "-full.png"))?;
    let fh = ui.get_panel_height();
    assert!(fh <= 600.0, "three vendors of two accounts, answering and more: {fh}px, over 600");
    // "2 more on the Accounts page" takes the person there, as the gear does.
    let mut went = false;
    for y in ((44 + fh as i32 - 160)..(44 + fh as i32 - 90)).step_by(3) {
        ui.set_action("".into());
        click(w, 1280.0 - 300.0, y as f32);
        if ui.get_action() == "manage" {
            went = true;
            break;
        }
    }
    assert!(went, "\"more\" does not open the Accounts page");
    println!("PASS minds panel: rows, buttons, tiles, the + list, and at its fullest {fh}px with more →");
    Ok(())
}
