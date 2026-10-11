//! The largest approval card a request may make, in both places it is drawn, at any window size
//! (third and fourth security reviews of #639). `approval_bounds` in the shell refuses a request
//! whose rows would not fit and cuts what is drawn to the same limits; the card itself never grows
//! past the room its host gives it, keeps Decline and Allow inside that room, and — when even its
//! pinned lines do not fit — scrolls everything above the buttons and holds Allow until the end
//! has been in view.
//!
//! This draws a card at the request limits — eight forty-character names, every value carrying
//! control characters drawn as escapes, the rows at the whole budget, three discrepancies, the
//! open-ended warning, the app's word about the call and the session row — in the top-right corner
//! of the whole shell and in the Lens panel, at the window size it is given (validate.sh runs
//! 800×600, 1280×720 and 1280×800). Decline answers inside the window, above the dock, and inside
//! the panel. Where the card had to scroll, Allow is disabled — a click on it grants nothing —
//! until the card has been scrolled to its end, and then it answers. At 800×600 the corner card has
//! to scroll. Then the vault prompt beside it, which must not lie over the card. Then the same card
//! in an agent's pane (approval_pane_tests).
use super::approval_tests::{button_top, card, diff_box, lines, message, save, scan, settle, RUN_RECIPE_SUMMARY};
use super::*;
use slint::platform::WindowEvent;
use slint::{Model, ModelRc, VecModel};
use std::cell::Cell;

/// `approval_bounds::TOTAL_CHARS` and `KEY_CHARS` in the shell.
const TOTAL_CHARS: usize = 640;
const KEY_CHARS: usize = 40;

/// `approvals::OPEN_ENDED_WARNING`, the longest warning a card carries.
const OPEN_ENDED_WARNING: &str = "What it runs can do anything you can. Allowing it for the session lets any mind or \
    caller on this desktop run any command through it, until the shell restarts or the mode is lowered.";

/// The card `row_for` makes of the largest request the shell asks about: eight escaped rows that,
/// with the seven "; " between them, come to exactly the budget — so the line is drawn whole, as
/// the refusal guarantees (it measures the same join).
pub(crate) fn largest() -> ApprovalRequest {
    let width_of = |i: usize| if i == 7 { 80 } else { 78 };
    let rows: Vec<String> = (0..8)
        .map(|i| {
            let key = format!("{i}{}", "k".repeat(KEY_CHARS - 1));
            let value = "<U+0001>".repeat(3) + &"v".repeat(width_of(i) - KEY_CHARS - 2 - 3 * 8);
            format!("{key}: {value}")
        })
        .collect();
    let exactly = rows.join("; ");
    assert_eq!(exactly.chars().count(), TOTAL_CHARS, "the fixture is at the budget");
    ApprovalRequest {
        id: "appr-largest".into(),
        args: lines(&rows.iter().map(String::as_str).collect::<Vec<_>>()),
        what: format!("Runs: {exactly}").into(),
        exactly: exactly.into(),
        discrepancies: lines(&[
            "\u{201c}pi\u{201d} is attached here \u{2014} this is not it.",
            "The caller called this `standard`; the app publishes `sensitive`.",
            "Its agent token was not issued to it; no agent's pane shows this.",
        ]),
        explained: "After this, every command this terminal is given runs as you, with your files and your keys.".into(),
        warning: OPEN_ENDED_WARNING.into(),
        can_session: true,
        ..card(RUN_RECIPE_SUMMARY)
    }
}

/// Scroll the section above a card's buttons to its end with the wheel, the way a person reads
/// on: a few turns over a point inside it.
pub(crate) fn read_to_end(w: &MinimalSoftwareWindow, x: f32, y: f32, width: u32, height: u32) {
    for _ in 0..12 {
        w.dispatch_event(WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(x, y),
            delta_x: 0.0,
            delta_y: -240.0,
        });
        settle(w, width, height);
    }
}

/// Where a card's buttons are, and how its Allow behaved: (Decline's y, Allow's y, whether Allow
/// had to wait for the end).
pub(crate) struct Answered {
    pub(crate) deny_y: f32,
    pub(crate) allow_y: f32,
    pub(crate) waited: bool,
}

/// Find Decline by scanning its column from `bottom` up; press Allow on the same row. If that
/// grants nothing, the card must have had to scroll: read it to its end and press again, which
/// must grant. Panics with `place` in the message if Decline is not found inside `top..bottom`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn answer(
    w: &MinimalSoftwareWindow,
    place: &str,
    (deny_x, allow_x, read_x, read_y): (f32, f32, f32, f32),
    (top, bottom): (f32, f32),
    (width, height): (u32, u32),
    denied: &dyn Fn() -> i32,
    allowed: &dyn Fn() -> i32,
    sessioned: &dyn Fn() -> i32,
) -> Answered {
    let before = denied();
    let deny_y = scan(w, deny_x, top, bottom, || denied() > before)
        .unwrap_or_else(|| panic!("{place}: Decline answers inside {top}..{bottom}"));
    let btn_top = button_top(w, deny_x, deny_y, top, denied);
    let allow_y = btn_top + 14.0;
    let before = allowed();
    click(w, allow_x, allow_y);
    let waited = allowed() == before;
    if waited {
        // The session row is a standing yes: it waits with Allow. Every point of the band under
        // the buttons where it is drawn is pressed, and none of them grants anything.
        let before = sessioned();
        let mut y = btn_top + 34.0;
        while y < (btn_top + 80.0).min(bottom) {
            click(w, read_x, y);
            y += 3.0;
        }
        assert_eq!(sessioned(), before, "{place}: the session row grants nothing before the card is read");
        // Disabled until the end has been in view: the click granted nothing. Read on, and it does.
        read_to_end(w, read_x, read_y, width, height);
        let before = allowed();
        click(w, allow_x, allow_y);
        assert!(allowed() > before, "{place}: Allow answers once the card has been read to its end");
    }
    println!("{place}: Decline at {deny_y} (button from {btn_top}), Allow at {allow_y}{}", if waited { ", after reading to the end" } else { "" });
    Answered { deny_y, allow_y, waited }
}

pub fn run(w: &MinimalSoftwareWindow, output: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let (fw, fh) = (width as f32, height as f32);
    let big = largest();

    // ── The top-right corner of the whole shell ──
    let shell = App::new()?;
    shell.set_current_screen(1);
    let (allowed, denied, sessioned) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    {
        let (a, d, s) = (allowed.clone(), denied.clone(), sessioned.clone());
        shell.on_approval_allow(move |_| a.set(a.get() + 1));
        shell.on_approval_deny(move |_| d.set(d.get() + 1));
        shell.on_approval_allow_session(move |_| s.set(s.get() + 1));
    }
    shell.set_pending_approvals(ModelRc::new(VecModel::from(vec![big.clone()])));
    shell.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    std::thread::sleep(std::time::Duration::from_millis(300));
    let alone = settle(w, width, height);
    save(&alone, output, width, height)?;
    // The card's left edge, and clear of the mind panel drawn at the right edge (sign-off, 5
    // October: the card sat over the panel's avatar). The card is 404px wide, with 16px between
    // it and the panel.
    let cx = shell.get_approval_corner_x();
    let panel = shell.get_mind_panel_width();
    assert!(cx >= 0.0, "the card starts on screen, at {cx}");
    assert!(cx + 404.0 + 16.0 <= fw - panel + 0.5, "the card ({cx}..{}) keeps 16px clear of the {panel}px mind panel", cx + 404.0);
    // The dock starts 48px above the window's foot; a button under it, or under the foot, is a
    // button nobody can press. The scan starts above the dock, so it never presses the dock.
    let dock_top = fh - 48.0;
    let corner = answer(
        w,
        &format!("corner at {width}×{height}"),
        (cx + 100.0, cx + 300.0, cx + 200.0, 140.0),
        (40.0, dock_top - 2.0),
        (width, height),
        &|| denied.get(),
        &|| allowed.get(),
        &|| sessioned.get(),
    );
    assert!(corner.deny_y < dock_top && corner.allow_y < dock_top, "the buttons are above the dock");
    if (width, height) == (800, 600) {
        assert!(corner.waited, "at 800×600 the largest card has to scroll, and Allow waits for its end");
    }
    // The shell republishes a waiting card every second, written into the model on screen
    // (`models::update`): what was read stays read, and Allow stays live through it.
    shell.get_pending_approvals().set_row_data(0, big.clone());
    settle(w, width, height);
    let before = allowed.get();
    click(w, cx + 300.0, corner.allow_y);
    assert!(allowed.get() > before, "a republish in place keeps the card read and Allow live");
    // The session row is a standing yes: it answers only once the card has been read (it has now).
    let before = sessioned.get();
    let session_y = scan(w, cx + 200.0, corner.deny_y + 8.0, dock_top - 2.0, || sessioned.get() > before)
        .expect("the session row answers under the buttons, above the dock");
    save(&settle(w, width, height), &output.replace(".png", "-read.png"), width, height)?;
    // A different request arriving in the same card — written into the same row, as the shell's
    // republish would — is not read: the card is back at its top and Allow waits again.
    if corner.waited {
        shell.get_pending_approvals().set_row_data(0, ApprovalRequest { id: "appr-largest-next".into(), ..big.clone() });
        settle(w, width, height);
        let (before, before_session) = (allowed.get(), sessioned.get());
        click(w, cx + 300.0, corner.allow_y);
        click(w, cx + 200.0, session_y);
        assert_eq!(allowed.get(), before, "a new request in a card that was read leaves Allow disabled until it is read too");
        assert_eq!(sessioned.get(), before_session, "and the session row with it");
        read_to_end(w, cx + 200.0, 140.0, width, height);
        let before = allowed.get();
        click(w, cx + 300.0, corner.allow_y);
        assert!(allowed.get() > before, "read to its end, the new request can be allowed");
    }

    // ── The vault prompt with it: never over the card ──
    let without = settle(w, width, height);
    shell.set_vault_unlock(VaultUnlockRequest {
        reason: "A mind asked to read a password saved in the vault.".into(),
        error: "".into(),
        first_time: false,
    });
    let both = settle(w, width, height);
    save(&both, &output.replace(".png", "-vault.png"), width, height)?;
    diff_box(without.as_slice(), both.as_slice(), width, (0, width), (40, height)).expect("the vault prompt is drawn");
    // Over the card's columns, nothing changes above its last row: the prompt is under the card,
    // or — when under it would be under the dock and there is room — beside it.
    let over_card = diff_box(without.as_slice(), both.as_slice(), width, (cx as u32, cx as u32 + 404), (40, height));
    if let Some((top_over_card, _, _)) = over_card {
        assert!(top_over_card as f32 > session_y + 8.0, "the vault prompt is under the card's last row ({session_y}), at {top_over_card}");
    }
    let before = denied.get();
    scan(w, cx + 100.0, corner.deny_y - 12.0, corner.deny_y + 4.0, || denied.get() > before).expect("Decline still answers with the vault prompt up");

    // ── The Lens panel, right-docked 440px wide between the bars ──
    let (panel_top, panel_bottom) = (48.0f32, fh - 60.0);
    let reply_top = panel_bottom - 48.0;
    let lens = ApprovalLensProbe::new()?;
    lens.set_messages(ModelRc::new(VecModel::from(vec![message("user", "Open a terminal for me.")])));
    lens.set_approvals(ModelRc::new(VecModel::from(vec![big])));
    lens.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    std::thread::sleep(std::time::Duration::from_millis(300));
    save(&settle(w, width, height), &output.replace(".png", "-lens.png"), width, height)?;
    let in_lens = answer(
        w,
        &format!("Lens at {width}×{height}"),
        (fw - 340.0, fw - 130.0, fw - 230.0, 160.0),
        (panel_top, reply_top - 2.0),
        (width, height),
        &|| lens.get_denied(),
        &|| lens.get_allowed(),
        &|| lens.get_sessioned(),
    );
    assert!(in_lens.deny_y < reply_top && in_lens.allow_y < reply_top, "both above the reply box, inside the panel");
    lens.hide()?;

    // ── An agent's own pane, which scrolls itself ──
    super::approval_pane_tests::run(w, output, width, height)?;

    println!(
        "PASS at {width}×{height}: the largest card a request may make keeps Decline and Allow inside the \
         window above the dock (corner, Decline at {}{}), inside the Lens panel (Decline at {}{}) and \
         inside an agent's pane; a click on a waiting Allow granted nothing; the vault prompt lies over none of it",
        corner.deny_y,
        if corner.waited { ", Allow after reading to the end" } else { "" },
        in_lens.deny_y,
        if in_lens.waited { ", Allow after reading to the end" } else { "" },
    );
    Ok(())
}
