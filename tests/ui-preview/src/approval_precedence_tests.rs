//! A waiting approval card stays on top of everything the shell itself draws (the outside design
//! review's precedence check, at the level this runner can reach). The whole shell, 1280×800, the
//! card in its top-right corner, and then each of the shell's own overlays opened over it in turn —
//! Quick Settings, Today, the power menu, the clipboard, the cheat sheet, the battery and network
//! popovers, the window switcher, the Lens, the mode menu and the Minds panel. With each one open
//! the card is drawn pixel for pixel as it was with none, and its Decline answers a click. The
//! window switcher used to be drawn over the card and took that click.
//!
//! The card is not always in the same place: it keeps clear of the open mind panel, and an
//! overlay that covers the panel lets it move out to the right edge (`approval-corner-x`, #648).
//! So it is compared and clicked where the shell says it is in each state, and each line says
//! where that was.
//!
//! What this cannot reach is the compositor: another app's maximised, fullscreen or always-on-top
//! window over the shell's own window. The shell's half of that — it brings itself back in front
//! when a window takes focus over a waiting card, and refuses to move windows while one waits — is
//! card_watch.rs's unit tests; what labwc then draws is the live check in the PR.
use super::approval_tests::{card, save, scan, settle, RUN_RECIPE_SUMMARY};
use super::*;
use slint::{ModelRc, VecModel};
use std::cell::Cell;
use std::rc::Rc;

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let ui = App::new()?;
    ui.set_current_screen(1);
    ui.set_power_profile("balanced".into());
    ui.set_pending_approvals(ModelRc::new(VecModel::from(vec![card(RUN_RECIPE_SUMMARY)])));
    let denied = Rc::new(Cell::new(0));
    let d = denied.clone();
    ui.on_approval_deny(move |_| d.set(d.get() + 1));
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    settle(w, width, height);

    // The card is 404px wide, its left edge where the shell says (`approval-corner-x`), under the
    // status bar. That edge moves: clear of the open mind panel with nothing else up, and out to
    // the right edge when an overlay covers the panel. So the card is found where it is drawn in
    // each state, and Decline, the left half of its button row, clicked there.
    let corner = |ui: &App| ui.get_approval_corner_x().round() as u32;
    let card_top = 48u32;
    let home = corner(&ui);
    // Compared well inside the card's border: its rounded corners and the gap above it show
    // whatever is behind, which an overlay there rightly changes.
    let deny_y = scan(w, home as f32 + 100.0, card_top as f32, height as f32 - 48.0, || denied.get() > 0)
        .expect("Decline answers on the corner card with nothing open");
    let card_bottom = deny_y as u32 + 12;
    let alone = settle(w, width, height);
    save(&alone, output, width, height)?;
    let card_px = |p: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, left: u32| -> Vec<slint::Rgb8Pixel> {
        (card_top + 12..card_bottom)
            .flat_map(|y| (left + 12..left + 404 - 12).map(move |x| (y * width + x) as usize))
            .map(|i| p.as_slice()[i])
            .collect()
    };
    let card_alone = card_px(&alone, home);

    type Open = fn(&App, bool);
    let overlays: [(&str, Open, fn(&App) -> bool); 11] = [
        ("Quick Settings", |u, o| u.set_quick_settings_open(o), |u| u.get_quick_settings_open()),
        ("Today", |u, o| u.set_today_open(o), |u| u.get_today_open()),
        ("the power menu", |u, o| u.set_power_menu_open(o), |u| u.get_power_menu_open()),
        ("the clipboard", |u, o| u.set_clip_panel_open(o), |u| u.get_clip_panel_open()),
        ("the cheat sheet", |u, o| u.set_cheat_sheet_open(o), |u| u.get_cheat_sheet_open()),
        ("the battery popover", |u, o| u.set_battery_popover_open(o), |u| u.get_battery_popover_open()),
        ("the network popover", |u, o| u.set_network_open(o), |u| u.get_network_open()),
        ("the window switcher", |u, o| u.set_alt_tab_open(o), |u| u.get_alt_tab_open()),
        ("the Lens", |u, o| u.set_lens_open(o), |u| u.get_lens_open()),
        ("the mode menu", |u, o| u.set_mind_menu_open(o), |u| u.get_mind_menu_open()),
        ("the Minds panel", |u, o| u.set_minds_panel_open(o), |u| u.get_minds_panel_open()),
    ];
    let mut failures = Vec::new();
    for (name, open, is_open) in overlays {
        open(&ui, true);
        let over = settle(w, width, height);
        if !is_open(&ui) {
            failures.push(format!("{name} did not stay open, so this check proved nothing"));
            continue;
        }
        let left = corner(&ui);
        let differ = card_px(&over, left).iter().zip(&card_alone).filter(|(a, b)| a != b).count();
        if differ > 0 {
            save(&over, &output.replace(".png", &format!("-{}.png", name.replace(' ', "-"))), width, height)?;
            failures.push(format!("with {name} open, {differ} pixels of the card are covered or changed"));
        }
        let before = denied.get();
        click(w, left as f32 + 100.0, deny_y);
        if denied.get() == before {
            failures.push(format!("with {name} open, a click on Decline did not reach the card"));
        }
        open(&ui, false);
        settle(w, width, height);
        println!("{name}: card at x={left} (alone at {home}), {differ} pixels of it differ; Decline answered: {}", denied.get() > before);
    }
    assert!(failures.is_empty(), "the waiting card is not on top of everything the shell draws:\n{}", failures.join("\n"));
    println!("PASS approval precedence: with each of {} shell overlays open, the card is drawn whole on top and Decline answers", overlays.len());
    Ok(())
}
