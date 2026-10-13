//! A card that expired answers nothing. The shell turns an unanswered request into a record row
//! once its time is up (`publish` in control_approvals.rs sets `decision: "expired"`), and the
//! store refuses a late grant (approval_acceptance_tests.rs). This is the pixel half: in the Lens,
//! at 1280×800, the card is answered at the place its buttons were, a person's late click on
//! that place after expiry presses nothing, and neither does Enter.
use super::approval_fit_tests::read_to_end;
use super::approval_tests::{card, message, save, scan, settle, RUN_RECIPE_SUMMARY};
use super::*;
use slint::platform::Key;
use slint::{ModelRc, VecModel};

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let (panel_left, panel_top, panel_bottom) = (900.0f32, 32.0f32, 760.0f32);
    let ui = ApprovalLensProbe::new()?;
    ui.set_messages(ModelRc::new(VecModel::from(vec![message("user", "Start the Council.")])));
    let waiting = card(RUN_RECIPE_SUMMARY);
    ui.set_approvals(ModelRc::new(VecModel::from(vec![waiting.clone()])));
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    settle(w, width, height);
    std::thread::sleep(std::time::Duration::from_millis(300));
    settle(w, width, height);

    // Where the buttons are while it waits: Decline found by its column, Allow on the same row.
    let (deny_x, allow_x) = (panel_left + 100.0, panel_left + 280.0);
    let deny_y = scan(w, deny_x, panel_top, panel_bottom - 4.0, || ui.get_denied() > 0)
        .expect("Decline answers while the card waits");
    // The Lens holds Allow until a card that scrolls has been read to its end (verify-approval-fit).
    read_to_end(w, panel_left + 190.0, deny_y - 120.0, width, height);
    let allow_y = scan(w, allow_x, panel_top, panel_bottom - 4.0, || ui.get_allowed() > 0)
        .expect("Allow answers while the card waits");

    // Its time runs out: the same request, now a record of an expiry.
    let expired = ApprovalRequest { decision: "expired".into(), age_text: "".into(), ..waiting };
    ui.set_approvals(ModelRc::new(VecModel::from(vec![expired])));
    save(&settle(w, width, height), output, width, height)?;
    let (allowed, sessioned, denied) = (ui.get_allowed(), ui.get_sessioned(), ui.get_denied());
    for (x, y) in [(allow_x, allow_y), (deny_x, deny_y), (allow_x, allow_y + 40.0), (allow_x, deny_y)] {
        click(w, x, y);
        click(w, x, y);
    }
    for k in [slint::SharedString::from("\n"), "\r".into(), " ".into(), Key::Tab.into(), "\n".into()] {
        key(w, k);
    }
    settle(w, width, height);
    assert_eq!(
        (ui.get_allowed(), ui.get_sessioned(), ui.get_denied()),
        (allowed, sessioned, denied),
        "after expiry no click where the buttons were, and no key, answers the card"
    );
    println!("PASS approval expiry: Allow answered at {allow_y} while the card waited; after expiry a double click there, on Decline and on the session row, and Enter/Space/Tab, answered nothing");
    Ok(())
}
